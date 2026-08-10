use sha2::{Digest, Sha256};

use crate::{
    CalculationHints, CalculationMode, Cell, CellContent, CellValue, DateSystem, DefinedNameScope,
    FormulaMetadata, NumberFormatKind, SharedFormulaRole, SheetVisibility, WorkbookSnapshot,
};

pub(crate) fn workbook_fingerprint(workbook: &WorkbookSnapshot) -> [u8; 32] {
    workbook_fingerprint_cancellable(workbook, &|| false)
        .expect("non-cancellable fingerprinting cannot be cancelled")
}

pub(crate) fn workbook_fingerprint_cancellable(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<[u8; 32], ()> {
    let mut hash = SemanticHash::new();
    // Schema byte 4: cell payloads are folded through immutable row-chunk digests. The Merkle-like
    // layout lets an edited workbook reuse every unchanged chunk while retaining exact identity.
    hash.u8(4);
    hash.date_system(workbook.date_system());
    hash.calculation_hints(workbook.calculation_hints());
    hash.usize(workbook.sheets().len());
    for sheet in workbook.sheets() {
        if cancelled() {
            return Err(());
        }
        hash.u32(sheet.id().get());
        hash.string(sheet.name().as_str());
        hash.sheet_visibility(sheet.visibility());
        hash.usize(sheet.len());
        let cell_chunks = sheet.semantic_cell_chunk_fingerprints_cancellable(cancelled)?;
        hash.usize(cell_chunks.len());
        for chunk in cell_chunks {
            if cancelled() {
                return Err(());
            }
            hash.bytes(&chunk);
        }
        hash.usize(sheet.merged_ranges().len());
        for range in sheet.merged_ranges() {
            if cancelled() {
                return Err(());
            }
            hash.range(*range);
        }
        // The whole table model is folded, including fields such as display_name that do
        // not feed calculation today: missing a fold shows up as a stale write, folding
        // too much only costs one extra recalculation.
        hash.usize(sheet.tables().len());
        for table in sheet.tables() {
            if cancelled() {
                return Err(());
            }
            hash.u32(table.id().get());
            hash.string_cancellable(table.name().as_str(), cancelled)?;
            hash.string_cancellable(table.display_name().as_str(), cancelled)?;
            hash.range(table.range());
            hash.string_cancellable(table.table_type().as_str(), cancelled)?;
            hash.u32(table.header_row_count());
            hash.u32(table.totals_row_count());
            hash.boolean(table.totals_row_shown());
            hash.usize(table.columns().len());
            for column in table.columns() {
                if cancelled() {
                    return Err(());
                }
                hash.u32(column.id());
                hash.string_cancellable(column.name(), cancelled)?;
                match column.totals_row_function() {
                    None => hash.u8(0),
                    Some(function) => {
                        hash.u8(1);
                        hash.string_cancellable(function.as_str(), cancelled)?;
                    }
                }
                hash.optional_string_cancellable(column.totals_row_label(), cancelled)?;
                hash.optional_table_formula(column.calculated_column_formula(), cancelled)?;
                hash.optional_table_formula(column.totals_row_formula(), cancelled)?;
            }
            hash.optional_table_auto_filter(table.auto_filter(), cancelled)?;
            hash.optional_table_sort_state(table.sort_state(), cancelled)?;
            match table.style_info() {
                Some(style) => {
                    hash.u8(1);
                    hash.optional_string_cancellable(style.name(), cancelled)?;
                    hash.boolean(style.show_first_column());
                    hash.boolean(style.show_last_column());
                    hash.boolean(style.show_row_stripes());
                    hash.boolean(style.show_column_stripes());
                }
                None => hash.u8(0),
            }
            hash.optional_bytes_cancellable(table.opaque_source_xml(), cancelled)?;
        }
    }
    hash.usize(workbook.defined_names().len());
    for name in workbook.defined_names() {
        if cancelled() {
            return Err(());
        }
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
    Ok(hash.finish())
}

pub(crate) fn cell_chunk_fingerprint_cancellable<'a>(
    cells: impl Iterator<Item = &'a Cell>,
    len: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<[u8; 32], ()> {
    let mut hash = SemanticHash::new();
    hash.u8(1);
    hash.usize(len);
    for cell in cells {
        if cancelled() {
            return Err(());
        }
        hash.cell(cell);
    }
    Ok(hash.finish())
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

    fn cell(&mut self, cell: &Cell) {
        self.u32(cell.address().row().get());
        self.u32(cell.address().column().get());
        self.number_format(cell.number_format());
        match cell.content() {
            CellContent::Literal(value) => {
                self.u8(0);
                self.cell_value(value);
            }
            CellContent::Formula(formula) => {
                self.u8(1);
                self.optional_string(formula.text().map(|text| text.as_str()));
                self.formula_metadata(formula.metadata());
                self.boolean(formula.recalculate_always());
            }
        }
    }

    fn optional_table_formula(
        &mut self,
        value: Option<&crate::TableFormula>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        match value {
            Some(value) => {
                self.u8(1);
                self.string_cancellable(value.text().as_str(), cancelled)?;
                self.boolean(value.is_array());
            }
            None => self.u8(0),
        }
        Ok(())
    }

    fn optional_table_auto_filter(
        &mut self,
        value: Option<&crate::TableAutoFilter>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        match value {
            Some(value) => {
                self.u8(1);
                self.range(value.range());
                self.boolean(value.declared_range().is_some());
                self.usize(value.filter_columns().len());
                for column in value.filter_columns() {
                    if cancelled() {
                        return Err(());
                    }
                    self.u32(column.column_id());
                    self.boolean(column.hidden_button());
                    self.boolean(column.show_button());
                    self.optional_table_filter_criteria(column.criteria(), cancelled)?;
                }
                self.optional_table_sort_state(value.sort_state(), cancelled)?;
            }
            None => self.u8(0),
        }
        Ok(())
    }

    fn optional_table_filter_criteria(
        &mut self,
        value: Option<&crate::TableFilterCriteria>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        let Some(value) = value else {
            self.u8(0);
            return Ok(());
        };
        self.u8(1);
        match value {
            crate::TableFilterCriteria::Values(filters) => {
                self.u8(0);
                self.boolean(filters.blank());
                self.optional_string_cancellable(
                    filters
                        .calendar_type()
                        .map(crate::TableCalendarType::as_str),
                    cancelled,
                )?;
                self.usize(filters.items().len());
                for item in filters.items() {
                    if cancelled() {
                        return Err(());
                    }
                    match item {
                        crate::TableFilterItem::Value(value) => {
                            self.u8(0);
                            self.optional_string_cancellable(value.as_deref(), cancelled)?;
                        }
                        crate::TableFilterItem::DateGroup(item) => {
                            self.u8(1);
                            self.u32(u32::from(item.year()));
                            self.optional_u16(item.month());
                            self.optional_u16(item.day());
                            self.optional_u16(item.hour());
                            self.optional_u16(item.minute());
                            self.optional_u16(item.second());
                            self.string_cancellable(item.grouping().as_str(), cancelled)?;
                        }
                    }
                }
            }
            crate::TableFilterCriteria::Custom(filters) => {
                self.u8(1);
                self.boolean(filters.and());
                self.usize(filters.filters().len());
                for filter in filters.filters() {
                    if cancelled() {
                        return Err(());
                    }
                    self.optional_string_cancellable(
                        filter
                            .operator()
                            .map(crate::TableCustomFilterOperator::as_str),
                        cancelled,
                    )?;
                    self.optional_string_cancellable(filter.value(), cancelled)?;
                }
            }
            crate::TableFilterCriteria::Dynamic(filter) => {
                self.u8(2);
                self.string_cancellable(filter.kind().as_str(), cancelled)?;
                self.optional_string_cancellable(
                    filter.value().map(crate::TableNumericValue::as_str),
                    cancelled,
                )?;
                self.optional_string_cancellable(
                    filter.iso_value().map(crate::TableDateTimeValue::as_str),
                    cancelled,
                )?;
                self.optional_string_cancellable(
                    filter.max_value().map(crate::TableNumericValue::as_str),
                    cancelled,
                )?;
                self.optional_string_cancellable(
                    filter
                        .max_iso_value()
                        .map(crate::TableDateTimeValue::as_str),
                    cancelled,
                )?;
            }
            crate::TableFilterCriteria::Color(filter) => {
                self.u8(3);
                self.optional_u32(filter.differential_format_id());
                self.boolean(filter.cell_color());
            }
            crate::TableFilterCriteria::Icon(filter) => {
                self.u8(4);
                self.string_cancellable(filter.icon_set().as_str(), cancelled)?;
                self.optional_u32(filter.icon_id());
            }
            crate::TableFilterCriteria::Top(filter) => {
                self.u8(5);
                self.boolean(filter.top());
                self.boolean(filter.percent());
                self.string_cancellable(filter.value().as_str(), cancelled)?;
                self.optional_string_cancellable(
                    filter.filter_value().map(crate::TableNumericValue::as_str),
                    cancelled,
                )?;
            }
        }
        Ok(())
    }

    fn optional_table_sort_state(
        &mut self,
        value: Option<&crate::TableSortState>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        match value {
            Some(value) => {
                self.u8(1);
                self.range(value.range());
                self.boolean(value.case_sensitive());
                self.boolean(value.column_sort());
                self.optional_string_cancellable(
                    value.sort_method().map(crate::TableSortMethod::as_str),
                    cancelled,
                )?;
                self.usize(value.conditions().len());
                for condition in value.conditions() {
                    if cancelled() {
                        return Err(());
                    }
                    self.range(condition.range());
                    self.boolean(condition.descending());
                    self.optional_string_cancellable(
                        condition.sort_by().map(crate::TableSortBy::as_str),
                        cancelled,
                    )?;
                    self.optional_string_cancellable(condition.custom_list(), cancelled)?;
                    self.optional_u32(condition.differential_format_id());
                    self.optional_string_cancellable(
                        condition.icon_set().map(crate::TableIconSet::as_str),
                        cancelled,
                    )?;
                    self.optional_u32(condition.icon_id());
                }
            }
            None => self.u8(0),
        }
        Ok(())
    }

    fn optional_bytes_cancellable(
        &mut self,
        value: Option<&[u8]>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        match value {
            Some(value) => {
                self.u8(1);
                self.bytes_cancellable(value, cancelled)?;
            }
            None => self.u8(0),
        }
        Ok(())
    }

    fn optional_string_cancellable(
        &mut self,
        value: Option<&str>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        match value {
            Some(value) => {
                self.u8(1);
                self.string_cancellable(value, cancelled)?;
            }
            None => self.u8(0),
        }
        Ok(())
    }

    fn string_cancellable(&mut self, value: &str, cancelled: &impl Fn() -> bool) -> Result<(), ()> {
        self.bytes_cancellable(value.as_bytes(), cancelled)
    }

    fn bytes_cancellable(&mut self, value: &[u8], cancelled: &impl Fn() -> bool) -> Result<(), ()> {
        self.u64(value.len() as u64);
        for chunk in value.chunks(64 * 1024) {
            if cancelled() {
                return Err(());
            }
            self.0.update(chunk);
        }
        Ok(())
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

    fn optional_u16(&mut self, value: Option<u16>) {
        self.optional_u32(value.map(u32::from));
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
    use std::cell::Cell;

    use super::{workbook_fingerprint, workbook_fingerprint_cancellable};
    use crate::{
        CalculationHints, CellAddress, CellContent, CellRange, CellValue, DateSystem, FormulaText,
        Provenance, ProviderIdentity, Sheet, SheetId, SheetName, SheetVisibility, Table,
        TableAutoFilter, TableColumn, TableFilterColumn, TableFilterCriteria, TableFilterItem,
        TableFormula, TableId, TableName, TableSortCondition, TableSortMethod, TableSortState,
        TableStyleInfo, TableType, TableValueFilters, TotalsRowFunction, WorkbookSnapshot,
        WorkbookSource,
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
    fn workbook_fingerprint_polls_cancellation_between_sparse_cells() {
        let workbook = workbook_with_number(1.0);
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 2
        };

        assert_eq!(
            workbook_fingerprint_cancellable(&workbook, &cancelled),
            Err(())
        );
        assert_eq!(polls.get(), 2);
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

    #[test]
    fn workbook_fingerprint_polls_cancellation_while_hashing_opaque_table_metadata() {
        let table = Table::new(
            TableId::new(1).expect("table id"),
            TableName::new("Opaque").expect("table name"),
            TableName::new("Opaque").expect("display name"),
            range("A1", "B3"),
            1,
            0,
            vec![
                TableColumn::new(1, "First", None).expect("column"),
                TableColumn::new(2, "Second", None).expect("column"),
            ],
        )
        .expect("table")
        .try_with_metadata(
            TableType::Worksheet,
            true,
            None,
            None,
            None,
            Some(vec![b'x'; 256 * 1024]),
        )
        .expect("valid metadata");
        let workbook = workbook_with_extras(Vec::new(), vec![table]);
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 6
        };

        assert_eq!(
            workbook_fingerprint_cancellable(&workbook, &cancelled),
            Err(())
        );
        assert!(
            polls.get() >= 6,
            "opaque bytes must be hashed in cancellable chunks"
        );
    }

    type TableDefinition<'a> = (
        &'a str,
        &'a str,
        CellRange,
        u32,
        u32,
        Vec<(&'a str, u32, Option<TotalsRowFunction>)>,
        u32,
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
                7_u32,
            )
        };
        let build = |definition: TableDefinition<'_>| {
            let (name, display_name, reference, header, totals, columns, id) = definition;
            let columns = columns
                .into_iter()
                .map(|(name, id, function)| TableColumn::new(id, name, function).expect("column"))
                .collect();
            workbook_with_extras(
                Vec::new(),
                vec![
                    Table::new(
                        TableId::new(id).expect("table id"),
                        TableName::new(name).expect("name"),
                        TableName::new(display_name).expect("display name"),
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
        changed.6 = 8;
        assert_ne!(reference, workbook_fingerprint(&build(changed)), "table ID");

        let mut changed = base();
        changed.0 = "Other";
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "programmatic name"
        );

        let mut changed = base();
        changed.2 = range("A1", "B5");
        assert_ne!(reference, workbook_fingerprint(&build(changed)), "@ref");

        let mut changed = base();
        changed.5[0].0 = "Renamed";
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "column name"
        );

        let mut changed = base();
        changed.5.swap(0, 1);
        changed.5[0].1 = 1;
        changed.5[1].1 = 2;
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "column order"
        );

        let mut changed = base();
        changed.5[1].2 = Some(TotalsRowFunction::Sum);
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "totalsRowFunction"
        );

        let mut changed = base();
        changed.5[0].1 = 3;
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "column ID"
        );

        let mut changed = base();
        changed.3 = 0;
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "header row count"
        );

        let mut changed = base();
        changed.4 = 1;
        assert_ne!(
            reference,
            workbook_fingerprint(&build(changed)),
            "totals row count"
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

    #[derive(Debug, Clone, Copy)]
    enum ExtendedTableMutation {
        TableType,
        TotalsRowShown,
        TotalsRowLabel,
        CalculatedFormulaText,
        CalculatedFormulaArray,
        TotalsFormulaText,
        AutoFilterRange,
        AutoFilterRangePresence,
        AutoFilterColumns,
        NestedSortState,
        FilterCriteria,
        SortStateRange,
        SortStateFlags,
        SortStateConditions,
        SortConditionAttributes,
        StyleName,
        StyleFlags,
        OpaqueSource,
    }

    #[test]
    fn workbook_fingerprint_folds_extended_table_metadata() {
        let reference = workbook_fingerprint(&workbook_with_extended_table(None));
        for mutation in [
            ExtendedTableMutation::TableType,
            ExtendedTableMutation::TotalsRowShown,
            ExtendedTableMutation::TotalsRowLabel,
            ExtendedTableMutation::CalculatedFormulaText,
            ExtendedTableMutation::CalculatedFormulaArray,
            ExtendedTableMutation::TotalsFormulaText,
            ExtendedTableMutation::AutoFilterRange,
            ExtendedTableMutation::AutoFilterRangePresence,
            ExtendedTableMutation::AutoFilterColumns,
            ExtendedTableMutation::NestedSortState,
            ExtendedTableMutation::FilterCriteria,
            ExtendedTableMutation::SortStateRange,
            ExtendedTableMutation::SortStateFlags,
            ExtendedTableMutation::SortStateConditions,
            ExtendedTableMutation::SortConditionAttributes,
            ExtendedTableMutation::StyleName,
            ExtendedTableMutation::StyleFlags,
            ExtendedTableMutation::OpaqueSource,
        ] {
            assert_ne!(
                reference,
                workbook_fingerprint(&workbook_with_extended_table(Some(mutation))),
                "{mutation:?}"
            );
        }
        assert_eq!(
            reference,
            workbook_fingerprint(&workbook_with_extended_table(None)),
            "stable"
        );
    }

    fn workbook_with_extended_table(mutation: Option<ExtendedTableMutation>) -> WorkbookSnapshot {
        let table_type = if matches!(mutation, Some(ExtendedTableMutation::TableType)) {
            TableType::Xml
        } else {
            TableType::QueryTable
        };
        let totals_row_shown = !matches!(mutation, Some(ExtendedTableMutation::TotalsRowShown));
        let label = if matches!(mutation, Some(ExtendedTableMutation::TotalsRowLabel)) {
            "Grand Total"
        } else {
            "Total"
        };
        let calculated_text =
            if matches!(mutation, Some(ExtendedTableMutation::CalculatedFormulaText)) {
                "[@Amount]*3"
            } else {
                "[@Amount]*2"
            };
        let calculated_array = !matches!(
            mutation,
            Some(ExtendedTableMutation::CalculatedFormulaArray)
        );
        let totals_text = if matches!(mutation, Some(ExtendedTableMutation::TotalsFormulaText)) {
            "SUBTOTAL(101,[Amount])"
        } else {
            "SUBTOTAL(109,[Amount])"
        };
        let columns = vec![
            TableColumn::new(1, "Region", None)
                .expect("column")
                .with_metadata(Some(label.to_owned()), None, None),
            TableColumn::new(2, "Amount", Some(TotalsRowFunction::Custom))
                .expect("column")
                .with_metadata(
                    None,
                    Some(TableFormula::new(
                        FormulaText::from_xlsx(calculated_text).expect("formula"),
                        calculated_array,
                    )),
                    Some(TableFormula::new(
                        FormulaText::from_xlsx(totals_text).expect("formula"),
                        false,
                    )),
                ),
        ];
        let auto_filter_range = if matches!(mutation, Some(ExtendedTableMutation::AutoFilterRange))
        {
            range("A1", "B4")
        } else {
            range("A1", "B3")
        };
        let filter_column_id = if matches!(mutation, Some(ExtendedTableMutation::AutoFilterColumns))
        {
            0
        } else {
            1
        };
        let filter_value = if matches!(mutation, Some(ExtendedTableMutation::FilterCriteria)) {
            "West"
        } else {
            "East"
        };
        let nested_sort = TableSortState::from_xlsx(
            range("A2", "B3"),
            !matches!(mutation, Some(ExtendedTableMutation::NestedSortState)),
            false,
            None,
            vec![TableSortCondition::from_xlsx(
                range("B2", "B3"),
                false,
                None,
                None,
                None,
                None,
                None,
            )],
        );
        let auto_filter = TableAutoFilter::from_xlsx(
            auto_filter_range,
            !matches!(
                mutation,
                Some(ExtendedTableMutation::AutoFilterRangePresence)
            ),
            vec![TableFilterColumn::from_xlsx(
                filter_column_id,
                false,
                true,
                Some(TableFilterCriteria::Values(TableValueFilters::from_xlsx(
                    false,
                    None,
                    vec![TableFilterItem::Value(Some(filter_value.into()))],
                ))),
            )],
            Some(nested_sort),
        );
        let sort_range = if matches!(mutation, Some(ExtendedTableMutation::SortStateRange)) {
            range("A1", "B3")
        } else {
            range("A2", "B3")
        };
        let sort_flags = matches!(mutation, Some(ExtendedTableMutation::SortStateFlags));
        let condition_range =
            if matches!(mutation, Some(ExtendedTableMutation::SortStateConditions)) {
                range("A2", "A3")
            } else {
                range("B2", "B3")
            };
        let condition_descending = matches!(
            mutation,
            Some(ExtendedTableMutation::SortConditionAttributes)
        );
        let conditions = vec![TableSortCondition::from_xlsx(
            condition_range,
            condition_descending,
            None,
            None,
            None,
            None,
            None,
        )];
        let sort_method = if matches!(
            mutation,
            Some(ExtendedTableMutation::SortConditionAttributes)
        ) {
            Some(TableSortMethod::Stroke)
        } else {
            None
        };
        let sort_state =
            TableSortState::from_xlsx(sort_range, sort_flags, false, sort_method, conditions);
        let style_name = if matches!(mutation, Some(ExtendedTableMutation::StyleName)) {
            "TableStyleMedium3"
        } else {
            "TableStyleMedium2"
        };
        let style_flags = matches!(mutation, Some(ExtendedTableMutation::StyleFlags));
        let opaque_source = if matches!(mutation, Some(ExtendedTableMutation::OpaqueSource)) {
            b"opaque:b".to_vec()
        } else {
            b"opaque:a".to_vec()
        };
        let table = Table::new(
            TableId::new(1).expect("table id"),
            TableName::new("Sales").expect("table name"),
            TableName::new("SalesDisplay").expect("display name"),
            range("A1", "B4"),
            1,
            0,
            columns,
        )
        .expect("table")
        .try_with_metadata(
            table_type,
            totals_row_shown,
            Some(auto_filter),
            Some(sort_state),
            Some(TableStyleInfo::new(
                Some(style_name.to_owned()),
                style_flags,
                false,
                true,
                false,
            )),
            Some(opaque_source),
        )
        .expect("valid table metadata");
        workbook_with_extras(Vec::new(), vec![table])
    }

    fn range(start: &str, end: &str) -> CellRange {
        CellRange::new(
            CellAddress::from_a1(start).expect("start"),
            CellAddress::from_a1(end).expect("end"),
        )
        .expect("range")
    }

    fn workbook_with_extras(merged_ranges: Vec<CellRange>, tables: Vec<Table>) -> WorkbookSnapshot {
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
