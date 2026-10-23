//! Digit layouts: decimal, scientific, and fraction.
//!
//! Every layout rounds half away from zero on the scaled double and pads with zeros counted by
//! `0` placeholders. A code the 0.1.26 subset formatted also keeps that subset's rules: `#NUM!`
//! once the rounded digits reach 2^53, a minus sign only when a digit or a `0` placeholder is
//! shown, an integer `0` kept unless decimals follow, and the decimal point omitted when no
//! decimal digit is shown. Other codes follow Excel, which shows nothing for a zero integer part
//! without a `0` placeholder (`TEXT(0,"#")` is empty) and always shows the decimal point
//! (`TEXT(1,"0.#")` is `1.`).

use super::super::value::ErrorKind;
use super::lexer::{Meridiem, Placeholder, Token};

/// The first count of rounded units a double cannot hold exactly.
const MAX_UNITS: f64 = 9.007_199_254_740_992e15;

/// Fraction denominators are searched up to this many digits.
const MAX_DENOMINATOR_DIGITS: usize = 4;

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Literal(String),
    Placeholder(Placeholder),
    DecimalPoint,
    Comma,
    Percent,
    Exponent {
        uppercase: bool,
        always_signed: bool,
    },
    Slash,
}

/// Renders a section whose tokens contain digit placeholders; `earlier_subset` keeps the rules of
/// the 0.1.26 subset for a code it formatted.
pub(super) fn render(
    tokens: &[Token],
    value: f64,
    signed: bool,
    earlier_subset: bool,
) -> Result<String, ErrorKind> {
    let items = tokens
        .iter()
        .map(|token| item(token, earlier_subset))
        .collect::<Result<Vec<_>, _>>()?;
    if items
        .iter()
        .any(|item| matches!(item, Item::Exponent { .. }))
    {
        return render_scientific(&items, value, signed);
    }
    let slash = fraction_slash(&items)?;
    // The 0.1.26 subset showed any other `/` as written, as in `0/2`. To Excel a slash before a
    // digit is a fixed denominator, and a slash elsewhere beside digits, as in `#%/#`, is
    // unmeasured.
    if !earlier_subset
        && items
            .iter()
            .enumerate()
            .any(|(index, item)| *item == Item::Slash && Some(index) != slash)
    {
        return Err(ErrorKind::Unsupported);
    }
    match slash {
        Some(slash) => render_fraction(&items, slash, value, signed),
        None => render_decimal(&items, value, signed, earlier_subset),
    }
}

/// Reads a token inside a digit layout. The 0.1.26 subset showed unquoted letters and `A/P` as
/// written; in other codes they are unsupported, because some hosts read letters as codes.
fn item(token: &Token, earlier_subset: bool) -> Result<Item, ErrorKind> {
    Ok(match token {
        Token::Literal(text) => Item::Literal(text.clone()),
        Token::Letters { text, .. } if earlier_subset => Item::Literal(text.clone()),
        Token::Meridiem(Meridiem::Letter { morning, evening }) if earlier_subset => {
            Item::Literal(format!("{morning}/{evening}"))
        }
        Token::Skip => Item::Literal(" ".to_owned()),
        Token::Placeholder(placeholder) => Item::Placeholder(*placeholder),
        Token::DecimalPoint => Item::DecimalPoint,
        Token::Comma => Item::Comma,
        Token::Percent => Item::Percent,
        Token::Exponent {
            uppercase,
            always_signed,
        } => Item::Exponent {
            uppercase: *uppercase,
            always_signed: *always_signed,
        },
        Token::Slash => Item::Slash,
        _ => return Err(ErrorKind::Unsupported),
    })
}

/// Finds the slash of a fraction layout such as `# ?/?` or `??/??`: a `/` between two digit
/// placeholders.
fn fraction_slash(items: &[Item]) -> Result<Option<usize>, ErrorKind> {
    let is_placeholder = |index: Option<usize>| {
        index
            .and_then(|index| items.get(index))
            .is_some_and(|item| matches!(item, Item::Placeholder(_)))
    };
    let mut found = None;
    for (index, item) in items.iter().enumerate() {
        if *item == Item::Slash
            && is_placeholder(index.checked_sub(1))
            && is_placeholder(Some(index + 1))
            && found.replace(index).is_some()
        {
            return Err(ErrorKind::Unsupported);
        }
    }
    Ok(found)
}

/// The digits of a rounded magnitude, split at the decimal point as 0.1.26 computed them.
struct Digits {
    integer: String,
    fraction: String,
}

