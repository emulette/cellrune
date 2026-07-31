use cellrune_interop::{
    CalculationDeltaDto, CalculationDeltaPageDto, CalculationReportDto, CalculationResultDto,
    CellDto, CellReferenceDto, CellValueDto, DefinedNameInspectionDto,
    DefinedNameInspectionResultDto, DefinedNameReferenceAreaDto, DefinedNameSheetSpanDto,
    EditReceiptDto, EditReceiptV2Dto, FunctionUsageReportDto, RangePageDto, WorkbookSummaryDto,
    WriteReportDto,
};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

pub(crate) fn workbook_summary<'py>(
    py: Python<'py>,
    value: &WorkbookSummaryDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("semantic_revision", value.semantic_revision)?;
    result.set_item("document_backed", value.document_backed)?;
    result.set_item("document_kind", &value.document_kind)?;
    result.set_item("date_system", &value.date_system)?;
    result.set_item("diagnostic_count", value.diagnostic_count)?;
    let sheets = PyList::empty(py);
    for sheet in &value.sheets {
        let item = PyDict::new(py);
        item.set_item("id", sheet.id)?;
        item.set_item("name", &sheet.name)?;
        item.set_item("visibility", &sheet.visibility)?;
        item.set_item("cell_count", sheet.cell_count)?;
        item.set_item("used_range", sheet.used_range.as_deref())?;
        item.set_item("merged_ranges", &sheet.merged_ranges)?;
        let tables = PyList::empty(py);
        for table in &sheet.tables {
            let table_item = PyDict::new(py);
            table_item.set_item("id", table.id)?;
            table_item.set_item("name", &table.name)?;
            table_item.set_item("display_name", &table.display_name)?;
            table_item.set_item("range", &table.range)?;
            table_item.set_item("header_row_count", table.header_row_count)?;
            table_item.set_item("totals_row_count", table.totals_row_count)?;
            let columns = PyList::empty(py);
            for column in &table.columns {
                let column_item = PyDict::new(py);
                column_item.set_item("id", column.id)?;
                column_item.set_item("name", &column.name)?;
                column_item
                    .set_item("totals_row_function", column.totals_row_function.as_deref())?;
                columns.append(column_item)?;
            }
            table_item.set_item("columns", columns)?;
            tables.append(table_item)?;
        }
        item.set_item("tables", tables)?;
        sheets.append(item)?;
    }
    result.set_item("sheets", sheets)?;
    Ok(result)
}

pub(crate) fn calculation_report<'py>(
    py: Python<'py>,
    value: &CalculationReportDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("semantic_revision", value.semantic_revision)?;
    result.set_item("formula_count", value.formula_count)?;
    result.set_item("value_count", value.value_count)?;
    result.set_item("unavailable_count", value.unavailable_count)?;
    result.set_item("materialized_cell_count", value.materialized_cell_count)?;
    Ok(result)
}

pub(crate) fn edit_receipt<'py>(
    py: Python<'py>,
    value: &EditReceiptDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("base_revision", value.base_revision)?;
    result.set_item("result_revision", value.result_revision)?;
    result.set_item("applied_change_count", value.applied_change_count)?;
    let changed_cells = PyList::empty(py);
    for cell in &value.changed_cells {
        changed_cells.append(cell_reference(py, cell)?)?;
    }
    result.set_item("changed_cells", changed_cells)?;
    let calculation_changed_cells = PyList::empty(py);
    for cell in &value.calculation_changed_cells {
        calculation_changed_cells.append(cell_reference(py, cell)?)?;
    }
    result.set_item("calculation_changed_cells", calculation_changed_cells)?;
    result.set_item("created_sheet_ids", &value.created_sheet_ids)?;
    result.set_item("topology_changed", value.topology_changed)?;
    result.set_item(
        "calculation_metadata_changed",
        value.calculation_metadata_changed,
    )?;
    Ok(result)
}

pub(crate) fn edit_receipt_v2<'py>(
    py: Python<'py>,
    value: &EditReceiptV2Dto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = edit_receipt(py, &value.receipt)?;
    result.set_item("changed_table_ids", &value.changed_table_ids)?;
    Ok(result)
}

pub(crate) fn calculation_delta<'py>(
    py: Python<'py>,
    value: &CalculationDeltaDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("cursor", value.cursor)?;
    result.set_item("base_revision", value.base_revision)?;
    result.set_item("result_revision", value.result_revision)?;
    result.set_item("mode", &value.mode)?;
    result.set_item("reason", &value.reason)?;
    result.set_item("dirty_count", value.dirty_count)?;
    result.set_item("evaluated_count", value.evaluated_count)?;
    result.set_item("parsed_formula_count", value.parsed_formula_count)?;
    let changed_cells = PyList::empty(py);
    for change in &value.changed_cells {
        let item = PyDict::new(py);
        item.set_item("cell", cell_reference(py, &change.cell)?)?;
        item.set_item("origin", &change.origin)?;
        item.set_item(
            "anchor",
            change
                .anchor
                .as_ref()
                .map(|anchor| cell_reference(py, anchor))
                .transpose()?,
        )?;
        item.set_item("range", change.range.as_deref())?;
        item.set_item("result", calculation_result(py, &change.result)?)?;
        changed_cells.append(item)?;
    }
    result.set_item("changed_cells", changed_cells)?;
    let removed = PyList::empty(py);
    for cell in &value.removed_materialized_cells {
        removed.append(cell_reference(py, cell)?)?;
    }
    result.set_item("removed_materialized_cells", removed)?;
    Ok(result)
}

