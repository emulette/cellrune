use std::collections::BTreeMap;

use pcre2::bytes::{CaptureLocations, Regex, RegexBuilder};

use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::value::ErrorKind;
use super::array_common::poll_cancellation;
use super::regex_options::start_options;
use super::regex_pattern::{analyze_pattern, is_plain_literal};

const MAX_REGEX_DEPTH: u64 = 65_536;
const MAX_REGEX_HEAP_KIB: u64 = 8 * 1_024;
const INITIAL_MATCH_LIMIT: u64 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RegexCaptureSet {
    spans: Vec<Option<(usize, usize)>>,
}

impl RegexCaptureSet {
    pub(super) fn span(&self, index: usize) -> Option<(usize, usize)> {
        self.spans.get(index).copied().flatten()
    }

    pub(super) fn len(&self) -> usize {
        self.spans.len()
    }
}

struct RegexBuildSpec {
    preserved: String,
    remainder: String,
    case_insensitive: bool,
    depth_limit: u64,
    heap_limit: u64,
    source_bytes: u64,
    scan_factor: u64,
    trailing_comment_terminator: &'static str,
}

struct RegexTier {
    match_limit: u64,
    regular: Regex,
    anchored_nonempty: Regex,
    regular_locations: CaptureLocations,
    anchored_locations: CaptureLocations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Regular,
    AnchoredNonempty,
}

pub(super) struct CompiledRegex {
    spec: RegexBuildSpec,
    tiers: Vec<RegexTier>,
    maximum_match_limit: u64,
    capture_name_indexes: BTreeMap<String, Vec<usize>>,
    has_root_recursion: bool,
}

