use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::XmlVersion;
use quick_xml::escape::unescape;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesRef, BytesStart, BytesText, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::package::PackageReader;
use super::serialization::{escape_attribute, escape_text};
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteLimits, FormulaRewriteRequest,
    SheetChanges, rewrite_formula,
};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, SPREADSHEETML_STRICT,
    SPREADSHEETML_TRANSITIONAL, namespace_reader, read_event,
};
use crate::{WorkbookSnapshot, XlsxDocument, case_insensitive_eq};

const SPREADSHEETML: &[&str] = &[SPREADSHEETML_TRANSITIONAL, SPREADSHEETML_STRICT];
const RELATIONSHIPS: &[&str] = &[
    DOCUMENT_RELATIONSHIPS_TRANSITIONAL,
    DOCUMENT_RELATIONSHIPS_STRICT,
];
/// Holds the `xm:f` formulas of x14 conditional formats, data validations, and sparklines.
const EXCEL_MAIN: &[&str] = &["http://schemas.microsoft.com/office/excel/2006/main"];
/// Holds `x14:sparklineGroups`.
const SPREADSHEETML_2009: &[&str] =
    &["http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"];
const CHART: &[&str] = &[
    "http://schemas.openxmlformats.org/drawingml/2006/chart",
    "http://purl.oclc.org/ooxml/drawingml/chart",
];
const CHART_2012: &[&str] = &["http://schemas.microsoft.com/office/drawing/2012/chart"];
const CHART_EX: &[&str] = &["http://schemas.microsoft.com/office/drawing/2014/chartex"];
const DETAIL_MISSING_SHEET_PART: &str = "source worksheet part mapping was not found";
const DETAIL_UNREWRITABLE_REFERENCE: &str =
    "a preserved reference to a renamed or deleted sheet cannot be rewritten";
const DETAIL_DELETED_PIVOT_SOURCE: &str =
    "a preserved pivot cache reads a deleted sheet or a table on one";
const DETAIL_DELETED_PIVOT_CHART: &str =
    "a preserved pivot chart reads a pivot table on a deleted sheet";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReferencePart {
    Worksheet,
    Chart,
    PivotCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeldTextKind {
    Formula,
    /// A sparkline's data source, which Excel removes with its element when it names a deleted
    /// sheet, keeping the sparkline's location.
    SparklineFormula,
    PivotSourceName,
}

/// The content of one formula-bearing element, held back until its end tag decides whether the
/// original events or a rewritten formula are written. A sparkline formula also holds its start
/// tag, since a deletion removes the whole element.
struct HeldText {
    depth: u64,
    kind: HeldTextKind,
    start: Option<BytesStart<'static>>,
    events: Vec<Event<'static>>,
    text: String,
    markup: bool,
}

/// How a held element is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeldOutcome {
    Unchanged,
    Rewritten,
    Removed,
}

/// The sheet changes a preserved part is rewritten for, as one request and as its rename-only and
/// deletion-only halves.
struct SheetReferenceRequests<'a> {
    changes: SheetChanges<'a>,
    combined: FormulaRewriteRequest<'a>,
    renames_only: FormulaRewriteRequest<'a>,
    deletions_only: FormulaRewriteRequest<'a>,
}

impl<'a> SheetReferenceRequests<'a> {
    fn new(changes: SheetChanges<'a>) -> Self {
        Self {
            changes,
            combined: FormulaRewriteRequest::Sheets(changes),
            renames_only: FormulaRewriteRequest::Sheets(SheetChanges {
                deleted: &[],
                deleted_tables: &[],
                ..changes
            }),
            deletions_only: FormulaRewriteRequest::Sheets(SheetChanges {
                renames: &[],
                ..changes
            }),
        }
    }

    fn renamed(&self, name: &str) -> Option<&'a str> {
        self.changes
            .renames
            .iter()
            .find(|(old_name, _)| case_insensitive_eq(name, old_name))
            .map(|(_, new_name)| *new_name)
    }

    fn deleted(&self, name: &str) -> bool {
        self.changes
            .deleted
            .iter()
            .any(|deleted| case_insensitive_eq(name, deleted))
    }
}