pub(crate) fn calculation_delta_page<'py>(
    py: Python<'py>,
    value: &CalculationDeltaPageDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("requested_cursor", value.requested_cursor)?;
    result.set_item("next_cursor", value.next_cursor)?;
    let deltas = PyList::empty(py);
    for delta in &value.deltas {
        deltas.append(calculation_delta(py, delta)?)?;
    }
    result.set_item("deltas", deltas)?;
    Ok(result)
}

pub(crate) fn range_page<'py>(
    py: Python<'py>,
    value: &RangePageDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("sheet", &value.sheet)?;
    result.set_item("start", &value.start)?;
    result.set_item("end", &value.end)?;
    result.set_item("total_cells", value.total_cells)?;
    result.set_item("offset", value.offset)?;
    result.set_item("next_offset", value.next_offset)?;
    let cells = PyList::empty(py);
    for cell in &value.cells {
        cells.append(cell_dict(py, cell)?)?;
    }
    result.set_item("cells", cells)?;
    Ok(result)
}

pub(crate) fn defined_name_inspection<'py>(
    py: Python<'py>,
    value: &DefinedNameInspectionDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("result", defined_name_result(py, &value.result)?)?;
    Ok(result)
}

fn defined_name_result<'py>(
    py: Python<'py>,
    value: &DefinedNameInspectionResultDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    match value {
        DefinedNameInspectionResultDto::Rectangular {
            sheet_id,
            sheet_name,
            range,
        } => {
            result.set_item("kind", "rectangular")?;
            result.set_item("sheet_id", sheet_id)?;
            result.set_item("sheet_name", sheet_name)?;
            result.set_item("range", range)?;
        }
        DefinedNameInspectionResultDto::ThreeDimensional { sheet_span, range } => {
            result.set_item("kind", "three_dimensional")?;
            result.set_item("sheet_span", defined_name_sheet_span(py, sheet_span)?)?;
            result.set_item("range", range)?;
        }
        DefinedNameInspectionResultDto::NonRectangular { areas } => {
            result.set_item("kind", "non_rectangular")?;
            let converted = PyList::empty(py);
            for area in areas {
                converted.append(defined_name_area(py, area)?)?;
            }
            result.set_item("areas", converted)?;
        }
        DefinedNameInspectionResultDto::EmptyReference => {
            result.set_item("kind", "empty_reference")?;
        }
        DefinedNameInspectionResultDto::DynamicFormula {
            dynamic_kind,
            formula,
        } => {
            result.set_item("kind", "dynamic_formula")?;
            result.set_item("dynamic_kind", dynamic_kind.as_str())?;
            result.set_item("formula", formula)?;
        }
        DefinedNameInspectionResultDto::Constant { formula } => {
            result.set_item("kind", "constant")?;
            result.set_item("formula", formula)?;
        }
        DefinedNameInspectionResultDto::ExternalReference {
            locator,
            workbook,
            sheet,
            sheet_end,
            target_kind,
            target_text,
        } => {
            result.set_item("kind", "external_reference")?;
            result.set_item("locator", locator.as_deref())?;
            result.set_item("workbook", workbook)?;
            result.set_item("sheet", sheet.as_deref())?;
            result.set_item("sheet_end", sheet_end.as_deref())?;
            result.set_item("target_kind", target_kind.as_str())?;
            result.set_item("target_text", target_text)?;
        }
        DefinedNameInspectionResultDto::Invalid { reason, detail } => {
            result.set_item("kind", "invalid")?;
            result.set_item("reason", reason.as_str())?;
            result.set_item("detail", detail.as_deref())?;
        }
        DefinedNameInspectionResultDto::Unsupported { reason, detail } => {
            result.set_item("kind", "unsupported")?;
            result.set_item("reason", reason.as_str())?;
            result.set_item("detail", detail.as_deref())?;
        }
        DefinedNameInspectionResultDto::NotFound => {
            result.set_item("kind", "not_found")?;
        }
    }
    Ok(result)
}

fn defined_name_area<'py>(
    py: Python<'py>,
    value: &DefinedNameReferenceAreaDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    match value {
        DefinedNameReferenceAreaDto::Rectangular {
            sheet_id,
            sheet_name,
            range,
        } => {
            result.set_item("kind", "rectangular")?;
            result.set_item("sheet_id", sheet_id)?;
            result.set_item("sheet_name", sheet_name)?;
            result.set_item("range", range)?;
        }
        DefinedNameReferenceAreaDto::ThreeDimensional { sheet_span, range } => {
            result.set_item("kind", "three_dimensional")?;
            result.set_item("sheet_span", defined_name_sheet_span(py, sheet_span)?)?;
            result.set_item("range", range)?;
        }
    }
    Ok(result)
}

