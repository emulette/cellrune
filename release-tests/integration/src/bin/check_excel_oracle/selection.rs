use std::collections::{BTreeMap, BTreeSet};

use cellrune::{
    CalculationCellId, CellAddress, CellContent, CellRange, EXCEL_MAX_COLUMNS, FormulaMetadata,
    SheetId, WorkbookSnapshot,
};
use cellrune_integration_tests::oracle::CaseSelection;

const MAX_SELECTED_CASES: usize = 100_000;
const MESSAGE_SELECTION_EMPTY: &str = "case selection must produce at least one result";
const MESSAGE_SELECTION_SHEETS_EMPTY: &str =
    "listed-sheets case selection requires at least one sheet";
const MESSAGE_SELECTION_SHEETS_DUPLICATE: &str =
    "listed-sheets case selection contains a duplicate sheet";
const MESSAGE_SELECTION_UNKNOWN_SHEET: &str = "case selection names an unknown sheet";
const MESSAGE_SELECTION_COLUMN: &str = "case-selection column is outside Excel bounds";
const MESSAGE_SELECTION_LIMIT: &str = "case selection exceeds its result safety limit";
const MESSAGE_SELECTION_RANGE_LIMIT: &str =
    "declared array result range exceeds the case-selection safety limit";
const MESSAGE_SELECTION_ADDRESS: &str = "invalid array result address";

pub(super) fn select_cases(
    workbook: &WorkbookSnapshot,
    selection: &CaseSelection,
) -> Result<BTreeMap<String, CalculationCellId>, String> {
    let listed_sheet_ids = resolve_listed_sheet_ids(workbook, selection)?;
    if let CaseSelection::ListedSheetsColumn { column, .. } = selection
        && !(1..=EXCEL_MAX_COLUMNS).contains(column)
    {
        return Err(format!("{MESSAGE_SELECTION_COLUMN}: {column}"));
    }

    let mut selected = BTreeMap::new();
    for sheet in workbook.sheets() {
        for cell in sheet.cells() {
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let include = match selection {
                CaseSelection::AllFormulaCells | CaseSelection::AllFormulaResults => true,
                CaseSelection::ListedSheetsColumn { column, .. } => {
                    listed_sheet_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(&sheet.id()))
                        && cell.address().column().get() == *column
                }
                CaseSelection::ManifestAddresses => true,
            };
            if include {
                insert_case(
                    &mut selected,
                    sheet.id(),
                    sheet.name().as_str(),
                    cell.address(),
                )?;
            }
            if matches!(selection, CaseSelection::AllFormulaResults) {
                match formula.metadata() {
                    FormulaMetadata::Array { range, .. }
                    | FormulaMetadata::DynamicArray {
                        range: Some(range), ..
                    } => insert_range(&mut selected, sheet.id(), sheet.name().as_str(), *range)?,
                    _ => {}
                }
            }
        }
    }
    if selected.is_empty() {
        Err(MESSAGE_SELECTION_EMPTY.to_owned())
    } else {
        Ok(selected)
    }
}

fn resolve_listed_sheet_ids(
    workbook: &WorkbookSnapshot,
    selection: &CaseSelection,
) -> Result<Option<BTreeSet<SheetId>>, String> {
    let CaseSelection::ListedSheetsColumn { sheets, .. } = selection else {
        return Ok(None);
    };
    if sheets.is_empty() {
        return Err(MESSAGE_SELECTION_SHEETS_EMPTY.to_owned());
    }
    let mut ids = BTreeSet::new();
    for name in sheets {
        let sheet = workbook
            .sheet_by_name(name)
            .ok_or_else(|| format!("{MESSAGE_SELECTION_UNKNOWN_SHEET}: {name}"))?;
        if !ids.insert(sheet.id()) {
            return Err(MESSAGE_SELECTION_SHEETS_DUPLICATE.to_owned());
        }
    }
    Ok(Some(ids))
}

