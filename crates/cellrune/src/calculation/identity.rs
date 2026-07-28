use sha2::{Digest, Sha256};

use crate::{
    CalculationHints, CalculationMode, CellContent, CellValue, DateSystem, DefinedNameScope,
    FormulaMetadata, NumberFormatKind, SharedFormulaRole, SheetVisibility, WorkbookSnapshot,
};

pub(crate) fn workbook_fingerprint(workbook: &WorkbookSnapshot) -> [u8; 32] {
    let mut hash = SemanticHash::new();
    // Schema byte 2: 0.1.6 folds merged ranges and the full table model per sheet.
    // Bumping it deliberately invalidates every fingerprint persisted under schema 1.
    hash.u8(2);
    hash.date_system(workbook.date_system());
    hash.calculation_hints(workbook.calculation_hints());
    hash.usize(workbook.sheets().len());
    for sheet in workbook.sheets() {
        hash.u32(sheet.id().get());
        hash.string(sheet.name().as_str());
        hash.sheet_visibility(sheet.visibility());
        hash.usize(sheet.len());
        for cell in sheet.cells() {
            hash.u32(cell.address().row().get());
            hash.u32(cell.address().column().get());
            hash.number_format(cell.number_format());
            match cell.content() {
                CellContent::Literal(value) => {
                    hash.u8(0);
                    hash.cell_value(value);
                }
                CellContent::Formula(formula) => {
                    hash.u8(1);
                    hash.optional_string(formula.text().map(|text| text.as_str()));
                    hash.formula_metadata(formula.metadata());
                    hash.boolean(formula.recalculate_always());
                }
            }
        }
        hash.usize(sheet.merged_ranges().len());
        for range in sheet.merged_ranges() {
            hash.range(*range);
        }
        // The whole table model is folded, including fields such as display_name that do
        // not feed calculation today: missing a fold shows up as a stale write, folding
        // too much only costs one extra recalculation.
        hash.usize(sheet.tables().len());
        for table in sheet.tables() {
            hash.string(table.name().as_str());
            hash.string(table.display_name());
            hash.range(table.range());
            hash.u32(table.header_row_count());
            hash.u32(table.totals_row_count());
            hash.usize(table.columns().len());
            for column in table.columns() {
                hash.u32(column.id());
                hash.string(column.name());
                match column.totals_row_function() {
                    None => hash.u8(0),
                    Some(function) => {
                        hash.u8(1);
                        hash.string(function.as_str());
                    }
                }
            }
        }
    }
    hash.usize(workbook.defined_names().len());
    for name in workbook.defined_names() {
        hash.string(name.name());
        match name.scope() {
            DefinedNameScope::Workbook => hash.u8(0),
            DefinedNameScope::Sheet(sheet_id) => {
                hash.u8(1);
                hash.u32(sheet_id.get());
            }
        }
        hash.string(name.formula().as_str());
        hash.boolean(name.hidden());
    }
    hash.finish()
}

struct SemanticHash(Sha256);

impl SemanticHash {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn finish(self) -> [u8; 32] {
        self.0.finalize().into()
    }

    fn bytes(&mut self, value: &[u8]) {
        self.u64(value.len() as u64);
        self.0.update(value);
    }

    fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn optional_string(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.string(value);
            }
            None => self.u8(0),
        }
    }

    fn boolean(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn usize(&mut self, value: usize) {
        self.u64(value as u64);
    }

    fn u8(&mut self, value: u8) {
        self.0.update([value]);
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.update(value.to_le_bytes());
    }

    fn date_system(&mut self, value: DateSystem) {
        self.u8(match value {
            DateSystem::Excel1900 => 0,
            DateSystem::Excel1904 => 1,
        });
    }

    fn calculation_hints(&mut self, value: CalculationHints) {
        self.u8(match value.mode() {
            None => 0,
            Some(CalculationMode::Automatic) => 1,
            Some(CalculationMode::AutomaticExceptDataTables) => 2,
            Some(CalculationMode::Manual) => 3,
        });
        self.optional_u32(value.calculation_id());
        self.optional_bool(value.full_calculation_on_load());
        self.optional_bool(value.force_full_calculation());
        self.optional_bool(value.iterative_calculation());
    }

    fn optional_u32(&mut self, value: Option<u32>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.u32(value);
            }
            None => self.u8(0),
        }
    }

    fn optional_bool(&mut self, value: Option<bool>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.boolean(value);
            }
            None => self.u8(0),
        }
    }

    fn sheet_visibility(&mut self, value: SheetVisibility) {
        self.u8(match value {
            SheetVisibility::Visible => 0,
            SheetVisibility::Hidden => 1,
            SheetVisibility::VeryHidden => 2,
        });
    }

    fn number_format(&mut self, value: &crate::NumberFormat) {
        self.u32(value.id());
        self.optional_string(value.code());
        self.u8(match value.kind() {
            NumberFormatKind::General => 0,
            NumberFormatKind::Number => 1,
            NumberFormatKind::Date => 2,
            NumberFormatKind::Time => 3,
            NumberFormatKind::DateTime => 4,
            NumberFormatKind::Duration => 5,
        });
    }

    fn cell_value(&mut self, value: &CellValue) {
        match value {
            CellValue::Blank => self.u8(0),
            CellValue::Number(number) => {
                self.u8(1);
                self.u64(number.get().to_bits());
            }
            CellValue::Text(text) => {
                self.u8(2);
                self.string(text);
            }
            CellValue::Logical(value) => {
                self.u8(3);
                self.boolean(*value);
            }
            CellValue::Error(error) => {
                self.u8(4);
                self.string(error.as_str());
            }
        }
    }

    fn formula_metadata(&mut self, value: &FormulaMetadata) {
        match value {
            FormulaMetadata::Normal => self.u8(0),
            FormulaMetadata::Shared {
                group_index,
                role,
                range,
            } => {
                self.u8(1);
                self.u32(*group_index);
                match role {
                    SharedFormulaRole::Anchor => self.u8(0),
                    SharedFormulaRole::Follower { anchor } => {
                        self.u8(1);
                        self.address(*anchor);
                    }
                }
                self.optional_range(*range);
            }
            FormulaMetadata::Array {
                range,
                always_calculate,
            } => {
                self.u8(2);
                self.range(*range);
                self.boolean(*always_calculate);
            }
            FormulaMetadata::DynamicArray {
                range,
                always_calculate,
            } => {
                self.u8(3);
                self.optional_range(*range);
                self.boolean(*always_calculate);
            }
            FormulaMetadata::DataTable {
                range,
                input_cell_1,
                input_cell_2,
                two_dimensional,
                row_oriented,
                input_cell_1_deleted,
                input_cell_2_deleted,
            } => {
                self.u8(4);
                self.range(*range);
                self.optional_address(*input_cell_1);
                self.optional_address(*input_cell_2);
                self.boolean(*two_dimensional);
                self.boolean(*row_oriented);
                self.boolean(*input_cell_1_deleted);
                self.boolean(*input_cell_2_deleted);
            }
        }
    }

    fn optional_address(&mut self, value: Option<crate::CellAddress>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.address(value);
            }
            None => self.u8(0),
        }
    }

    fn optional_range(&mut self, value: Option<crate::CellRange>) {
        match value {
            Some(value) => {
                self.u8(1);
                self.range(value);
            }
            None => self.u8(0),
        }
    }

    fn range(&mut self, value: crate::CellRange) {
        self.address(value.start());
        self.address(value.end());
    }

    fn address(&mut self, value: crate::CellAddress) {
        self.u32(value.row().get());
        self.u32(value.column().get());
    }
}

#[cfg(test)]
mod tests {
    use super::workbook_fingerprint;
    use crate::{
        CalculationHints, CellAddress, CellContent, CellRange, CellValue, DateSystem, Provenance,
        ProviderIdentity, Sheet, SheetId, SheetName, SheetVisibility, Table, TableColumn,
        TableName, TotalsRowFunction, WorkbookSnapshot, WorkbookSource,
    };

    #[test]
    fn workbook_fingerprint_is_stable_and_changes_with_cell_semantics() {
        let first = workbook_with_number(1.0);
        let same = workbook_with_number(1.0);
        let changed = workbook_with_number(2.0);

        assert_eq!(workbook_fingerprint(&first), workbook_fingerprint(&same));
        assert_ne!(workbook_fingerprint(&first), workbook_fingerprint(&changed));
    }