fn defined_name_sheet_span<'py>(
    py: Python<'py>,
    value: &DefinedNameSheetSpanDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("start_sheet_id", value.start_sheet_id)?;
    result.set_item("start_sheet_name", &value.start_sheet_name)?;
    result.set_item("end_sheet_id", value.end_sheet_id)?;
    result.set_item("end_sheet_name", &value.end_sheet_name)?;
    Ok(result)
}

pub(crate) fn function_usage<'py>(
    py: Python<'py>,
    value: &FunctionUsageReportDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("formula_count", value.formula_count)?;
    result.set_item("parsed_formula_count", value.parsed_formula_count)?;
    result.set_item("unparsed_formula_count", value.unparsed_formula_count)?;
    let entries = PyList::empty(py);
    for entry in &value.entries {
        let item = PyDict::new(py);
        item.set_item("name", &entry.name)?;
        item.set_item("supported", entry.supported)?;
        item.set_item("call_count", entry.call_count)?;
        item.set_item("formula_count", entry.formula_count)?;
        let samples = PyList::empty(py);
        for sample in &entry.sample_cells {
            let cell = PyDict::new(py);
            cell.set_item("sheet_id", sample.sheet_id)?;
            cell.set_item("sheet_name", &sample.sheet_name)?;
            cell.set_item("address", &sample.address)?;
            samples.append(cell)?;
        }
        item.set_item("sample_cells", samples)?;
        entries.append(item)?;
    }
    result.set_item("entries", entries)?;
    Ok(result)
}

pub(crate) fn write_report<'py>(
    py: Python<'py>,
    value: &WriteReportDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("schema_version", value.schema_version)?;
    result.set_item("complete", value.complete)?;
    result.set_item("policy", &value.policy)?;
    result.set_item("materialized_count", value.materialized_count)?;
    let invalidated = PyList::empty(py);
    for cell in &value.invalidated_cells {
        let item = PyDict::new(py);
        item.set_item("sheet_id", cell.sheet_id)?;
        item.set_item("sheet_name", &cell.sheet_name)?;
        item.set_item("address", &cell.address)?;
        invalidated.append(item)?;
    }
    result.set_item("invalidated_cells", invalidated)?;
    result.set_item("changed_parts", &value.changed_parts)?;
    result.set_item("removed_parts", &value.removed_parts)?;
    result.set_item("diagnostic_count", value.diagnostic_count)?;
    Ok(result)
}

fn cell_dict<'py>(py: Python<'py>, cell: &CellDto) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("address", &cell.address)?;
    result.set_item("formula", cell.formula.as_deref())?;
    result.set_item("source_value", cell_value(py, &cell.source_value)?)?;
    result.set_item(
        "source_value_state",
        match cell.source_value_state {
            cellrune_interop::SavedValueStateDto::Literal => "literal",
            cellrune_interop::SavedValueStateDto::Saved => "saved",
            cellrune_interop::SavedValueStateDto::Missing => "missing",
            cellrune_interop::SavedValueStateDto::Invalid => "invalid",
        },
    )?;
    result.set_item(
        "calculated",
        cell.calculated
            .as_ref()
            .map(|calculated| calculation_result(py, calculated))
            .transpose()?,
    )?;
    Ok(result)
}

fn calculation_result<'py>(
    py: Python<'py>,
    value: &CalculationResultDto,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    match value {
        CalculationResultDto::Value { value } => {
            result.set_item("kind", "value")?;
            result.set_item("value", cell_value(py, value)?)?;
        }
        CalculationResultDto::Unavailable {
            code,
            message,
            detail,
        } => {
            result.set_item("kind", "unavailable")?;
            result.set_item("code", code)?;
            result.set_item("message", message)?;
            result.set_item("detail", detail.as_deref())?;
        }
    }
    Ok(result)
}

fn cell_reference<'py>(py: Python<'py>, value: &CellReferenceDto) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("sheet_id", value.sheet_id)?;
    result.set_item("sheet_name", &value.sheet_name)?;
    result.set_item("address", &value.address)?;
    Ok(result)
}

fn cell_value<'py>(py: Python<'py>, value: &CellValueDto) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    match value {
        CellValueDto::Blank => result.set_item("kind", "blank")?,
        CellValueDto::Number { value } => {
            result.set_item("kind", "number")?;
            result.set_item("value", value)?;
        }
        CellValueDto::Text { value } => {
            result.set_item("kind", "text")?;
            result.set_item("value", value)?;
        }
        CellValueDto::Logical { value } => {
            result.set_item("kind", "logical")?;
            result.set_item("value", value)?;
        }
        CellValueDto::Error { value } => {
            result.set_item("kind", "error")?;
            result.set_item("value", value)?;
        }
        CellValueDto::Unsupported => result.set_item("kind", "unsupported")?,
    }
    Ok(result)
}
