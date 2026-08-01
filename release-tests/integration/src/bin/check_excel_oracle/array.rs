use std::collections::{BTreeMap, BTreeSet};

use cellrune::{
    CalculationCellId, CellAddress, CellRange, MaterializedResultOrigin, WorkbookSnapshot,
};
use cellrune_integration_tests::oracle::{
    CacheStatus, CellRuneArrayResult, CellRuneArrayResultCell, ObservedCase, ObservedValue,
    values_match,
};

use super::{LoadedOracle, observed_result};

pub(super) fn audit_observed_result(
    context: &str,
    loaded: &LoadedOracle,
    anchor_id: CalculationCellId,
    observation: &ObservedCase,
    compare_calculation: bool,
    problems: &mut Vec<String>,
) -> Option<ArrayResultAudit> {
    let result = observation.result.as_ref()?;
    let Some((sheet_name, range_text)) = result.range.rsplit_once('!') else {
        problems.push(format!("{context}: array result has an invalid range"));
        return None;
    };
    let (start_text, end_text) = range_text
        .split_once(':')
        .map_or((range_text, range_text), |(start, end)| (start, end));
    let Ok(start) = CellAddress::from_a1(start_text) else {
        problems.push(format!(
            "{context}: array result has an invalid start address"
        ));
        return None;
    };
    let Ok(end) = CellAddress::from_a1(end_text) else {
        problems.push(format!(
            "{context}: array result has an invalid end address"
        ));
        return None;
    };
    let Ok(range) = CellRange::new(start, end) else {
        problems.push(format!("{context}: array result range is not ordered"));
        return None;
    };
    let Some(sheet) = loaded.workbook.sheet_by_name(sheet_name) else {
        problems.push(format!("{context}: array result names an unknown sheet"));
        return None;
    };
    let expected_cells = u64::from(range.height()) * u64::from(range.width());
    if result.rows != range.height()
        || result.columns != range.width()
        || u64::try_from(result.cells.len()).ok() != Some(expected_cells)
    {
        problems.push(format!(
            "{context}: array result shape does not match its range"
        ));
        return None;
    }
    let anchor = CalculationCellId::new(sheet.id(), start);
    if anchor != anchor_id {
        problems.push(format!(
            "{context}: array result range does not start at its formula anchor"
        ));
    }
    let mut addresses = BTreeSet::new();
    for cell in &result.cells {
        if !addresses.insert(cell.address.as_str()) {
            problems.push(format!("{context}: array result contains a duplicate cell"));
            continue;
        }
        let Some((cell_sheet, cell_address)) = cell.address.rsplit_once('!') else {
            problems.push(format!(
                "{context}: array result cell has an invalid address"
            ));
            continue;
        };
        if cell_sheet != sheet_name {
            problems.push(format!(
                "{context}: array result cell escapes its result sheet"
            ));
            continue;
        }
        let Ok(address) = CellAddress::from_a1(cell_address) else {
            problems.push(format!(
                "{context}: array result cell has an invalid address"
            ));
            continue;
        };
        if !range.contains(address) {
            problems.push(format!(
                "{context}: array result cell escapes its declared range"
            ));
            continue;
        }
    }
    if !compare_calculation {
        return None;
    }
    let calculated = match calculated_array_result(&loaded.workbook, &loaded.calculation, anchor_id)
    {
        Ok(calculated) => calculated,
        Err(error) => {
            problems.push(format!("{context}: {error}"));
            return None;
        }
    };
    let (mismatch_count, signature_mismatch_count) =
        observed_array_result_comparison(observation, &calculated);
    Some(ArrayResultAudit {
        calculated,
        mismatch_count,
        signature_mismatch_count,
    })
}

pub(super) struct ArrayResultAudit {
    pub(super) calculated: CellRuneArrayResult,
    pub(super) mismatch_count: usize,
    pub(super) signature_mismatch_count: usize,
}

