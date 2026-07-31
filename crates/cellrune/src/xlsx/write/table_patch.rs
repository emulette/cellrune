use std::collections::BTreeSet;
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use super::serialization::escape_attribute;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{SPREADSHEETML_STRICT, SPREADSHEETML_TRANSITIONAL};
use crate::{CellRange, Table, TableColumn, TableSortState};

const DETAIL_TABLE_PATCH_STRUCTURE: &str = "source table XML does not match the stable table model";
const DETAIL_REWRITTEN_XML_BYTES: &str = "max_rewritten_xml_bytes";

struct ActiveFormula {
    depth: u64,
    replacement: String,
    wrote: bool,
}

struct ActiveSort<'a> {
    depth: u64,
    state: &'a TableSortState,
    next_condition: usize,
}

pub(crate) fn patch_table_xml(
    source_bytes: &[u8],
    source: &PartPath,
    table: &Table,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    if source_bytes.len() as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            source_bytes.len() as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    let mut xml = NsReader::from_reader(source_bytes);
    xml.config_mut().check_end_names = true;
    xml.config_mut().allow_unmatched_ends = false;
    xml.config_mut().expand_empty_elements = false;
    xml.config_mut().trim_text(false);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut saw_root = false;
    let mut root_depth = None;
    let mut seen_columns = BTreeSet::new();
    let mut auto_filter_depth = None;
    let mut table_columns_depth = None;
    let mut current_column: Option<(u64, &TableColumn)> = None;
    let mut active_formula: Option<ActiveFormula> = None;
    let mut active_sort: Option<ActiveSort<'_>> = None;

    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                let local = element.local_name();
                let local = local.as_ref();
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                if depth == 1 && spreadsheet && local == b"table" {
                    saw_root = true;
                    root_depth = Some(depth);
                    let patched = patch_attributes(
                        &element,
                        source,
                        &[
                            ("name", Some(table.name().as_str().to_owned())),
                            (
                                "displayName",
                                Some(table.display_name().as_str().to_owned()),
                            ),
                            ("ref", Some(range_text(table.range()))),
                        ],
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if root_depth.is_some_and(|root| depth == root + 1)
                    && spreadsheet
                    && local == b"tableColumns"
                {
                    table_columns_depth = Some(depth);
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if root_depth.is_some_and(|root| depth == root + 1)
                    && spreadsheet
                    && local == b"autoFilter"
                {
                    let replacement = table
                        .auto_filter()
                        .map(|filter| filter.declared_range().map(range_text));
                    let patched =
                        patch_attributes(&element, source, &[("ref", replacement.flatten())])?;
                    auto_filter_depth = Some(depth);
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if spreadsheet
                    && local == b"sortState"
                    && (root_depth.is_some_and(|root| depth == root + 1)
                        || auto_filter_depth.is_some_and(|filter| depth == filter + 1))
                {
                    let state = if auto_filter_depth.is_some_and(|filter| depth == filter + 1) {
                        table.auto_filter().and_then(|filter| filter.sort_state())
                    } else {
                        table.sort_state()
                    }
                    .ok_or_else(|| invalid_structure(source))?;
                    let patched = patch_attributes(
                        &element,
                        source,
                        &[("ref", Some(range_text(state.range())))],
                    )?;
                    active_sort = Some(ActiveSort {
                        depth,
                        state,
                        next_condition: 0,
                    });
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if spreadsheet
                    && local == b"sortCondition"
                    && active_sort
                        .as_ref()
                        .is_some_and(|sort| depth == sort.depth + 1)
                {
                    let sort = active_sort
                        .as_mut()
                        .ok_or_else(|| invalid_structure(source))?;
                    let condition = sort
                        .state
                        .conditions()
                        .get(sort.next_condition)
                        .ok_or_else(|| invalid_structure(source))?;
                    sort.next_condition += 1;
                    let patched = patch_attributes(
                        &element,
                        source,
                        &[("ref", Some(range_text(condition.range())))],
                    )?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if spreadsheet
                    && local == b"tableColumn"
                    && table_columns_depth.is_some_and(|columns| depth == columns + 1)
                {
                    let column = column_for_element(&element, table, source)?;
                    seen_columns.insert(column.id());
                    let patched = patch_attributes(
                        &element,
                        source,
                        &[("name", Some(column.name().to_owned()))],
                    )?;
                    current_column = Some((depth, column));
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if spreadsheet
                    && matches!(local, b"calculatedColumnFormula" | b"totalsRowFormula")
                    && current_column
                        .as_ref()
                        .is_some_and(|(column_depth, _)| depth == *column_depth + 1)
                {
                    let (_, column) = current_column.ok_or_else(|| invalid_structure(source))?;
                    let formula = if local == b"calculatedColumnFormula" {
                        column.calculated_column_formula()
                    } else {
                        column.totals_row_formula()
                    }
                    .ok_or_else(|| invalid_structure(source))?;
                    active_formula = Some(ActiveFormula {
                        depth,
                        replacement: formula.text().as_str().to_owned(),
                        wrote: false,
                    });
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                let local = element.local_name();
                let local = local.as_ref();
                let element_depth = depth.saturating_add(1);
                let spreadsheet = is_spreadsheet_element(&xml, element.name());
                let output = if depth == 0 && spreadsheet && local == b"table" {
                    saw_root = true;
                    patch_attributes(
                        &element,
                        source,
                        &[
                            ("name", Some(table.name().as_str().to_owned())),
                            (
                                "displayName",
                                Some(table.display_name().as_str().to_owned()),
                            ),
                            ("ref", Some(range_text(table.range()))),
                        ],
                    )?
                } else if root_depth.is_some_and(|root| element_depth == root + 1)
                    && spreadsheet
                    && local == b"autoFilter"
                {
                    let replacement = table
                        .auto_filter()
                        .and_then(|filter| filter.declared_range())
                        .map(range_text);
                    patch_attributes(&element, source, &[("ref", replacement)])?
                } else if spreadsheet
                    && local == b"sortState"
                    && (root_depth.is_some_and(|root| element_depth == root + 1)
                        || auto_filter_depth.is_some_and(|filter| element_depth == filter + 1))
                {
                    let state =
                        if auto_filter_depth.is_some_and(|filter| element_depth == filter + 1) {
                            table.auto_filter().and_then(|filter| filter.sort_state())
                        } else {
                            table.sort_state()
                        }
                        .ok_or_else(|| invalid_structure(source))?;
                    if !state.conditions().is_empty() {
                        return Err(invalid_structure(source));
                    }
                    patch_attributes(
                        &element,
                        source,
                        &[("ref", Some(range_text(state.range())))],
                    )?
                } else if spreadsheet
                    && local == b"sortCondition"
                    && active_sort
                        .as_ref()
                        .is_some_and(|sort| element_depth == sort.depth + 1)
                {
                    let sort = active_sort
                        .as_mut()
                        .ok_or_else(|| invalid_structure(source))?;
                    let condition = sort
                        .state
                        .conditions()
                        .get(sort.next_condition)
                        .ok_or_else(|| invalid_structure(source))?;
                    sort.next_condition += 1;
                    patch_attributes(
                        &element,
                        source,
                        &[("ref", Some(range_text(condition.range())))],
                    )?
                } else if spreadsheet
                    && local == b"tableColumn"
                    && table_columns_depth.is_some_and(|columns| element_depth == columns + 1)
                {
                    let column = column_for_element(&element, table, source)?;
                    seen_columns.insert(column.id());
                    patch_attributes(
                        &element,
                        source,
                        &[("name", Some(column.name().to_owned()))],
                    )?
                } else {
                    element.into_owned()
                };
                write_event(&mut writer, Event::Empty(output), source)?;
            }
            Event::Text(_) | Event::CData(_) if active_formula.is_some() => {
                let formula = active_formula.as_mut().expect("checked formula");
                if !formula.wrote {
                    write_event(
                        &mut writer,
                        Event::Text(BytesText::new(&formula.replacement).into_owned()),
                        source,
                    )?;
                    formula.wrote = true;
                }
            }
            Event::End(element) => {
                if active_formula
                    .as_ref()
                    .is_some_and(|formula| formula.depth == depth)
                {
                    let formula = active_formula.as_mut().expect("active formula");
                    if !formula.wrote {
                        write_event(
                            &mut writer,
                            Event::Text(BytesText::new(&formula.replacement).into_owned()),
                            source,
                        )?;
                    }
                    active_formula = None;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                if active_sort.as_ref().is_some_and(|sort| sort.depth == depth) {
                    let sort = active_sort.take().expect("active sort");
                    if sort.next_condition != sort.state.conditions().len() {
                        return Err(invalid_structure(source));
                    }
                }
                if current_column
                    .as_ref()
                    .is_some_and(|(column_depth, _)| *column_depth == depth)
                {
                    current_column = None;
                }
                if auto_filter_depth == Some(depth) {
                    auto_filter_depth = None;
                }
                if table_columns_depth == Some(depth) {
                    table_columns_depth = None;
                }
                if root_depth == Some(depth) {
                    root_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => write_event(&mut writer, other.into_owned(), source)?,
        }
        buffer.clear();
    }
    if !saw_root
        || table
            .columns()
            .iter()
            .any(|column| !seen_columns.contains(&column.id()))
    {
        return Err(invalid_structure(source));
    }
    let output = writer.into_inner().into_inner();
    if output.len() as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            output.len() as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    Ok(output)
}

fn column_for_element<'a>(
    element: &BytesStart<'_>,
    table: &'a Table,
    source: &PartPath,
) -> Result<&'a TableColumn, XlsxWriteError> {
    let mut id = None;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == b"id" {
            id = std::str::from_utf8(attribute.value.as_ref())
                .ok()
                .and_then(|value| value.parse::<u32>().ok());
        }
    }
    let id = id.ok_or_else(|| invalid_structure(source))?;
    table
        .columns()
        .iter()
        .find(|column| column.id() == id)
        .ok_or_else(|| invalid_structure(source))
}

fn patch_attributes(
    element: &BytesStart<'_>,
    source: &PartPath,
    replacements: &[(&str, Option<String>)],
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = std::str::from_utf8(element.name().as_ref())
        .map_err(|error| invalid_xml(source, error))?
        .to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if replacements
            .iter()
            .any(|(name, _)| attribute.key.as_ref() == name.as_bytes())
        {
            continue;
        }
        patched.push_attribute(attribute);
    }
    for (name, value) in replacements {
        if let Some(value) = value {
            let escaped = escape_attribute(value)?;
            patched.push_attribute((name.as_bytes(), escaped.as_bytes()));
        }
    }
    Ok(patched.into_owned())
}

fn range_text(range: CellRange) -> String {
    if range.start() == range.end() {
        range.start().to_string()
    } else {
        format!("{}:{}", range.start(), range.end())
    }
}

fn is_spreadsheet_element(xml: &NsReader<&[u8]>, name: QName<'_>) -> bool {
    matches!(
        xml.resolver().resolve_element(name).0,
        ResolveResult::Bound(namespace)
            if namespace.as_ref() == SPREADSHEETML_TRANSITIONAL
                || namespace.as_ref() == SPREADSHEETML_STRICT
    )
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

fn enforce_depth(depth: u64, limits: WriteLimits, source: &PartPath) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .with_detail(format!(
                    "max_xml_depth: {depth} > {}",
                    limits.max_xml_depth()
                ))
                .at_source(source.source_id()),
        )
    } else {
        Ok(())
    }
}

fn invalid_structure(source: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(DETAIL_TABLE_PATCH_STRUCTURE)
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

fn resource_error(source: &PartPath, actual: u64, maximum: u64) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!(
            "{DETAIL_REWRITTEN_XML_BYTES}: {actual} > {maximum}"
        ))
        .at_source(source.source_id())
}

#[cfg(test)]
mod tests {
    use super::{WriteLimits, patch_table_xml};
    use crate::xlsx::package::PartPath;
    use crate::{
        CellAddress, CellRange, Table, TableColumn, TableColumnId, TableColumnName, TableId,
        TableName,
    };

    #[test]
    fn replacement_attributes_preserve_xml_whitespace_with_character_references() {
        let mut table = Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("Sales").expect("table name"),
            TableName::new("Sales").expect("display name"),
            CellRange::new(
                CellAddress::from_a1("A1").expect("start"),
                CellAddress::from_a1("A2").expect("end"),
            )
            .expect("range"),
            1,
            0,
            vec![TableColumn::new(1, "Amount", None).expect("column")],
        )
        .expect("table");
        assert!(table.rename_column(
            TableColumnId::new(1).expect("column ID"),
            &TableColumnName::new("Gross\nAmount").expect("name"),
        ));
        let source = PartPath::from_archive_name(b"xl/tables/table1.xml").expect("part");
        let xml = br#"<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Sales" displayName="Sales" ref="A1:A2"><tableColumns count="1"><tableColumn id="1" name="Amount"/></tableColumns></table>"#;
        let output = patch_table_xml(xml, &source, &table, WriteLimits::default()).expect("patch");
        let output = String::from_utf8(output).expect("UTF-8");
        assert!(output.contains(r#"name="Gross&#xA;Amount""#), "{output}");
        assert!(!output.contains("&amp;#xA;"), "{output}");
    }
}