impl CompiledRegex {
    pub(super) fn compile(
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        pattern: &str,
        case_insensitive: bool,
    ) -> Result<Self, ErrorKind> {
        poll_cancellation(context)?;
        engine.ensure_text_bytes(pattern.len())?;

        let options = start_options(pattern);
        let function_limit = engine.max_function_iterations();
        let native_ceiling = native_match_ceiling(function_limit);
        let maximum_match_limit = options
            .match_limit
            .map_or(native_ceiling, |user| user.min(native_ceiling));
        let depth_limit = function_limit.clamp(1, MAX_REGEX_DEPTH);
        let depth_limit = options
            .depth_limit
            .map_or(depth_limit, |user| user.min(depth_limit));
        let heap_limit = options
            .heap_limit
            .map_or(MAX_REGEX_HEAP_KIB, |user| user.min(MAX_REGEX_HEAP_KIB));
        let source_bytes = u64::try_from(pattern.len()).map_err(|_| ErrorKind::Num)?;
        let scan_factor = if options.preserved.is_empty()
            && !case_insensitive
            && is_plain_literal(options.remainder)
        {
            0
        } else {
            source_bytes
        };
        let analysis = analyze_pattern(pattern, options.newline);
        let spec = RegexBuildSpec {
            preserved: options.preserved,
            remainder: options.remainder.to_owned(),
            case_insensitive,
            depth_limit,
            heap_limit,
            source_bytes,
            scan_factor,
            trailing_comment_terminator: if analysis.ends_in_extended_comment {
                options.newline.comment_terminator()
            } else {
                ""
            },
        };
        let initial_match_limit = maximum_match_limit.min(INITIAL_MATCH_LIMIT);
        let tier = build_tier(engine, context, &spec, initial_match_limit)?;
        let capture_names = tier.regular.capture_names().to_vec();
        let capture_name_work = capture_names.iter().try_fold(
            u64::try_from(capture_names.len()).map_err(|_| ErrorKind::Num)?,
            |work, name| {
                let bytes = name.as_ref().map_or(Ok(0), |name| {
                    u64::try_from(name.len()).map_err(|_| ErrorKind::Num)
                })?;
                work.checked_add(bytes).ok_or(ErrorKind::Num)
            },
        )?;
        engine.charge_function_iterations(context, capture_name_work)?;
        let mut capture_name_indexes: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, name) in capture_names.iter().enumerate() {
            if let Some(name) = name {
                capture_name_indexes
                    .entry(name.clone())
                    .or_default()
                    .push(index);
            }
        }
        poll_cancellation(context)?;
        Ok(Self {
            spec,
            tiers: vec![tier],
            maximum_match_limit,
            capture_name_indexes,
            has_root_recursion: analysis.has_root_recursion,
        })
    }

    pub(super) fn capture_name_indexes(&self) -> &BTreeMap<String, Vec<usize>> {
        &self.capture_name_indexes
    }

    pub(super) fn captures(
        &mut self,
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        subject: &str,
        maximum_matches: Option<usize>,
    ) -> Result<Vec<RegexCaptureSet>, ErrorKind> {
        poll_cancellation(context)?;
        engine.ensure_text_bytes(subject.len())?;
        let subject_bytes = u64::try_from(subject.len()).map_err(|_| ErrorKind::Num)?;
        engine.charge_function_iterations(
            context,
            self.spec
                .scan_factor
                .checked_mul(subject_bytes)
                .ok_or(ErrorKind::Num)?,
        )?;
        engine.charge_function_iterations(context, subject.len() as u64)?;

        let mut output = Vec::new();
        let mut cursor = 0;
        while cursor <= subject.len()
            && maximum_matches.is_none_or(|maximum| output.len() < maximum)
        {
            poll_cancellation(context)?;
            let Some((start, end)) = self.read_capture(
                engine,
                context,
                MatchKind::Regular,
                subject,
                cursor,
                &mut output,
            )?
            else {
                break;
            };
            if start != end {
                cursor = end;
                continue;
            }
            if maximum_matches.is_some_and(|maximum| output.len() >= maximum) {
                break;
            }
            if self.has_root_recursion {
                return Err(ErrorKind::Value);
            }

            if let Some((_, nonempty_end)) = self.read_capture(
                engine,
                context,
                MatchKind::AnchoredNonempty,
                subject,
                start,
                &mut output,
            )? {
                cursor = nonempty_end;
                continue;
            }
            if start == subject.len() {
                break;
            }
            cursor = next_utf8_boundary(subject, start);
        }
        poll_cancellation(context)?;
        Ok(output)
    }

    fn read_capture(
        &mut self,
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        kind: MatchKind,
        subject: &str,
        start: usize,
        output: &mut Vec<RegexCaptureSet>,
    ) -> Result<Option<(usize, usize)>, ErrorKind> {
        let mut tier_index = 0;
        loop {
            let charge = self.tiers[tier_index].match_limit.max(1);
            engine.charge_function_iterations(context, charge)?;
            poll_cancellation(context)?;
            let result = {
                let tier = &mut self.tiers[tier_index];
                match kind {
                    MatchKind::Regular => tier.regular.captures_read_at(
                        &mut tier.regular_locations,
                        subject.as_bytes(),
                        start,
                    ),
                    MatchKind::AnchoredNonempty => tier.anchored_nonempty.captures_read_at(
                        &mut tier.anchored_locations,
                        subject.as_bytes(),
                        start,
                    ),
                }
            };
            poll_cancellation(context)?;
            match result {
                Ok(None) => return Ok(None),
                Ok(Some(matched)) => {
                    let captures_len = self.tiers[tier_index].regular.captures_len();
                    engine.charge_function_iterations(
                        context,
                        u64::try_from(captures_len).map_err(|_| ErrorKind::Num)?,
                    )?;
                    let locations = match kind {
                        MatchKind::Regular => &self.tiers[tier_index].regular_locations,
                        MatchKind::AnchoredNonempty => &self.tiers[tier_index].anchored_locations,
                    };
                    let spans = (0..captures_len)
                        .map(|index| locations.get(index))
                        .collect();
                    output.push(RegexCaptureSet { spans });
                    return Ok(Some((matched.start(), matched.end())));
                }
                Err(error)
                    if error.code() == pcre2_sys::PCRE2_ERROR_MATCHLIMIT
                        && self.tiers[tier_index].match_limit < self.maximum_match_limit =>
                {
                    if tier_index + 1 == self.tiers.len() {
                        let current = self.tiers[tier_index].match_limit;
                        let next = current
                            .saturating_mul(4)
                            .max(current.saturating_add(1))
                            .min(self.maximum_match_limit);
                        self.tiers
                            .push(build_tier(engine, context, &self.spec, next)?);
                    }
                    tier_index += 1;
                }
                Err(_) => {
                    return Err(ErrorKind::ResourceLimit(
                        CalculationLimitKind::FunctionIterations,
                    ));
                }
            }
        }
    }
}