pub(super) fn calculated_array_result(
    workbook: &WorkbookSnapshot,
    calculation: &cellrune::CalculationSnapshot,
    anchor_id: CalculationCellId,
) -> Result<CellRuneArrayResult, String> {
    let Some(anchor_cell) = calculation.materialized_cell(anchor_id) else {
        return Ok(CellRuneArrayResult::Missing);
    };
    let (owner, range, array_origin) = match anchor_cell.origin() {
        MaterializedResultOrigin::LegacyArray { anchor, range }
        | MaterializedResultOrigin::DynamicSpill { anchor, range } => (anchor, range, true),
        MaterializedResultOrigin::DirectFormula => {
            let range = CellRange::new(anchor_id.address(), anchor_id.address())
                .expect("one-cell calculation range is valid");
            (anchor_id, range, false)
        }
        _ => return Ok(CellRuneArrayResult::Missing),
    };
    if owner != anchor_id || range.start() != anchor_id.address() {
        return Err("calculated array result is not owned by its formula anchor".to_owned());
    }
    let sheet = workbook
        .sheet_by_id(anchor_id.sheet_id())
        .ok_or_else(|| "calculated array result names an unknown sheet".to_owned())?;
    let range_text = if range.start() == range.end() {
        format!("{}!{}", sheet.name().as_str(), range.start())
    } else {
        format!(
            "{}!{}:{}",
            sheet.name().as_str(),
            range.start(),
            range.end()
        )
    };
    let capacity = usize::try_from(u64::from(range.height()) * u64::from(range.width()))
        .map_err(|_| "calculated array result shape is too large".to_owned())?;
    let mut cells = Vec::with_capacity(capacity);
    for row in range.start().row().get()..=range.end().row().get() {
        for column in range.start().column().get()..=range.end().column().get() {
            let address = CellAddress::from_indices(row, column)
                .map_err(|error| format!("calculated array result address is invalid: {error}"))?;
            let id = CalculationCellId::new(anchor_id.sheet_id(), address);
            let materialized = calculation.materialized_cell(id).ok_or_else(|| {
                format!("calculated array result is missing materialized cell {address}")
            })?;
            let same_owner_and_range = if array_origin {
                matches!(
                    materialized.origin(),
                    MaterializedResultOrigin::LegacyArray { anchor, range: cell_range }
                        | MaterializedResultOrigin::DynamicSpill { anchor, range: cell_range }
                        if anchor == anchor_id && cell_range == range
                )
            } else {
                id == anchor_id
                    && matches!(
                        materialized.origin(),
                        MaterializedResultOrigin::DirectFormula
                    )
            };
            if !same_owner_and_range {
                return Err(format!(
                    "calculated array result cell {address} has an inconsistent owner or range"
                ));
            }
            let value = observed_result(Some(materialized.result()))?
                .ok_or_else(|| format!("calculated array result cell {address} is unavailable"))?;
            cells.push(CellRuneArrayResultCell {
                address: format!("{}!{address}", sheet.name().as_str()),
                value: value.value,
                value_type: value.value_type,
            });
        }
    }
    Ok(CellRuneArrayResult::Materialized {
        range: range_text,
        rows: range.height(),
        columns: range.width(),
        cells,
    })
}

pub(super) fn array_result_comparison(
    observed: &cellrune_integration_tests::oracle::ObservedResult,
    observed_cells: &BTreeMap<&str, Option<ObservedValue>>,
    anchor_address: &str,
    calculated: &CellRuneArrayResult,
) -> (usize, usize) {
    let CellRuneArrayResult::Materialized {
        range,
        rows,
        columns,
        cells,
    } = calculated
    else {
        let mismatches = observed.cells.len().max(1);
        return (mismatches, mismatches);
    };
    let shape_mismatch = usize::from(
        range != &observed.range || rows != &observed.rows || columns != &observed.columns,
    );
    let mut mismatches = shape_mismatch;
    let mut signature_mismatches = shape_mismatch;
    let calculated_cells = cells
        .iter()
        .map(|cell| (cell.address.as_str(), cell))
        .collect::<BTreeMap<_, _>>();
    for (address, expected) in observed_cells {
        let actual = calculated_cells.get(address).map(|cell| ObservedValue {
            value: cell.value.clone(),
            value_type: cell.value_type.clone(),
        });
        let matches = match (actual.as_ref(), expected.as_ref()) {
            (Some(actual), Some(expected)) => values_match(actual, expected, None).unwrap_or(false),
            (None, None) => true,
            _ => false,
        };
        if !matches {
            mismatches += 1;
            if *address != anchor_address {
                signature_mismatches += 1;
            }
        }
    }
    (mismatches, signature_mismatches)
}

pub(super) fn observed_array_result_comparison(
    observation: &ObservedCase,
    calculated: &CellRuneArrayResult,
) -> (usize, usize) {
    let observed = observation
        .result
        .as_ref()
        .expect("caller checked the observed array result");
    let observed_cells = observed
        .cells
        .iter()
        .map(|cell| (cell.address.as_str(), observed_result_cell_value(cell)))
        .collect::<BTreeMap<_, _>>();
    array_result_comparison(
        observed,
        &observed_cells,
        observation.address.as_str(),
        calculated,
    )
}

fn observed_result_cell_value(
    cell: &cellrune_integration_tests::oracle::ObservedResultCell,
) -> Option<ObservedValue> {
    if cell.cache_status != CacheStatus::Semantic {
        return None;
    }
    let value = cell
        .rich_error
        .resolved_error
        .as_ref()
        .or(cell.rich_error.fallback_error.as_ref())
        .or(cell.cache_value.as_ref())?
        .clone();
    let value_type = if cell.rich_error.present {
        "e"
    } else {
        &cell.cache_type
    };
    Some(ObservedValue {
        value,
        value_type: value_type.to_owned(),
    })
}