    #[test]
    fn workbook_fingerprint_folds_merged_ranges() {
        let base = workbook_with_extras(Vec::new(), Vec::new());
        let merged = workbook_with_extras(vec![range("A1", "B2")], Vec::new());
        let moved = workbook_with_extras(vec![range("A1", "B3")], Vec::new());

        assert_ne!(workbook_fingerprint(&base), workbook_fingerprint(&merged));
        assert_ne!(workbook_fingerprint(&merged), workbook_fingerprint(&moved));
        assert_eq!(
            workbook_fingerprint(&merged),
            workbook_fingerprint(&workbook_with_extras(vec![range("A1", "B2")], Vec::new()))
        );
    }

    type TableDefinition<'a> = (
        &'a str,
        &'a str,
        CellRange,
        u32,
        u32,
        Vec<(&'a str, u32, Option<TotalsRowFunction>)>,
    );

    #[test]
    fn workbook_fingerprint_folds_every_table_field() {
        let base = || -> TableDefinition<'static> {
            (
                "Sales",
                "SalesDisplay",
                range("A1", "B4"),
                1_u32,
                0_u32,
                vec![("First", 1_u32, None), ("Second", 2, None)],
            )
        };
        let build = |definition: TableDefinition<'_>| {
            let (name, display_name, reference, header, totals, columns) = definition;
            let columns = columns
                .into_iter()
                .map(|(name, id, function)| {
                    TableColumn::new(id, name, function).expect("column")
                })
                .collect();
            workbook_with_extras(
                Vec::new(),
                vec![
                    Table::new(
                        TableName::new(name).expect("name"),
                        display_name,
                        reference,
                        header,
                        totals,
                        columns,
                    )
                    .expect("table"),
                ],
            )
        };
        let reference = workbook_fingerprint(&build(base()));

        let mut changed = base();
        changed.2 = range("A1", "B5");
        assert_ne!(reference, workbook_fingerprint(&build(changed)), "@ref");

        let mut changed = base();
        changed.5[0].0 = "Renamed";
        assert_ne!(reference, workbook_fingerprint(&build(changed)), "column name");

        let mut changed = base();
        changed.5.swap(0, 1);
        changed.5[0].1 = 1;
        changed.5[1].1 = 2;
        assert_ne!(reference, workbook_fingerprint(&build(changed)), "column order");

        let mut changed = base();
        changed.5[1].2 = Some(TotalsRowFunction::Sum);
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "totalsRowFunction"
        );

        let mut changed = base();
        changed.3 = 0;
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "header row count"
        );

        let mut changed = base();
        changed.1 = "OtherDisplay";
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "display name is folded too - the whole model is folded"
        );

        assert_eq!(reference, workbook_fingerprint(&build(base())), "stable");
    }

    fn range(start: &str, end: &str) -> CellRange {
        CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range")
    }

    fn workbook_with_extras(
        merged_ranges: Vec<CellRange>,
        tables: Vec<Table>,
    ) -> WorkbookSnapshot {
        let sheet_id = SheetId::new(1).expect("valid sheet ID");
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new("Sheet1").expect("valid sheet name"),
            SheetVisibility::Visible,
        );
        sheet.set_merged_ranges(merged_ranges);
        sheet.set_tables(tables);
        WorkbookSnapshot::new(
            vec![sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("identity-test", "1").expect("valid provider"),
                None,
            ),
        )
        .expect("valid workbook")
    }

    fn workbook_with_number(value: f64) -> WorkbookSnapshot {
        let sheet_id = SheetId::new(1).expect("valid sheet ID");
        let mut sheet = Sheet::new(
            sheet_id,
            SheetName::new("Sheet1").expect("valid sheet name"),
            SheetVisibility::Visible,
        );
        sheet
            .insert_cell(
                CellAddress::from_a1("A1").expect("valid cell address"),
                CellContent::Literal(CellValue::number(value).expect("finite number")),
            )
            .expect("unique cell");
        WorkbookSnapshot::new(
            vec![sheet],
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(
                ProviderIdentity::new("identity-test", "1").expect("valid provider"),
                None,
            ),
        )
        .expect("valid workbook")
    }
}