/// Rewrites references to renamed and deleted sheets in the worksheet, chart, and pivot cache
/// parts the workbook model does not carry: conditional formats, data validations, sparklines,
/// internal hyperlinks, chart series, and pivot cache sources. Only parts that change are
/// replaced, and the parts a deletion removes are skipped.
///
/// A reference to a deleted sheet becomes `#REF!`, except that a sparkline loses its data source
/// and an internal hyperlink keeps its location, as Excel saves them. A pivot cache or pivot chart
/// that reads a deleted sheet is refused, since its source cannot be rewritten.
pub(crate) fn rewrite_sheet_references(
    document: &XlsxDocument,
    draft: &WorkbookSnapshot,
    source_reader: &mut PackageReader<'_>,
    replacements: &mut BTreeMap<PartPath, Vec<u8>>,
    removed_parts: &BTreeSet<PartPath>,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    let original = document.workbook();
    let renames = original
        .sheets()
        .iter()
        .filter_map(|sheet| {
            let name = draft.sheet_by_id(sheet.id())?.name();
            (name != sheet.name()).then(|| (sheet.name().as_str(), name.as_str()))
        })
        .collect::<Vec<_>>();
    let deleted_sheets = original
        .sheets()
        .iter()
        .filter(|sheet| draft.sheet_by_id(sheet.id()).is_none())
        .collect::<Vec<_>>();
    if renames.is_empty() && deleted_sheets.is_empty() {
        return Ok(());
    }
    let deleted = deleted_sheets
        .iter()
        .map(|sheet| sheet.name().as_str())
        .collect::<Vec<_>>();
    let deleted_tables = deleted_sheets
        .iter()
        .flat_map(|sheet| sheet.tables())
        .map(|table| table.display_name().as_str())
        .collect::<Vec<_>>();
    let tabs = original
        .non_worksheet_tabs()
        .tab_names(original.sheets().iter().map(|sheet| sheet.name().as_str()));
    let non_worksheets = original
        .non_worksheet_tabs()
        .iter()
        .map(|tab| tab.name().as_str())
        .collect::<Vec<_>>();
    let requests = SheetReferenceRequests::new(SheetChanges {
        renames: &renames,
        deleted: &deleted,
        tabs: &tabs,
        non_worksheets: &non_worksheets,
        deleted_tables: &deleted_tables,
    });
    let mut parts = Vec::new();
    for sheet in draft
        .sheets()
        .iter()
        .filter(|sheet| original.sheet_by_id(sheet.id()).is_some())
    {
        let part = document.worksheet_part_path(sheet.id()).ok_or_else(|| {
            XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                .with_detail(DETAIL_MISSING_SHEET_PART)
        })?;
        parts.push((part, ReferencePart::Worksheet));
    }
    parts.extend(
        document
            .chart_part_paths()
            .iter()
            .map(|part| (part, ReferencePart::Chart)),
    );
    parts.extend(
        document
            .pivot_cache_part_paths()
            .iter()
            .map(|part| (part, ReferencePart::PivotCache)),
    );
    for (part, kind) in parts {
        if removed_parts.contains(part) {
            continue;
        }
        let patched = match replacements.get(part) {
            Some(bytes) => patch_sheet_references(bytes, part, kind, &requests, limits)?,
            None => patch_sheet_references(
                &source_reader.read_part(part)?,
                part,
                kind,
                &requests,
                limits,
            )?,
        };
        if let Some(bytes) = patched {
            replacements.insert(part.clone(), bytes);
        }
    }
    Ok(())
}

