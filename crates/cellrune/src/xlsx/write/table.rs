use super::serialization::{escape_attribute, escape_text};
use super::{XlsxWriteError, XlsxWriteErrorCode};
use crate::{
    CellAddress, CellContent, CellValue, Sheet, Table, TableAutoFilter, TableFilterCriteria,
    TableFilterItem, TableFormula, TableSortState, TableType,
};

pub(crate) const TABLE_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
const TABLE_RELATIONSHIP_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const DETAIL_OPAQUE_TABLE: &str =
    "canonical table serialization cannot preserve unmodeled source metadata";
const DETAIL_EXTERNAL_TABLE_TYPE: &str =
    "canonical table serialization cannot synthesize query or XML table dependencies";
const DETAIL_EXTERNAL_STYLE_DEPENDENCY: &str =
    "canonical table serialization cannot synthesize differential or custom table styles";
const DETAIL_TABLE_HEADER: &str =
    "canonical table header cells must exactly match the table column names";
const DETAIL_INVALID_TABLE_SCALAR: &str =
    "canonical table serialization requires OOXML-valid table and column names";

pub(crate) fn table_part_name(table: &Table) -> String {
    format!("xl/tables/table{}.xml", table.id().get())
}

pub(crate) fn worksheet_relationships_part_name(sheet_index: usize) -> String {
    format!("xl/worksheets/_rels/sheet{}.xml.rels", sheet_index + 1)
}

pub(crate) fn table_relationship_id(table: &Table) -> String {
    format!("rIdCellRuneTable{}", table.id().get())
}

pub(crate) fn worksheet_relationships_xml(sheet: &Sheet) -> Option<String> {
    if sheet.tables().is_empty() {
        return None;
    }
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for table in sheet.tables() {
        xml.push_str("<Relationship Id=\"");
        xml.push_str(&table_relationship_id(table));
        xml.push_str("\" Type=\"");
        xml.push_str(TABLE_RELATIONSHIP_TYPE);
        xml.push_str("\" Target=\"../tables/table");
        xml.push_str(&table.id().get().to_string());
        xml.push_str(".xml\"/>");
    }
    xml.push_str("</Relationships>");
    Some(xml)
}