fn native_match_ceiling(function_limit: u64) -> u64 {
    (function_limit / 4).max(1).min(u64::from(u32::MAX))
}

fn build_tier(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    spec: &RegexBuildSpec,
    match_limit: u64,
) -> Result<RegexTier, ErrorKind> {
    poll_cancellation(context)?;
    engine.charge_function_iterations(
        context,
        spec.source_bytes.checked_mul(2).ok_or(ErrorKind::Num)?,
    )?;
    let prefix = format!(
        "{}(*LIMIT_MATCH={match_limit})(*LIMIT_DEPTH={})(*LIMIT_HEAP={})",
        spec.preserved, spec.depth_limit, spec.heap_limit
    );
    let regular_pattern = format!("{prefix}{}", spec.remainder);
    let anchored_pattern = format!(
        "{prefix}(*NOTEMPTY_ATSTART)\\G(?:{}{}\\E)",
        spec.remainder, spec.trailing_comment_terminator
    );
    let regular = build(&regular_pattern, spec.case_insensitive)?;
    let anchored_nonempty = build(&anchored_pattern, spec.case_insensitive)?;
    poll_cancellation(context)?;
    let capture_slots = u64::try_from(regular.captures_len()).map_err(|_| ErrorKind::Num)?;
    engine
        .charge_function_iterations(context, capture_slots.checked_mul(2).ok_or(ErrorKind::Num)?)?;
    let regular_locations = regular.capture_locations();
    let anchored_locations = anchored_nonempty.capture_locations();
    Ok(RegexTier {
        match_limit,
        regular,
        anchored_nonempty,
        regular_locations,
        anchored_locations,
    })
}

fn build(pattern: &str, case_insensitive: bool) -> Result<Regex, ErrorKind> {
    let mut builder = RegexBuilder::new();
    builder.utf(true).ucp(true).caseless(case_insensitive);
    builder.build(pattern).map_err(|_| ErrorKind::Value)
}

fn next_utf8_boundary(text: &str, start: usize) -> usize {
    text[start..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| start + offset)
}

#[cfg(test)]
mod tests {
    use pcre2::bytes::RegexBuilder;

    #[test]
    fn bundled_engine_supports_pcre2_lookbehind_and_backreferences() {
        let mut builder = RegexBuilder::new();
        builder.utf(true).ucp(true);
        let regex = builder
            .build(r"(?<=prefix-)([a-z]+)-\1")
            .expect("valid PCRE2 expression");
        assert!(regex.is_match(b"prefix-cell-cell").expect("match succeeds"));
    }

    #[test]
    fn anchored_nonempty_verb_retries_an_empty_alternative() {
        let mut builder = RegexBuilder::new();
        builder.utf(true).ucp(true);
        let regex = builder
            .build(r"(*NOTEMPTY_ATSTART)\G(?:(?:|a))")
            .expect("valid PCRE2 control verbs");
        let matched = regex.find_at(b"a", 0).expect("match succeeds");
        assert_eq!(matched.map(|value| value.as_bytes()), Some(&b"a"[..]));
    }

    #[test]
    fn pcre2_match_limit_interrupts_pathological_backtracking() {
        let mut builder = RegexBuilder::new();
        builder.utf(true).ucp(true);
        let regex = builder
            .build(r"(*LIMIT_MATCH=10)(*NO_START_OPT)(*NO_AUTO_POSSESS)^(a|aa)+$")
            .expect("valid bounded PCRE2 expression");
        let mut subject = vec![b'a'; 128];
        subject.push(b'b');
        assert!(regex.is_match(&subject).is_err());
    }

    #[test]
    fn adaptive_match_ceiling_stays_inside_pcre2_numeric_limits() {
        assert_eq!(super::native_match_ceiling(1), 1);
        assert_eq!(super::native_match_ceiling(1_000_000), 250_000);
        assert_eq!(super::native_match_ceiling(u64::MAX), u64::from(u32::MAX));
    }
}