fn patch_sheet_references(
    bytes: &[u8],
    source: &PartPath,
    part: ReferencePart,
    requests: &SheetReferenceRequests<'_>,
    limits: WriteLimits,
) -> Result<Option<Vec<u8>>, XlsxWriteError> {
    let never_cancelled = || false;
    let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &never_cancelled);
    let mut xml = configured_reader(bytes, limits);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut pivot_source_depth = None::<u64>;
    let mut sparkline_groups_depth = None::<u64>;
    let mut held = None::<HeldText>;
    let mut changed = false;
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        if let Some(text) = held.as_mut()
            && !matches!(event, Event::Eof)
        {
            match event {
                Event::Text(content) => {
                    text.text
                        .push_str(&content.xml_content(XmlVersion::Implicit1_0));
                    text.events.push(Event::Text(content.into_owned()));
                }
                Event::CData(content) => {
                    text.text
                        .push_str(&content.xml_content(XmlVersion::Implicit1_0));
                    text.events.push(Event::CData(content.into_owned()));
                }
                Event::GeneralRef(reference) => {
                    text.text.push_str(&decode_reference(&reference, source)?);
                    text.events.push(Event::GeneralRef(reference.into_owned()));
                }
                Event::End(element) if depth == text.depth => {
                    let text = held.take().expect("held text");
                    let outcome =
                        write_held_text(&mut writer, text, requests, &mut budget, source)?;
                    changed |= outcome != HeldOutcome::Unchanged;
                    if outcome != HeldOutcome::Removed {
                        write_event(&mut writer, Event::End(element), source)?;
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Start(element) => {
                    depth = depth.saturating_add(1);
                    enforce_depth(depth, limits, source)?;
                    text.markup = true;
                    text.events.push(Event::Start(element.into_owned()));
                }
                Event::End(element) => {
                    depth = depth.saturating_sub(1);
                    text.events.push(Event::End(element.into_owned()));
                }
                other => {
                    text.markup = true;
                    text.events.push(other.into_owned());
                }
            }
            buffer.clear();
            continue;
        }
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                let kind = held_text_kind(
                    &xml,
                    &element,
                    part,
                    depth,
                    pivot_source_depth,
                    sparkline_groups_depth.is_some(),
                );
                if part == ReferencePart::Chart
                    && element.local_name().as_ref() == "pivotSource"
                    && in_namespace(&xml, element.name(), CHART)
                {
                    pivot_source_depth = Some(depth);
                }
                if part == ReferencePart::Worksheet
                    && element.local_name().as_ref() == "sparklineGroups"
                    && in_namespace(&xml, element.name(), SPREADSHEETML_2009)
                {
                    sparkline_groups_depth = Some(depth);
                }
                let patched = patch_attribute(&xml, &element, part, requests, &mut budget, source)?;
                changed |= patched.is_some();
                let element = patched.unwrap_or_else(|| element.into_owned());
                let start = if kind == Some(HeldTextKind::SparklineFormula) {
                    Some(element)
                } else {
                    write_event(&mut writer, Event::Start(element), source)?;
                    None
                };
                if let Some(kind) = kind {
                    held = Some(HeldText {
                        depth,
                        kind,
                        start,
                        events: Vec::new(),
                        text: String::new(),
                        markup: false,
                    });
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                let patched = patch_attribute(&xml, &element, part, requests, &mut budget, source)?;
                changed |= patched.is_some();
                write_event(
                    &mut writer,
                    Event::Empty(patched.unwrap_or(element)),
                    source,
                )?;
            }
            Event::End(element) => {
                if pivot_source_depth == Some(depth) {
                    pivot_source_depth = None;
                }
                if sparkline_groups_depth == Some(depth) {
                    sparkline_groups_depth = None;
                }
                write_event(&mut writer, Event::End(element), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other, source)?,
        }
        buffer.clear();
    }
    if !changed {
        return Ok(None);
    }
    if let Some(text) = held {
        for event in text.start.map(Event::Start).into_iter().chain(text.events) {
            write_event(&mut writer, event, source)?;
        }
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(Some(output))
}

/// Selects the elements whose text is a formula, plus the `[file]Sheet!PivotTable` name that ties
/// a pivot chart to its pivot table.
fn held_text_kind(
    xml: &NsReader<&[u8]>,
    element: &BytesStart<'_>,
    part: ReferencePart,
    depth: u64,
    pivot_source_depth: Option<u64>,
    in_sparkline_groups: bool,
) -> Option<HeldTextKind> {
    let local_name = element.local_name();
    let local_name = local_name.as_ref();
    let name = element.name();
    let formula = match part {
        ReferencePart::Worksheet => {
            if local_name == "f" && in_namespace(xml, name, EXCEL_MAIN) {
                return Some(if in_sparkline_groups {
                    HeldTextKind::SparklineFormula
                } else {
                    HeldTextKind::Formula
                });
            }
            matches!(local_name, "formula" | "formula1" | "formula2")
                && in_namespace(xml, name, SPREADSHEETML)
        }
        ReferencePart::Chart => {
            if local_name == "name"
                && pivot_source_depth.is_some_and(|parent| depth == parent + 1)
                && in_namespace(xml, name, CHART)
            {
                return Some(HeldTextKind::PivotSourceName);
            }
            (local_name == "f"
                && (in_namespace(xml, name, CHART) || in_namespace(xml, name, CHART_EX)))
                || (matches!(local_name, "f" | "sqref") && in_namespace(xml, name, CHART_2012))
        }
        ReferencePart::PivotCache => false,
    };
    formula.then_some(HeldTextKind::Formula)
}

/// Rewrites the one attribute that can name a sheet: an internal hyperlink location, a
/// conditional-format threshold, or a pivot cache source sheet. Returns `None` when the start
/// tag is unchanged.
///
/// A deletion leaves a hyperlink location as it is, as Excel does, and refuses a pivot cache
/// whose source is a deleted sheet or a table on one.
fn patch_attribute(
    xml: &NsReader<&[u8]>,
    element: &BytesStart<'_>,
    part: ReferencePart,
    requests: &SheetReferenceRequests<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
    source: &PartPath,
) -> Result<Option<BytesStart<'static>>, XlsxWriteError> {
    let local_name = element.local_name();
    let target = match (part, local_name.as_ref()) {
        (ReferencePart::Worksheet, "hyperlink") => "location",
        (ReferencePart::Worksheet, "cfvo") => "val",
        (ReferencePart::PivotCache, "worksheetSource" | "rangeSet") => "sheet",
        _ => return Ok(None),
    };
    if !in_namespace(xml, element.name(), SPREADSHEETML) {
        return Ok(None);
    }
    let mut value = None;
    let mut source_name = None;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        let (namespace, attribute_name) = xml.resolver().resolve_attribute(attribute.key);
        // A relationship ID points the reference into another package: an external hyperlink
        // target or an external workbook source, whose sheets this rename does not touch.
        if attribute_name.as_ref() == "id"
            && matches!(
                namespace,
                ResolveResult::Bound(namespace) if RELATIONSHIPS.contains(&namespace.as_ref())
            )
        {
            return Ok(None);
        }
        if attribute.key.as_ref() == target || attribute.key.as_ref() == "name" {
            let decoded = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|error| invalid_xml(source, error))?
                .into_owned();
            if attribute.key.as_ref() == target {
                value = Some(decoded);
            } else {
                source_name = Some(decoded);
            }
        }
    }
    if part == ReferencePart::PivotCache
        && (value
            .as_deref()
            .is_some_and(|sheet| requests.deleted(sheet))
            || source_name.as_deref().is_some_and(|table| {
                requests
                    .changes
                    .deleted_tables
                    .iter()
                    .any(|deleted| case_insensitive_eq(table, deleted))
            }))
    {
        return Err(reads_deleted_sheet(source, DETAIL_DELETED_PIVOT_SOURCE));
    }
    let Some(value) = value else {
        return Ok(None);
    };
    let replacement = match part {
        ReferencePart::PivotCache => requests.renamed(&value).map(str::to_owned),
        _ if target == "location" => rewrite(&value, &requests.renames_only, budget, source)?,
        _ => rewrite(&value, &requests.combined, budget, source)?,
    };
    let Some(replacement) = replacement else {
        return Ok(None);
    };
    let escaped = escape_attribute(&replacement)?;
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == target {
            patched.push_attribute(Attribute {
                key: attribute.key,
                value: Cow::Borrowed(&escaped),
            });
        } else {
            patched.push_attribute(attribute);
        }
    }
    Ok(Some(patched.into_owned()))
}