pub(crate) fn validate_table_headers(sheet: &Sheet) -> Result<(), XlsxWriteError> {
    for table in sheet.tables() {
        match table.header_row_count() {
            0 => continue,
            1 => {}
            _ => {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                        .with_detail(DETAIL_TABLE_HEADER),
                );
            }
        }
        let row = table.range().start().row().get();
        let first_column = table.range().start().column().get();
        for (offset, column) in table.columns().iter().enumerate() {
            let offset = u32::try_from(offset).expect("table width fits in u32");
            let address = CellAddress::from_indices(row, first_column + offset)
                .expect("validated table range contains every header address");
            let matches = sheet.cell(address).is_some_and(|cell| {
                matches!(
                    cell.content(),
                    CellContent::Literal(CellValue::Text(value)) if value == column.name()
                )
            });
            if !matches {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                        .with_detail(DETAIL_TABLE_HEADER),
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn push_table_parts(output: &mut String, sheet: &Sheet) {
    if sheet.tables().is_empty() {
        return;
    }
    output.push_str("<tableParts count=\"");
    output.push_str(&sheet.tables().len().to_string());
    output.push_str("\">");
    for table in sheet.tables() {
        output.push_str("<tablePart r:id=\"");
        output.push_str(&table_relationship_id(table));
        output.push_str("\"/>");
    }
    output.push_str("</tableParts>");
}

pub(crate) fn table_xml(table: &Table) -> Result<String, XlsxWriteError> {
    if table.has_opaque_metadata() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                .with_detail(DETAIL_OPAQUE_TABLE),
        );
    }
    if table.table_type() != TableType::Worksheet {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                .with_detail(DETAIL_EXTERNAL_TABLE_TYPE),
        );
    }
    validate_canonical_table_scalars(table)?;
    validate_canonical_style_dependencies(table)?;
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id=""#,
    );
    xml.push_str(&table.id().get().to_string());
    xml.push_str("\" name=\"");
    xml.push_str(&escape_attribute(table.name().as_str())?);
    xml.push_str("\" displayName=\"");
    xml.push_str(&escape_attribute(table.display_name().as_str())?);
    xml.push_str("\" ref=\"");
    push_range(&mut xml, table.range());
    xml.push('"');
    xml.push_str(" headerRowCount=\"");
    xml.push_str(&table.header_row_count().to_string());
    xml.push_str("\" totalsRowCount=\"");
    xml.push_str(&table.totals_row_count().to_string());
    xml.push_str(if table.totals_row_shown() {
        "\" totalsRowShown=\"1\">"
    } else {
        "\" totalsRowShown=\"0\">"
    });
    if let Some(filter) = table.auto_filter() {
        push_auto_filter(&mut xml, filter)?;
    }
    if let Some(sort_state) = table.sort_state() {
        push_sort_state(&mut xml, sort_state)?;
    }
    xml.push_str("<tableColumns count=\"");
    xml.push_str(&table.columns().len().to_string());
    xml.push_str("\">");
    for column in table.columns() {
        xml.push_str("<tableColumn id=\"");
        xml.push_str(&column.id().to_string());
        xml.push_str("\" name=\"");
        xml.push_str(&escape_attribute(column.name())?);
        xml.push('"');
        if let Some(function) = column.totals_row_function() {
            xml.push_str(" totalsRowFunction=\"");
            xml.push_str(function.as_str());
            xml.push('"');
        }
        if let Some(label) = column.totals_row_label() {
            xml.push_str(" totalsRowLabel=\"");
            xml.push_str(&escape_attribute(label)?);
            xml.push('"');
        }
        if column.calculated_column_formula().is_none() && column.totals_row_formula().is_none() {
            xml.push_str("/>");
            continue;
        }
        xml.push('>');
        if let Some(formula) = column.calculated_column_formula() {
            push_formula(&mut xml, "calculatedColumnFormula", formula)?;
        }
        if let Some(formula) = column.totals_row_formula() {
            push_formula(&mut xml, "totalsRowFormula", formula)?;
        }
        xml.push_str("</tableColumn>");
    }
    xml.push_str("</tableColumns>");
    if let Some(style) = table.style_info() {
        xml.push_str("<tableStyleInfo");
        if let Some(name) = style.name() {
            xml.push_str(" name=\"");
            xml.push_str(&escape_attribute(name)?);
            xml.push('"');
        }
        push_bool_attribute(&mut xml, "showFirstColumn", style.show_first_column());
        push_bool_attribute(&mut xml, "showLastColumn", style.show_last_column());
        push_bool_attribute(&mut xml, "showRowStripes", style.show_row_stripes());
        push_bool_attribute(&mut xml, "showColumnStripes", style.show_column_stripes());
        xml.push_str("/>");
    }
    xml.push_str("</table>");
    Ok(xml)
}

fn validate_canonical_table_scalars(table: &Table) -> Result<(), XlsxWriteError> {
    let valid = table.name().validate_xlsx().is_ok()
        && table.display_name().validate_xlsx().is_ok()
        && table
            .columns()
            .iter()
            .all(|column| column.validate_xlsx().is_ok());
    if !valid {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                .with_detail(DETAIL_INVALID_TABLE_SCALAR),
        );
    }
    Ok(())
}

fn validate_canonical_style_dependencies(table: &Table) -> Result<(), XlsxWriteError> {
    let uses_differential_format = table
        .auto_filter()
        .is_some_and(auto_filter_uses_differential_format)
        || table
            .sort_state()
            .is_some_and(sort_state_uses_differential_format);
    let uses_custom_table_style = table
        .style_info()
        .and_then(crate::TableStyleInfo::name)
        .is_some_and(|name| !is_builtin_table_style(name));
    if uses_differential_format || uses_custom_table_style {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                .with_detail(DETAIL_EXTERNAL_STYLE_DEPENDENCY),
        );
    }
    Ok(())
}

fn auto_filter_uses_differential_format(filter: &TableAutoFilter) -> bool {
    filter.filter_columns().iter().any(|column| {
        matches!(
            column.criteria(),
            Some(TableFilterCriteria::Color(color))
                if color.differential_format_id().is_some()
        )
    }) || filter
        .sort_state()
        .is_some_and(sort_state_uses_differential_format)
}