/// Rounds `magnitude` to `decimals` places and returns the digits and the rounded unit count.
fn rounded_digits(magnitude: f64, decimals: usize) -> Result<(Digits, u64), ErrorKind> {
    let factor = 10_f64.powi(decimals as i32);
    let scaled_units = (magnitude * factor).round();
    if scaled_units >= MAX_UNITS {
        return Err(ErrorKind::Num);
    }
    let units = scaled_units as u64;
    let unit_text = units.to_string();
    let padded = if unit_text.len() <= decimals {
        format!(
            "{}{}",
            "0".repeat(decimals + 1 - unit_text.len()),
            unit_text
        )
    } else {
        unit_text
    };
    let split = padded.len() - decimals;
    Ok((
        Digits {
            integer: padded[..split].to_owned(),
            fraction: padded[split..].to_owned(),
        },
        units,
    ))
}

/// Applies the integer rules: zero padding to the count of `0` placeholders and no lone `0` when
/// every integer placeholder is optional. The 0.1.26 subset kept that `0` unless decimals
/// followed, which `keep_lone_zero` asks for.
fn integer_text(mut integer: String, places: &[Placeholder], keep_lone_zero: bool) -> String {
    let zeros = count(places, Placeholder::Zero);
    while integer.len() < zeros {
        integer.insert(0, '0');
    }
    if integer == "0" && zeros == 0 && !keep_lone_zero {
        integer.clear();
    }
    integer
}

/// Drops trailing zeros an optional decimal placeholder does not show; a dropped `?` becomes a
/// space. Returns the text of each decimal placeholder.
fn fraction_slots(fraction: &str, places: &[Placeholder]) -> Vec<String> {
    let optional = places.len() - count(places, Placeholder::Zero);
    let dropped = fraction
        .chars()
        .rev()
        .take(optional)
        .take_while(|digit| *digit == '0')
        .count();
    let shown = places.len() - dropped;
    fraction
        .chars()
        .zip(places)
        .enumerate()
        .map(|(index, (digit, placeholder))| {
            if index < shown {
                digit.to_string()
            } else if *placeholder == Placeholder::Question {
                " ".to_owned()
            } else {
                String::new()
            }
        })
        .collect()
}

/// Spreads integer digits over placeholders from the right; the first placeholder takes every
/// extra leading digit, and an empty `?` shows a space.
fn integer_slots(digits: &str, places: &[Placeholder]) -> Vec<String> {
    let characters = digits.chars().collect::<Vec<_>>();
    let mut slots = vec![String::new(); places.len()];
    for (offset, slot) in slots.iter_mut().enumerate().rev() {
        let from_right = places.len() - 1 - offset;
        if offset == 0 && characters.len() > from_right {
            *slot = characters[..characters.len() - from_right].iter().collect();
        } else if from_right < characters.len() {
            *slot = characters[characters.len() - 1 - from_right].to_string();
        }
        if slot.is_empty() && places[offset] == Placeholder::Question {
            *slot = " ".to_owned();
        }
    }
    slots
}