/// Writes a held element's content, and its start tag when that was held too, and reports how
/// it was written. The caller writes the end tag unless the element was removed.
fn write_held_text(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    held: HeldText,
    requests: &SheetReferenceRequests<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
    source: &PartPath,
) -> Result<HeldOutcome, XlsxWriteError> {
    let replacement = match held.kind {
        HeldTextKind::SparklineFormula
            if rewrite(&held.text, &requests.deletions_only, budget, source)?.is_some() =>
        {
            return Ok(HeldOutcome::Removed);
        }
        HeldTextKind::Formula | HeldTextKind::SparklineFormula => {
            rewrite(&held.text, &requests.combined, budget, source)?
        }
        HeldTextKind::PivotSourceName => {
            if pivot_source_names_sheet(&held.text, |sheet| requests.deleted(sheet)) {
                return Err(reads_deleted_sheet(source, DETAIL_DELETED_PIVOT_CHART));
            }
            if pivot_source_names_sheet(&held.text, |sheet| requests.renamed(sheet).is_some()) {
                return Err(unrewritable(source));
            }
            None
        }
    };
    if let Some(start) = held.start {
        write_event(writer, Event::Start(start), source)?;
    }
    match replacement {
        Some(_) if held.markup => Err(unrewritable(source)),
        Some(text) => {
            write_event(
                writer,
                Event::Text(BytesText::from_escaped(escape_text(&text)?)),
                source,
            )?;
            Ok(HeldOutcome::Rewritten)
        }
        None => {
            for event in held.events {
                write_event(writer, event, source)?;
            }
            Ok(HeldOutcome::Unchanged)
        }
    }
}