fn sort_state_uses_differential_format(sort_state: &TableSortState) -> bool {
    sort_state
        .conditions()
        .iter()
        .any(|condition| condition.differential_format_id().is_some())
}

fn is_builtin_table_style(name: &str) -> bool {
    [
        ("TableStyleLight", 21_u32),
        ("TableStyleMedium", 28_u32),
        ("TableStyleDark", 11_u32),
    ]
    .iter()
    .any(|(prefix, maximum)| {
        name.strip_prefix(prefix)
            .and_then(|suffix| {
                suffix
                    .parse::<u32>()
                    .ok()
                    .filter(|number| suffix == number.to_string())
            })
            .is_some_and(|number| (1..=*maximum).contains(&number))
    })
}

fn push_auto_filter(output: &mut String, filter: &TableAutoFilter) -> Result<(), XlsxWriteError> {
    output.push_str("<autoFilter");
    if let Some(range) = filter.declared_range() {
        output.push_str(" ref=\"");
        push_range(output, range);
        output.push('"');
    }
    output.push('>');
    for column in filter.filter_columns() {
        output.push_str("<filterColumn colId=\"");
        output.push_str(&column.column_id().to_string());
        output.push('"');
        push_bool_attribute(output, "hiddenButton", column.hidden_button());
        push_bool_attribute(output, "showButton", column.show_button());
        if column.criteria().is_none() {
            output.push_str("/>");
            continue;
        }
        output.push('>');
        match column.criteria().expect("criteria checked above") {
            TableFilterCriteria::Values(filters) => {
                output.push_str("<filters");
                push_bool_attribute(output, "blank", filters.blank());
                if let Some(calendar_type) = filters.calendar_type() {
                    push_escaped_attribute(output, "calendarType", calendar_type.as_str())?;
                }
                if filters.items().is_empty() {
                    output.push_str("/>");
                } else {
                    output.push('>');
                    for item in filters.items() {
                        match item {
                            TableFilterItem::Value(value) => {
                                output.push_str("<filter");
                                if let Some(value) = value {
                                    push_escaped_attribute(output, "val", value)?;
                                }
                                output.push_str("/>");
                            }
                            TableFilterItem::DateGroup(item) => {
                                output.push_str("<dateGroupItem year=\"");
                                output.push_str(&item.year().to_string());
                                output.push('"');
                                push_optional_u16_attribute(output, "month", item.month());
                                push_optional_u16_attribute(output, "day", item.day());
                                push_optional_u16_attribute(output, "hour", item.hour());
                                push_optional_u16_attribute(output, "minute", item.minute());
                                push_optional_u16_attribute(output, "second", item.second());
                                push_escaped_attribute(
                                    output,
                                    "dateTimeGrouping",
                                    item.grouping().as_str(),
                                )?;
                                output.push_str("/>");
                            }
                        }
                    }
                    output.push_str("</filters>");
                }
            }
            TableFilterCriteria::Custom(filters) => {
                output.push_str("<customFilters");
                push_bool_attribute(output, "and", filters.and());
                if filters.filters().is_empty() {
                    output.push_str("/>");
                } else {
                    output.push('>');
                    for filter in filters.filters() {
                        output.push_str("<customFilter");
                        if let Some(operator) = filter.operator() {
                            push_escaped_attribute(output, "operator", operator.as_str())?;
                        }
                        if let Some(value) = filter.value() {
                            push_escaped_attribute(output, "val", value)?;
                        }
                        output.push_str("/>");
                    }
                    output.push_str("</customFilters>");
                }
            }
            TableFilterCriteria::Dynamic(filter) => {
                output.push_str("<dynamicFilter");
                push_escaped_attribute(output, "type", filter.kind().as_str())?;
                if let Some(value) = filter.value() {
                    push_escaped_attribute(output, "val", value.as_str())?;
                }
                if let Some(value) = filter.iso_value() {
                    push_escaped_attribute(output, "valIso", value.as_str())?;
                }
                if let Some(value) = filter.max_value() {
                    push_escaped_attribute(output, "maxVal", value.as_str())?;
                }
                if let Some(value) = filter.max_iso_value() {
                    push_escaped_attribute(output, "maxValIso", value.as_str())?;
                }
                output.push_str("/>");
            }
            TableFilterCriteria::Color(filter) => {
                output.push_str("<colorFilter");
                push_optional_u32_attribute(output, "dxfId", filter.differential_format_id());
                push_bool_attribute(output, "cellColor", filter.cell_color());
                output.push_str("/>");
            }
            TableFilterCriteria::Icon(filter) => {
                output.push_str("<iconFilter");
                push_escaped_attribute(output, "iconSet", filter.icon_set().as_str())?;
                push_optional_u32_attribute(output, "iconId", filter.icon_id());
                output.push_str("/>");
            }
            TableFilterCriteria::Top(filter) => {
                output.push_str("<top10");
                push_bool_attribute(output, "top", filter.top());
                push_bool_attribute(output, "percent", filter.percent());
                push_escaped_attribute(output, "val", filter.value().as_str())?;
                if let Some(value) = filter.filter_value() {
                    push_escaped_attribute(output, "filterVal", value.as_str())?;
                }
                output.push_str("/>");
            }
        }
        output.push_str("</filterColumn>");
    }
    if let Some(sort_state) = filter.sort_state() {
        push_sort_state(output, sort_state)?;
    }
    output.push_str("</autoFilter>");
    Ok(())
}