fn insert_range(
    selected: &mut BTreeMap<String, CalculationCellId>,
    sheet_id: SheetId,
    sheet_name: &str,
    range: CellRange,
) -> Result<(), String> {
    let range_cells = u64::from(range.height()) * u64::from(range.width());
    if range_cells > MAX_SELECTED_CASES as u64 {
        return Err(format!(
            "{sheet_name}: {MESSAGE_SELECTION_RANGE_LIMIT}: {range_cells} > {MAX_SELECTED_CASES}"
        ));
    }
    for row in range.start().row().get()..=range.end().row().get() {
        for column in range.start().column().get()..=range.end().column().get() {
            let address = CellAddress::from_indices(row, column)
                .map_err(|error| format!("{sheet_name}: {MESSAGE_SELECTION_ADDRESS}: {error:?}"))?;
            insert_case(selected, sheet_id, sheet_name, address)?;
        }
    }
    Ok(())
}

fn insert_case(
    selected: &mut BTreeMap<String, CalculationCellId>,
    sheet_id: SheetId,
    sheet_name: &str,
    address: CellAddress,
) -> Result<(), String> {
    let key = format!("{sheet_name}!{address}");
    if selected.len() >= MAX_SELECTED_CASES && !selected.contains_key(&key) {
        return Err(format!("{MESSAGE_SELECTION_LIMIT}: {MAX_SELECTED_CASES}"));
    }
    selected.insert(key, CalculationCellId::new(sheet_id, address));
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{MESSAGE_SELECTION_RANGE_LIMIT, insert_range, select_cases};
    use cellrune::{
        CalculationHints, CellAddress, CellContent, CellRange, DateSystem, FormulaCell,
        FormulaDialect, FormulaMetadata, FormulaText, Provenance, ProviderIdentity, SavedResult,
        Sheet, SheetId, SheetName, SheetVisibility, WorkbookSnapshot, WorkbookSource,
    };
    use cellrune_integration_tests::oracle::CaseSelection;

    fn workbook_with_formula() -> WorkbookSnapshot {
        let mut sheet = Sheet::new(
            SheetId::new(1).expect("sheet ID"),
            SheetName::new("Arithmetic").expect("sheet name"),
            SheetVisibility::Visible,
        );
        sheet
            .insert_cell(
                CellAddress::from_a1("F1").expect("cell address"),
                CellContent::Formula(FormulaCell::new(
                    FormulaDialect::ExcelA1,
                    FormulaText::from_xlsx("1+1").expect("formula"),
                    SavedResult::Missing,
                    FormulaMetadata::Normal,
                )),
            )
            .expect("unique formula");
        WorkbookSnapshot::new(
            vec![sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("oracle-selection-test", "1").expect("provider"),
                None,
            ),
        )
        .expect("workbook")
    }

    #[test]
    fn listed_sheet_selection_uses_case_insensitive_resolution() {
        let selected = select_cases(
            &workbook_with_formula(),
            &CaseSelection::ListedSheetsColumn {
                sheets: vec!["arithmetic".to_owned()],
                column: 6,
            },
        )
        .expect("case-insensitive sheet selection");
        assert!(selected.contains_key("Arithmetic!F1"));
    }

    #[test]
    fn listed_sheet_selection_rejects_case_variant_duplicates() {
        let error = select_cases(
            &workbook_with_formula(),
            &CaseSelection::ListedSheetsColumn {
                sheets: vec!["Arithmetic".to_owned(), "arithmetic".to_owned()],
                column: 6,
            },
        )
        .expect_err("same sheet listed twice");
        assert!(error.contains("duplicate sheet"));
    }

    #[test]
    fn declared_array_range_is_bounded_before_expansion() {
        let mut selected = BTreeMap::new();
        let range = CellRange::new(
            CellAddress::from_a1("A1").expect("range start"),
            CellAddress::from_a1("XFD1048576").expect("range end"),
        )
        .expect("ordered Excel range");
        let error = insert_range(
            &mut selected,
            SheetId::new(1).expect("sheet ID"),
            "Sheet1",
            range,
        )
        .expect_err("oversized range");
        assert!(error.contains(MESSAGE_SELECTION_RANGE_LIMIT));
        assert!(selected.is_empty());
    }
}