/// Reports whether a pivot chart source, `[file]Sheet!PivotTable`, names a sheet `matches`
/// selects. That name has no formula grammar to rewrite the sheet with, so the change is refused
/// instead.
fn pivot_source_names_sheet(name: &str, matches: impl Fn(&str) -> bool) -> bool {
    name.match_indices('!').any(|(index, _)| {
        let qualifier = &name[..index];
        let sheet = qualifier
            .rsplit_once(']')
            .map_or(qualifier, |(_, sheet)| sheet)
            .trim_matches('\'')
            .replace("''", "'");
        matches(&sheet)
    })
}

/// Rewrites one preserved formula with the same typed sheet-qualifier edits as cell formulas. A
/// formula that names a renamed or deleted sheet but does not parse is refused rather than left
/// stale.
fn rewrite(
    formula: &str,
    request: &FormulaRewriteRequest<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
    source: &PartPath,
) -> Result<Option<String>, XlsxWriteError> {
    rewrite_formula(formula, request, budget).map_err(|error| match error {
        FormulaRewriteError::Parse { .. } | FormulaRewriteError::SourceEdit(_) => {
            unrewritable(source)
        }
        FormulaRewriteError::Cancelled | FormulaRewriteError::LimitExceeded { .. } => {
            unreachable!("an unbounded non-cancellable rewrite cannot exhaust rewrite control")
        }
    })
}

fn decode_reference(reference: &BytesRef<'_>, source: &PartPath) -> Result<String, XlsxWriteError> {
    unescape(&format!("&{};", reference.as_ref()))
        .map(Cow::into_owned)
        .map_err(|error| invalid_xml(source, error))
}

fn in_namespace(xml: &NsReader<&[u8]>, name: QName<'_>, namespaces: &[&str]) -> bool {
    matches!(
        xml.resolver().resolve_element(name).0,
        ResolveResult::Bound(namespace) if namespaces.contains(&namespace.as_ref())
    )
}

fn configured_reader(bytes: &[u8], limits: WriteLimits) -> NsReader<&[u8]> {
    let mut reader = namespace_reader(bytes, limits.max_xml_depth());
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
}

fn enforce_depth(depth: u64, limits: WriteLimits, source: &PartPath) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        return Err(resource_error(
            source,
            DETAIL_XML_DEPTH,
            depth,
            limits.max_xml_depth(),
        ));
    }
    Ok(())
}

fn enforce_bytes(
    actual: usize,
    limits: WriteLimits,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if actual as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            DETAIL_XML_BYTES,
            actual as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    Ok(())
}