fn push_sort_state(output: &mut String, sort_state: &TableSortState) -> Result<(), XlsxWriteError> {
    output.push_str("<sortState ref=\"");
    push_range(output, sort_state.range());
    output.push('"');
    push_bool_attribute(output, "caseSensitive", sort_state.case_sensitive());
    push_bool_attribute(output, "columnSort", sort_state.column_sort());
    if let Some(sort_method) = sort_state.sort_method() {
        push_escaped_attribute(output, "sortMethod", sort_method.as_str())?;
    }
    if sort_state.conditions().is_empty() {
        output.push_str("/>");
        return Ok(());
    }
    output.push('>');
    for condition in sort_state.conditions() {
        output.push_str("<sortCondition ref=\"");
        push_range(output, condition.range());
        output.push('"');
        push_bool_attribute(output, "descending", condition.descending());
        if let Some(sort_by) = condition.sort_by() {
            push_escaped_attribute(output, "sortBy", sort_by.as_str())?;
        }
        if let Some(custom_list) = condition.custom_list() {
            push_escaped_attribute(output, "customList", custom_list)?;
        }
        push_optional_u32_attribute(output, "dxfId", condition.differential_format_id());
        if let Some(icon_set) = condition.icon_set() {
            push_escaped_attribute(output, "iconSet", icon_set.as_str())?;
        }
        push_optional_u32_attribute(output, "iconId", condition.icon_id());
        output.push_str("/>");
    }
    output.push_str("</sortState>");
    Ok(())
}

fn push_escaped_attribute(
    output: &mut String,
    name: &str,
    value: &str,
) -> Result<(), XlsxWriteError> {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    output.push_str(&escape_attribute(value)?);
    output.push('"');
    Ok(())
}

fn push_optional_u32_attribute(output: &mut String, name: &str, value: Option<u32>) {
    if let Some(value) = value {
        output.push(' ');
        output.push_str(name);
        output.push_str("=\"");
        output.push_str(&value.to_string());
        output.push('"');
    }
}

fn push_optional_u16_attribute(output: &mut String, name: &str, value: Option<u16>) {
    if let Some(value) = value {
        output.push(' ');
        output.push_str(name);
        output.push_str("=\"");
        output.push_str(&value.to_string());
        output.push('"');
    }
}

fn push_formula(
    output: &mut String,
    element: &str,
    formula: &TableFormula,
) -> Result<(), XlsxWriteError> {
    output.push('<');
    output.push_str(element);
    if formula.is_array() {
        output.push_str(" array=\"1\"");
    }
    output.push('>');
    output.push_str(&escape_text(formula.text().as_str())?);
    output.push_str("</");
    output.push_str(element);
    output.push('>');
    Ok(())
}

fn push_bool_attribute(output: &mut String, name: &str, value: bool) {
    output.push(' ');
    output.push_str(name);
    output.push_str(if value { "=\"1\"" } else { "=\"0\"" });
}

fn push_range(output: &mut String, range: crate::CellRange) {
    output.push_str(&range.start().to_string());
    if range.start() != range.end() {
        output.push(':');
        output.push_str(&range.end().to_string());
    }
}