fn group_thousands(digits: &str) -> String {
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn count(places: &[Placeholder], wanted: Placeholder) -> usize {
    places.iter().filter(|place| **place == wanted).count()
}

/// Whether anything but grouping commas sits between the first and last placeholder of a run.
fn interleaved(items: &[Item], positions: &[usize]) -> bool {
    let (Some(first), Some(last)) = (positions.first(), positions.last()) else {
        return false;
    };
    items[*first..=*last]
        .iter()
        .any(|item| !matches!(item, Item::Placeholder(_) | Item::Comma))
}

fn render_decimal(
    items: &[Item],
    value: f64,
    signed: bool,
    earlier_subset: bool,
) -> Result<String, ErrorKind> {
    let mut points = items
        .iter()
        .enumerate()
        .filter(|(_, item)| **item == Item::DecimalPoint)
        .map(|(index, _)| index);
    let point = points.next();
    if points.next().is_some() {
        return Err(ErrorKind::Unsupported);
    }
    let boundary = point.unwrap_or(items.len());
    let placeholder_positions = |range: std::ops::Range<usize>| {
        range
            .filter(|index| matches!(items[*index], Item::Placeholder(_)))
            .collect::<Vec<_>>()
    };
    let integer_positions = placeholder_positions(0..boundary);
    let fraction_positions = placeholder_positions(boundary..items.len());
    let places_at = |positions: &[usize]| {
        positions
            .iter()
            .map(|index| match items[*index] {
                Item::Placeholder(placeholder) => placeholder,
                _ => unreachable!("positions index placeholders"),
            })
            .collect::<Vec<_>>()
    };
    let integer_places = places_at(&integer_positions);
    let fraction_places = places_at(&fraction_positions);

    let mut grouping = false;
    let mut thousands_scale = 0;
    let mut scale_open = false;
    for (index, current) in items.iter().enumerate() {
        let after_placeholder = matches!(
            index.checked_sub(1).map(|before| &items[before]),
            Some(Item::Placeholder(_))
        );
        let before_placeholder = matches!(items.get(index + 1), Some(Item::Placeholder(_)));
        if *current != Item::Comma {
            scale_open = matches!(current, Item::Placeholder(_));
            continue;
        }
        if after_placeholder && before_placeholder && index < boundary {
            grouping = true;
            scale_open = false;
        } else if (after_placeholder || scale_open) && !before_placeholder {
            thousands_scale += 1;
            scale_open = true;
        } else {
            return Err(ErrorKind::Unsupported);
        }
    }
    if grouping
        && (interleaved(items, &integer_positions)
            || integer_places.contains(&Placeholder::Question))
    {
        return Err(ErrorKind::Unsupported);
    }

    let mut scaled = value;
    for _ in items.iter().filter(|item| **item == Item::Percent) {
        scaled *= 100.0;
    }
    for _ in 0..thousands_scale {
        scaled /= 1000.0;
    }
    if !scaled.is_finite() {
        return Err(ErrorKind::Num);
    }
    let negative = scaled < 0.0;
    let decimals = fraction_places.len();
    let (digits, units) = rounded_digits(scaled.abs(), decimals)?;
    let integer = integer_text(
        digits.integer,
        &integer_places,
        earlier_subset && decimals == 0,
    );
    let fraction = fraction_slots(&digits.fraction, &fraction_places);
    let point_shown = !earlier_subset || fraction.iter().any(|slot| !slot.is_empty());
    let zero_placeholders = count(&integer_places, Placeholder::Zero);
    // Whether Excel signs a negative number that rounds to zero without a `0` placeholder has not
    // been measured.
    if signed && negative && units == 0 && zero_placeholders == 0 && !earlier_subset {
        return Err(ErrorKind::Unsupported);
    }
    let integer_slots = if grouping {
        let mut slots = vec![String::new(); integer_places.len()];
        slots[0] = group_thousands(&integer);
        slots
    } else {
        integer_slots(&integer, &integer_places)
    };

    let mut output = String::new();
    if signed && negative && (units > 0 || zero_placeholders > 0) {
        output.push('-');
    }
    let (mut integer_slot, mut fraction_slot) = (0, 0);
    for (index, current) in items.iter().enumerate() {
        match current {
            Item::Literal(text) => output.push_str(text),
            Item::Percent => output.push('%'),
            Item::Slash => output.push('/'),
            Item::Comma | Item::Exponent { .. } => {}
            Item::DecimalPoint => {
                if integer_places.is_empty() {
                    output.push_str(&integer);
                }
                if point_shown {
                    output.push('.');
                }
            }
            Item::Placeholder(_) if index < boundary => {
                output.push_str(&integer_slots[integer_slot]);
                integer_slot += 1;
            }
            Item::Placeholder(_) => {
                output.push_str(&fraction[fraction_slot]);
                fraction_slot += 1;
            }
        }
    }
    Ok(output)
}

fn render_scientific(items: &[Item], value: f64, signed: bool) -> Result<String, ErrorKind> {
    let exponent_index = items
        .iter()
        .position(|item| matches!(item, Item::Exponent { .. }))
        .expect("scientific layouts contain an exponent");
    let Item::Exponent {
        uppercase,
        always_signed,
    } = items[exponent_index]
    else {
        unreachable!("the position matched an exponent");
    };
    let (mantissa_items, exponent_items) = (&items[..exponent_index], &items[exponent_index + 1..]);
    let exponent_width = exponent_items
        .iter()
        .take_while(|item| matches!(item, Item::Placeholder(_)))
        .count();
    let exponent_places = exponent_items[..exponent_width]
        .iter()
        .map(|item| match item {
            Item::Placeholder(placeholder) => *placeholder,
            _ => unreachable!("taken while placeholders"),
        })
        .collect::<Vec<_>>();
    let suffix = &exponent_items[exponent_width..];
    if exponent_width == 0
        || exponent_places.contains(&Placeholder::Question)
        || !suffix.iter().all(|item| matches!(item, Item::Literal(_)))
    {
        return Err(ErrorKind::Unsupported);
    }
    let first = mantissa_items
        .iter()
        .position(|item| !matches!(item, Item::Literal(_)))
        .ok_or(ErrorKind::Unsupported)?;
    let (prefix, layout) = mantissa_items.split_at(first);
    let point = layout.iter().position(|item| *item == Item::DecimalPoint);
    let (integer_items, fraction_items) = match point {
        Some(point) => (&layout[..point], &layout[point + 1..]),
        None => (layout, &layout[layout.len()..]),
    };
    let places = |items: &[Item]| {
        items
            .iter()
            .map(|item| match item {
                Item::Placeholder(placeholder) => Ok(*placeholder),
                _ => Err(ErrorKind::Unsupported),
            })
            .collect::<Result<Vec<_>, _>>()
    };
    let integer_places = places(integer_items)?;
    let fraction_places = places(fraction_items)?;
    let step = match integer_places.len() {
        0 => return Err(ErrorKind::Unsupported),
        1 => 1,
        // `##0.0E+0` keeps the exponent a multiple of the integer placeholders.
        width if integer_places.contains(&Placeholder::Hash) => width as i32,
        _ => return Err(ErrorKind::Unsupported),
    };

    let magnitude = value.abs();
    if !magnitude.is_finite() {
        return Err(ErrorKind::Num);
    }
    let decimals = fraction_places.len();
    let (exponent, digits) = if magnitude == 0.0 {
        (0, rounded_digits(0.0, decimals)?.0)
    } else {
        let mut exponent = magnitude.log10().floor() as i32;
        if 10_f64.powi(exponent) > magnitude {
            exponent -= 1;
        } else if 10_f64.powi(exponent + 1) <= magnitude {
            exponent += 1;
        }
        exponent = exponent.div_euclid(step) * step;
        loop {
            let mantissa = magnitude / 10_f64.powi(exponent);
            if !mantissa.is_finite() {
                return Err(ErrorKind::Num);
            }
            let (digits, _) = rounded_digits(mantissa, decimals)?;
            if digits.integer.len() > step as usize {
                exponent += step;
                continue;
            }
            break (exponent, digits);
        }
    };
    // How Excel shows a zero mantissa without a `0` placeholder, or a decimal point without a
    // shown decimal digit, has not been measured in scientific notation.
    if magnitude == 0.0 && !integer_places.contains(&Placeholder::Zero) {
        return Err(ErrorKind::Unsupported);
    }
    let integer = integer_text(digits.integer, &integer_places, false);
    let fraction = fraction_slots(&digits.fraction, &fraction_places).concat();
    if point.is_some() && fraction.is_empty() {
        return Err(ErrorKind::Unsupported);
    }

    let mut output = String::new();
    if signed && value < 0.0 {
        output.push('-');
    }
    for item in prefix {
        if let Item::Literal(text) = item {
            output.push_str(text);
        }
    }
    output.push_str(&integer_slots(&integer, &integer_places).concat());
    if !fraction.is_empty() {
        output.push('.');
        output.push_str(&fraction);
    }
    output.push(if uppercase { 'E' } else { 'e' });
    if exponent < 0 {
        output.push('-');
    } else if always_signed {
        output.push('+');
    }
    let exponent_digits = exponent.unsigned_abs().to_string();
    for _ in exponent_digits.len()..count(&exponent_places, Placeholder::Zero) {
        output.push('0');
    }
    output.push_str(&exponent_digits);
    for item in suffix {
        if let Item::Literal(text) = item {
            output.push_str(text);
        }
    }
    Ok(output)
}

/// Renders `# ?/?`-style layouts: an optional integer part, then the closest fraction whose
/// denominator has at most as many digits as its placeholders.
fn render_fraction(
    items: &[Item],
    slash: usize,
    value: f64,
    signed: bool,
) -> Result<String, ErrorKind> {
    let run_start = |end: usize| {
        items[..end]
            .iter()
            .rposition(|item| !matches!(item, Item::Placeholder(_)))
            .map_or(0, |position| position + 1)
    };
    let numerator_start = run_start(slash);
    let denominator_end = items[slash + 1..]
        .iter()
        .position(|item| !matches!(item, Item::Placeholder(_)))
        .map_or(items.len(), |position| slash + 1 + position);
    let integer_end = items[..numerator_start]
        .iter()
        .rposition(|item| matches!(item, Item::Placeholder(_)))
        .map(|position| position + 1);
    let integer_start = integer_end.map(run_start);
    let literal_only = |range: &[Item]| range.iter().all(|item| matches!(item, Item::Literal(_)));
    let prefix_end = integer_start.unwrap_or(numerator_start);
    let separator = &items[integer_end.unwrap_or(numerator_start)..numerator_start];
    if !literal_only(&items[..prefix_end])
        || !literal_only(separator)
        || !literal_only(&items[denominator_end..])
    {
        return Err(ErrorKind::Unsupported);
    }
    let places = |range: &[Item]| {
        range
            .iter()
            .map(|item| match item {
                Item::Placeholder(placeholder) => *placeholder,
                _ => unreachable!("runs hold placeholders"),
            })
            .collect::<Vec<_>>()
    };
    let integer_places = integer_start
        .zip(integer_end)
        .map(|(start, end)| places(&items[start..end]));
    let numerator_places = places(&items[numerator_start..slash]);
    let denominator_places = places(&items[slash + 1..denominator_end]);
    if denominator_places.len() > MAX_DENOMINATOR_DIGITS
        || denominator_places.contains(&Placeholder::Zero)
    {
        return Err(ErrorKind::Unsupported);
    }

    let magnitude = value.abs();
    if !magnitude.is_finite() || magnitude >= MAX_UNITS {
        return Err(ErrorKind::Num);
    }
    let maximum_denominator = 10_u64.pow(denominator_places.len() as u32) - 1;
    let (mut whole, part) = match integer_places {
        Some(_) => (magnitude.floor(), magnitude - magnitude.floor()),
        None => (0.0, magnitude),
    };
    let (mut numerator, denominator) = closest_fraction(part, maximum_denominator);
    if integer_places.is_some() && numerator == denominator {
        whole += 1.0;
        numerator = 0;
    }
    // Excel 16.114 shows a fraction that rounds to zero as spaces as wide as the fraction after
    // the whole part (`TEXT(1.999,"# ?/?")` is `2    `) and signs a negative number before an
    // empty whole part (`TEXT(-0.25,"# ?/?")` is `- 1/4`; excel-measure RESULTS.md, section B).
    // A zero fraction without a nonzero whole part or beside `#` or `0` placeholders, and a
    // negative number without a whole part, are unmeasured.
    let zero_fraction = numerator == 0;
    let only_question =
        |places: &[Placeholder]| places.iter().all(|place| *place == Placeholder::Question);
    if (zero_fraction
        && (whole == 0.0
            || !only_question(&numerator_places)
            || !only_question(&denominator_places)))
        || (signed && value < 0.0 && integer_places.is_none())
    {
        return Err(ErrorKind::Unsupported);
    }
    let whole_digits = if whole == 0.0 {
        String::new()
    } else {
        (whole as u64).to_string()
    };

    let mut output = String::new();
    if signed && value < 0.0 {
        output.push('-');
    }
    push_literals(&items[..prefix_end], &mut output);
    if let Some(integer_places) = &integer_places {
        output.push_str(&padded_left(&whole_digits, integer_places));
    }
    push_literals(separator, &mut output);
    if zero_fraction {
        output.push_str(&" ".repeat(numerator_places.len() + 1 + denominator_places.len()));
    } else {
        output.push_str(&padded_left(&numerator.to_string(), &numerator_places));
        output.push('/');
        let denominator = denominator.to_string();
        output.push_str(&denominator);
        for place in denominator_places.iter().skip(denominator.len()) {
            if *place == Placeholder::Question {
                output.push(' ');
            }
        }
    }
    push_literals(&items[denominator_end..], &mut output);
    Ok(output)
}

fn push_literals(items: &[Item], output: &mut String) {
    for item in items {
        if let Item::Literal(text) = item {
            output.push_str(text);
        }
    }
}

/// Pads `digits` on the left to the placeholders: `0` adds a zero, `?` a space, `#` nothing.
fn padded_left(digits: &str, places: &[Placeholder]) -> String {
    let missing = places.len().saturating_sub(digits.chars().count());
    let mut output = String::new();
    for place in &places[..missing] {
        match place {
            Placeholder::Zero => output.push('0'),
            Placeholder::Question => output.push(' '),
            Placeholder::Hash => {}
        }
    }
    output.push_str(digits);
    output
}

/// The fraction closest to `value` with a denominator from 1 through `maximum_denominator`; a
/// tie keeps the smaller denominator.
fn closest_fraction(value: f64, maximum_denominator: u64) -> (u64, u64) {
    let mut best = (value.round() as u64, 1);
    let mut best_error = (value - value.round()).abs();
    for denominator in 2..=maximum_denominator {
        let numerator = (value * denominator as f64).round();
        let error = (value - numerator / denominator as f64).abs();
        if error < best_error {
            best = (numerator as u64, denominator);
            best_error = error;
        }
    }
    let divisor = greatest_common_divisor(best.0, best.1);
    (best.0 / divisor, best.1 / divisor)
}

fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left.max(1)
}