fn write_event(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    event: Event<'_>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    writer
        .write_event(event)
        .map_err(|error| invalid_xml(source, error))
}

fn unrewritable(source: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(DETAIL_UNREWRITABLE_REFERENCE)
        .at_source(source.source_id())
}

/// Refuses a part whose source names a deleted sheet, naming the part in the detail.
fn reads_deleted_sheet(source: &PartPath, detail: &str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(format!("{detail}: {}", source.as_str()))
        .at_source(source.source_id())
}

fn invalid_xml(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_cause(cause)
}

fn resource_error(
    source: &PartPath,
    name: &'static str,
    actual: u64,
    maximum: u64,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!("{name}: {actual} > {maximum}"))
        .at_source(source.source_id())
}

#[cfg(test)]
mod tests {
    use super::{ReferencePart, SheetReferenceRequests, patch_sheet_references};
    use crate::calculation::formula_rewrite::SheetChanges;
    use crate::xlsx::package::PartPath;
    use crate::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};

    const RENAMES: &[(&str, &str)] = &[("Data", "New Data")];

    fn rename_requests() -> SheetReferenceRequests<'static> {
        SheetReferenceRequests::new(SheetChanges {
            renames: RENAMES,
            deleted: &[],
            tabs: &[],
            non_worksheets: &[],
            deleted_tables: &[],
        })
    }

    fn deletion_requests() -> SheetReferenceRequests<'static> {
        SheetReferenceRequests::new(SheetChanges {
            renames: &[("Other", "Renamed")],
            deleted: &["Data"],
            tabs: &["Data", "Other", "Summary"],
            non_worksheets: &[],
            deleted_tables: &["Things"],
        })
    }

    fn patch(part: ReferencePart, xml: &str) -> Option<String> {
        let source = PartPath::from_archive_name(b"xl/part.xml").expect("part");
        patch_sheet_references(
            xml.as_bytes(),
            &source,
            part,
            &rename_requests(),
            WriteLimits::default(),
        )
        .expect("patch")
        .map(|bytes| String::from_utf8(bytes).expect("UTF-8"))
    }

    fn patch_deletion(part: ReferencePart, xml: &str) -> Result<Option<String>, XlsxWriteError> {
        let source = PartPath::from_archive_name(b"xl/part.xml").expect("part");
        patch_sheet_references(
            xml.as_bytes(),
            &source,
            part,
            &deletion_requests(),
            WriteLimits::default(),
        )
        .map(|bytes| bytes.map(|bytes| String::from_utf8(bytes).expect("UTF-8")))
    }

    #[test]
    fn deleted_sheet_references_in_worksheet_parts_follow_excel() {
        let worksheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><conditionalFormatting sqref="A1"><cfRule type="expression" priority="1"><formula>'Data'!$A$1&gt;0</formula></cfRule></conditionalFormatting><conditionalFormatting sqref="A2"><cfRule type="colorScale" priority="2"><colorScale><cfvo type="formula" val="Data!$A$1"/><cfvo type="max"/></colorScale></cfRule></conditionalFormatting><dataValidations count="1"><dataValidation type="list" sqref="B1"><formula1>Data!$A$1:$A$3</formula1></dataValidation></dataValidations><hyperlinks><hyperlink ref="C1" location="'Data'!A1"/><hyperlink ref="C2" location="Other!A1"/></hyperlinks><extLst><ext uri="{CF}"><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="expression"><xm:f>Data!$A$2&gt;1</xm:f></x14:cfRule><xm:sqref>A3</xm:sqref></x14:conditionalFormatting></x14:conditionalFormattings></ext><ext uri="{SP}"><x14:sparklineGroups><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>'Data'!A1:A3</xm:f><xm:sqref>D1</xm:sqref></x14:sparkline><x14:sparkline><xm:f>Other!A1:A3</xm:f><xm:sqref>D2</xm:sqref></x14:sparkline><x14:sparkline><xm:f>Summary!A1:A3</xm:f><xm:sqref>D3</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst></worksheet>"#;
        assert_eq!(
            patch_deletion(ReferencePart::Worksheet, worksheet).expect("patched"),
            Some(
                worksheet
                    .replace(
                        "<formula>'Data'!$A$1&gt;0</formula>",
                        "<formula>#REF!&gt;0</formula>"
                    )
                    .replace(r#"val="Data!$A$1""#, r##"val="#REF!""##)
                    .replace(
                        "<formula1>Data!$A$1:$A$3</formula1>",
                        "<formula1>#REF!</formula1>"
                    )
                    .replace(r#"location="Other!A1""#, r#"location="Renamed!A1""#)
                    .replace("<xm:f>Data!$A$2&gt;1</xm:f>", "<xm:f>#REF!&gt;1</xm:f>")
                    .replace("<xm:f>'Data'!A1:A3</xm:f>", "")
                    .replace("<xm:f>Other!A1:A3</xm:f>", "<xm:f>Renamed!A1:A3</xm:f>")
            )
        );
    }

    #[test]
    fn deleted_sheet_chart_series_become_ref_errors() {
        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:ser><c:tx><c:strRef><c:f>Summary!$A$1</c:f></c:strRef></c:tx><c:val><c:numRef><c:f>'Data'!$A$1:$A$3</c:f></c:numRef></c:val></c:ser></c:chartSpace>"#;
        assert_eq!(
            patch_deletion(ReferencePart::Chart, chart).expect("patched"),
            Some(chart.replace("<c:f>'Data'!$A$1:$A$3</c:f>", "<c:f>#REF!</c:f>"))
        );
    }

    #[test]
    fn pivot_sources_reading_a_deleted_sheet_are_refused_with_their_part() {
        let cache = |source: &str| {
            format!(
                r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><cacheSource type="worksheet">{source}</cacheSource></pivotCacheDefinition>"#
            )
        };
        for source in [
            r#"<worksheetSource ref="A1:B3" sheet="data"/>"#,
            r#"<worksheetSource name="things"/>"#,
            r#"<consolidation><rangeSets count="1"><rangeSet ref="A1:B3" sheet="Data"/></rangeSets></consolidation>"#,
        ] {
            let error = patch_deletion(ReferencePart::PivotCache, &cache(source))
                .expect_err("a pivot cache reading a deleted sheet cannot be preserved");
            assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
            assert_eq!(
                error.detail(),
                Some(
                    "a preserved pivot cache reads a deleted sheet or a table on one: xl/part.xml"
                )
            );
        }
        let other = cache(r#"<worksheetSource ref="A1:B3" sheet="Other"/>"#);
        assert_eq!(
            patch_deletion(ReferencePart::PivotCache, &other).expect("renamed source"),
            Some(other.replace(r#"sheet="Other""#, r#"sheet="Renamed""#))
        );
        let external = cache(r#"<worksheetSource ref="A1:B3" sheet="Data" r:id="rId1"/>"#);
        assert_eq!(
            patch_deletion(ReferencePart::PivotCache, &external).expect("external source"),
            None
        );

        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:pivotSource><c:name>[Book1.xlsx]Data!PivotTable1</c:name></c:pivotSource></c:chartSpace>"#;
        let error = patch_deletion(ReferencePart::Chart, chart)
            .expect_err("a pivot chart of a deleted sheet cannot be preserved");
        assert_eq!(
            error.detail(),
            Some("a preserved pivot chart reads a pivot table on a deleted sheet: xl/part.xml")
        );
    }

    #[test]
    fn pivot_cache_sources_rewrite_only_local_sheets() {
        let consolidation = r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><cacheSource type="consolidation"><consolidation><rangeSets count="2"><rangeSet ref="A1:B3" sheet="data"/><rangeSet ref="A1:B3" sheet="Data" r:id="rId1"/></rangeSets></consolidation></cacheSource></pivotCacheDefinition>"#;
        assert_eq!(
            patch(ReferencePart::PivotCache, consolidation),
            Some(consolidation.replacen(r#"sheet="data""#, r#"sheet="New Data""#, 1))
        );
        let external = r#"<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><cacheSource type="worksheet"><worksheetSource ref="A1:B3" sheet="Data" r:id="rId1"/></cacheSource></pivotCacheDefinition>"#;
        assert_eq!(patch(ReferencePart::PivotCache, external), None);
    }

    #[test]
    fn chart_extension_and_chartex_formulas_are_rewritten() {
        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:c15="http://schemas.microsoft.com/office/drawing/2012/chart"><c:extLst><c:ext><c15:fullRef><c15:sqref>Data!$A$1:$A$9</c15:sqref></c15:fullRef><c15:f>'Data'!$B$1</c15:f></c:ext></c:extLst></c:chartSpace>"#;
        assert_eq!(
            patch(ReferencePart::Chart, chart),
            Some(
                chart
                    .replace("Data!$A$1:$A$9", "'New Data'!$A$1:$A$9")
                    .replace("'Data'!$B$1", "'New Data'!$B$1")
            )
        );
        let chartex = r#"<cx:chartSpace xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex"><cx:chartData><cx:data id="0"><cx:numDim type="val"><cx:f dir="row">Data!$A$1:$C$1</cx:f></cx:numDim></cx:data></cx:chartData></cx:chartSpace>"#;
        assert_eq!(
            patch(ReferencePart::Chart, chartex),
            Some(chartex.replace("Data!$A$1", "'New Data'!$A$1"))
        );
    }

    #[test]
    fn chart_part_that_is_not_utf8_fails_instead_of_keeping_a_stale_reference() {
        let chart = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:f>Data!$A$1</c:f></c:chartSpace>"#;
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(chart.encode_utf16().flat_map(u16::to_le_bytes));
        let source = PartPath::from_archive_name(b"xl/charts/chart1.xml").expect("part");
        let error = patch_sheet_references(
            &utf16,
            &source,
            ReferencePart::Chart,
            &rename_requests(),
            WriteLimits::default(),
        )
        .expect_err("a chart that cannot be read cannot be rewritten");
        assert_eq!(error.code(), XlsxWriteErrorCode::InvalidGeneratedXml);
    }

    #[test]
    fn chart_elements_may_each_declare_up_to_256_namespaces() {
        let declarations = |prefix: &str, count: usize| {
            (0..count)
                .map(|index| format!(r#" xmlns:{prefix}{index}="urn:{prefix}{index}""#))
                .collect::<String>()
        };
        let chart = |space: &str, plot: &str| {
            format!(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"{space}><c:chart{plot}><c:f>Data!$A$1</c:f></c:chart></c:chartSpace>"#
            )
        };
        let nested = chart(&declarations("a", 255), &declarations("b", 256));
        assert_eq!(
            patch(ReferencePart::Chart, &nested),
            Some(nested.replace("Data!$A$1", "'New Data'!$A$1"))
        );

        let source = PartPath::from_archive_name(b"xl/charts/chart1.xml").expect("part");
        let error = patch_sheet_references(
            chart(&declarations("a", 256), "").as_bytes(),
            &source,
            ReferencePart::Chart,
            &rename_requests(),
            WriteLimits::default(),
        )
        .expect_err("257 declarations on one element");
        assert_eq!(error.code(), XlsxWriteErrorCode::InvalidGeneratedXml);
        assert_eq!(error.detail(), None);
    }

    #[test]
    fn formula_text_with_markup_is_refused_only_when_it_needs_a_rewrite() {
        let source = PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part");
        let worksheet = |formula: &str| {
            format!(
                r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><conditionalFormatting sqref="A1"><cfRule type="expression" priority="1"><formula>{formula}</formula></cfRule></conditionalFormatting></worksheet>"#
            )
        };
        let error = patch_sheet_references(
            worksheet("Data!A1<!-- note -->&gt;0").as_bytes(),
            &source,
            ReferencePart::Worksheet,
            &rename_requests(),
            WriteLimits::default(),
        )
        .expect_err("a rewrite would drop the comment");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
        assert_eq!(
            patch(
                ReferencePart::Worksheet,
                &worksheet("Other!A1<!-- note -->&gt;0")
            ),
            None
        );
    }
}
