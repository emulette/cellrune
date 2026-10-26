use std::collections::BTreeSet;

use super::super::eval::CompiledWorkbook;
use super::super::functions::descriptor::Volatility;
use super::super::runtime::CellId;
use crate::{
    CalculationCellId, CalculationSnapshot, CellContent, MaterializedResultOrigin, SheetId,
    SheetName, TableId, TableName, WorkbookSnapshot,
};

const CHARGED_WORK_POLL_INTERVAL: u32 = 256;

/// Tracks charged impact-preparation work and polls cancellation at bounded intervals.
struct ImpactWorkCharger<'a> {
    charged: u32,
    cancelled: &'a dyn Fn() -> bool,
}

impl ImpactWorkCharger<'_> {
    fn charge(&mut self) -> Result<(), ()> {
        self.charged += 1;
        if self.charged >= CHARGED_WORK_POLL_INTERVAL {
            self.charged = 0;
            if (self.cancelled)() {
                return Err(());
            }
        }
        Ok(())
    }
}

pub(super) fn affected_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    previous: Option<&CalculationSnapshot>,
    changed_cells: &[CalculationCellId],
    existing_dirty: &BTreeSet<CalculationCellId>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let impact = affected_formula_impact(workbook, compiled, previous, changed_cells, cancelled)?;
    let mut replacement = BTreeSet::new();
    for cell in existing_dirty {
        if cancelled() {
            return Err(());
        }
        replacement.insert(*cell);
    }
    replacement.extend(impact.direct);
    replacement.extend(impact.transitive);
    Ok(replacement)
}

pub(super) struct AffectedFormulaImpact {
    pub(super) direct: BTreeSet<CalculationCellId>,
    pub(super) transitive: BTreeSet<CalculationCellId>,
}

pub(super) fn affected_formula_impact(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    previous: Option<&CalculationSnapshot>,
    changed_cells: &[CalculationCellId],
    cancelled: &impl Fn() -> bool,
) -> Result<AffectedFormulaImpact, ()> {
    if cancelled() {
        return Err(());
    }

    let mut charger = ImpactWorkCharger {
        charged: 0,
        cancelled,
    };

    let mut changed_internal = Vec::with_capacity(changed_cells.len());
    for cell in changed_cells {
        charger.charge()?;
        if let Some(internal) = public_to_internal(workbook, *cell) {
            changed_internal.push(internal);
        }
    }
    let mut direct = BTreeSet::new();
    if let Some(previous) = previous {
        for cell in changed_cells {
            charger.charge()?;
            let Some(materialized) = previous.materialized_cell(*cell) else {
                continue;
            };
            if let MaterializedResultOrigin::DynamicSpill { anchor, .. } = materialized.origin()
                && let Some(anchor) = public_to_internal(workbook, anchor)
            {
                direct.insert(anchor);
            }
        }
    }
    for changed in changed_internal {
        let formulas = compiled.direct_affected_formulas(changed, &mut || charger.charge())?;
        for formula in formulas {
            charger.charge()?;
            direct.insert(formula);
        }
        if compiled.contains_formula(changed) {
            direct.insert(changed);
        }
    }
    let mut all = direct.clone();
    compiled.close_over_dependents(&mut all, direct.iter().copied(), &mut || charger.charge())?;
    let mut public_direct = BTreeSet::new();
    for cell in direct {
        charger.charge()?;
        if let Some(cell) = internal_to_public(workbook, cell) {
            public_direct.insert(cell);
        }
    }
    let mut public_transitive = BTreeSet::new();
    for cell in all {
        charger.charge()?;
        if let Some(cell) = internal_to_public(workbook, cell)
            && !public_direct.contains(&cell)
        {
            public_transitive.insert(cell);
        }
    }
    Ok(AffectedFormulaImpact {
        direct: public_direct,
        transitive: public_transitive,
    })
}

/// Returns the formulas whose references depend on values and every formula depending on them:
/// what an edit can change through the cells such a formula reads now, which the targets it was
/// compiled with may no longer cover.
pub(super) fn value_dependent_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let impact = reader_impact(
        workbook,
        compiled,
        compiled.value_dependent_formulas(),
        cancelled,
    )?;
    let mut all = impact.direct;
    all.extend(impact.transitive);
    Ok(all)
}

/// Returns the formulas that read a changed volatile input and every formula depending on them.
pub(super) fn volatile_input_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    changed: &[Volatility],
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let readers = changed
        .iter()
        .flat_map(|volatility| compiled.volatile_formulas(*volatility));
    let impact = reader_impact(workbook, compiled, readers, cancelled)?;
    let mut all = impact.direct;
    all.extend(impact.transitive);
    Ok(all)
}

/// Returns the formulas that read number formats, as `direct`, and every formula depending on
/// them, as `transitive`: what a format-only edit can change.
pub(super) fn number_format_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    cancelled: &impl Fn() -> bool,
) -> Result<AffectedFormulaImpact, ()> {
    reader_impact(
        workbook,
        compiled,
        compiled.number_format_formulas(),
        cancelled,
    )
}

fn reader_impact<'a>(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    readers: impl IntoIterator<Item = &'a CellId>,
    cancelled: &impl Fn() -> bool,
) -> Result<AffectedFormulaImpact, ()> {
    let mut charger = ImpactWorkCharger {
        charged: 0,
        cancelled,
    };
    let mut all = BTreeSet::new();
    let mut pending = Vec::new();
    for cell in readers {
        charger.charge()?;
        if all.insert(*cell) {
            pending.push(*cell);
        }
    }
    compiled.close_over_dependents(&mut all, pending.iter().copied(), &mut || charger.charge())?;
    let mut direct = BTreeSet::new();
    for cell in &pending {
        charger.charge()?;
        direct.extend(internal_to_public(workbook, *cell));
    }
    let mut transitive = BTreeSet::new();
    for cell in all {
        charger.charge()?;
        if let Some(cell) = internal_to_public(workbook, cell)
            && !direct.contains(&cell)
        {
            transitive.insert(cell);
        }
    }
    Ok(AffectedFormulaImpact { direct, transitive })
}

pub(super) fn public_to_internal(
    workbook: &WorkbookSnapshot,
    cell: CalculationCellId,
) -> Option<CellId> {
    let sheet = workbook.sheet_position(cell.sheet_id())?;
    Some((
        sheet,
        cell.address().row().get(),
        cell.address().column().get(),
    ))
}

fn internal_to_public(workbook: &WorkbookSnapshot, cell: CellId) -> Option<CalculationCellId> {
    let sheet = workbook.sheets().get(cell.0)?;
    let address = crate::CellAddress::from_indices(cell.1, cell.2).ok()?;
    Some(CalculationCellId::new(sheet.id(), address))
}

pub(super) fn formula_cells(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let mut formulas = BTreeSet::new();
    for cell in compiled.formula_cells() {
        if cancelled() {
            return Err(());
        }
        if let Some(cell) = internal_to_public(workbook, cell) {
            formulas.insert(cell);
        }
    }
    Ok(formulas)
}

pub(super) fn formula_cells_from_workbook(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let mut formulas = BTreeSet::new();
    for sheet in workbook.sheets() {
        for cell in sheet.cells() {
            if cancelled() {
                return Err(());
            }
            if matches!(cell.content(), CellContent::Formula(_)) {
                formulas.insert(CalculationCellId::new(sheet.id(), cell.address()));
            }
        }
    }
    Ok(formulas)
}

/// Returns the cells of `sheets` whose content or number format differs between `before` and
/// `after`, in sheet and row-major order: those a row, column, or range move vacated or filled.
pub(super) fn moved_cells(
    before: &WorkbookSnapshot,
    after: &WorkbookSnapshot,
    sheets: &[SheetId],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<CalculationCellId>, ()> {
    let mut charger = ImpactWorkCharger {
        charged: 0,
        cancelled,
    };
    let mut moved = Vec::new();
    for sheet_id in sheets {
        let (mut old, mut new) = (
            sheet_cells(before, *sheet_id).peekable(),
            sheet_cells(after, *sheet_id).peekable(),
        );
        loop {
            charger.charge()?;
            // `Sheet::cells` runs in row-major order.
            let position =
                |cell: &&crate::Cell| (cell.address().row().get(), cell.address().column().get());
            let address = match (old.peek(), new.peek()) {
                (None, None) => break,
                (Some(_), None) => old.next().map(crate::Cell::address),
                (None, Some(_)) => new.next().map(crate::Cell::address),
                (Some(left), Some(right)) => match position(left).cmp(&position(right)) {
                    std::cmp::Ordering::Less => old.next().map(crate::Cell::address),
                    std::cmp::Ordering::Greater => new.next().map(crate::Cell::address),
                    std::cmp::Ordering::Equal => {
                        let (left, right) = (old.next(), new.next());
                        (left != right)
                            .then(|| left.map(crate::Cell::address))
                            .flatten()
                    }
                },
            };
            moved.extend(address.map(|address| CalculationCellId::new(*sheet_id, address)));
        }
    }
    Ok(moved)
}

fn sheet_cells(
    workbook: &WorkbookSnapshot,
    sheet_id: SheetId,
) -> impl Iterator<Item = &crate::Cell> {
    workbook
        .sheet_by_id(sheet_id)
        .into_iter()
        .flat_map(crate::Sheet::cells)
}

/// Returns the formulas whose results a sheet added or renamed as `names` can change without
/// rewriting them: those naming one of `names` and those reading sheet names or the number of
/// sheets. Returns `None` when a defined name reads them, which changes every formula using it.
pub(super) fn sheet_name_readers(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    names: &[SheetName],
    cancelled: &impl Fn() -> bool,
) -> Result<Option<BTreeSet<CalculationCellId>>, ()> {
    let keys = names
        .iter()
        .map(|name| name.lookup_key().to_owned())
        .collect::<BTreeSet<_>>();
    if workbook.defined_names().iter().any(|name| {
        crate::calculation::formula_references::formula_references(name.formula().as_str())
            .is_some_and(|references| references.reads_sheet_names(&keys))
    }) {
        return Ok(None);
    }
    Ok(Some(
        compiled
            .sheet_name_readers(&keys, false, cancelled)?
            .into_iter()
            .filter_map(|cell| internal_to_public(workbook, cell))
            .collect(),
    ))
}

/// Returns the formulas whose results table edits of `tables` can change without rewriting them:
/// those reading the tables, naming one by its name before or after, or standing in a table's
/// range before or after, whose structured references without a table name follow it. Returns
/// `None` when a defined name names one of the tables.
pub(super) fn table_readers(
    before: &WorkbookSnapshot,
    after: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    tables: &[TableId],
    cancelled: &impl Fn() -> bool,
) -> Result<Option<BTreeSet<CalculationCellId>>, ()> {
    let ids = tables.iter().copied().collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    let mut ranges = Vec::new();
    for workbook in [before, after] {
        for id in &ids {
            if let Some(location) = workbook.table_location_by_id(*id) {
                let sheet = &workbook.sheets()[location.sheet_index];
                let table = &sheet.tables()[location.table_index];
                names.insert(table.display_name().lookup_key().to_owned());
                ranges.push((sheet.id(), table.range()));
            }
        }
    }
    let names_table = |text: &str| {
        crate::calculation::formula_references::formula_references(text).is_some_and(|references| {
            references.tables.iter().any(|table| {
                TableName::new(table.as_str()).is_ok_and(|table| names.contains(table.lookup_key()))
            })
        })
    };
    if after
        .defined_names()
        .iter()
        .any(|name| names_table(name.formula().as_str()))
    {
        return Ok(None);
    }
    let mut charger = ImpactWorkCharger {
        charged: 0,
        cancelled,
    };
    let mut readers = compiled
        .table_readers(&ids, &names, cancelled)?
        .into_iter()
        .filter_map(|cell| internal_to_public(before, cell))
        .collect::<BTreeSet<_>>();
    for cell in compiled.formula_cells() {
        charger.charge()?;
        if let Some(public) = internal_to_public(before, cell)
            && ranges.iter().any(|(sheet, range)| {
                public.sheet_id() == *sheet && range.contains(public.address())
            })
        {
            readers.insert(public);
        }
    }
    Ok(Some(readers))
}

/// Whether every formula reading one of `tables` is among `edits`, which recompile in place
/// with what they read now.
pub(super) fn tables_covered_by(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    tables: &BTreeSet<TableId>,
    edits: &BTreeSet<CalculationCellId>,
    cancelled: &impl Fn() -> bool,
) -> Result<bool, ()> {
    Ok(compiled
        .table_readers(tables, &BTreeSet::new(), cancelled)?
        .into_iter()
        .all(|cell| internal_to_public(workbook, cell).is_some_and(|cell| edits.contains(&cell))))
}

/// What deleting, copying, importing, or moving sheets between `before` and `after` changes for
/// the compiled formulas.
pub(super) struct SheetOrderChange {
    /// The index in `after` of each sheet of `before`, `None` for a deleted one.
    pub(super) indices: Vec<Option<usize>>,
    /// The sheets of `after` that `before` does not hold.
    pub(super) added: Vec<SheetId>,
    /// The formulas whose results the change can alter without rewriting them: those naming a
    /// sheet it adds, deletes, or moves, reading sheet names or the number of sheets, reading
    /// across sheets, or reading a table on a moved sheet. `None` when a defined name reads the
    /// sheet names or a table name, which changes every formula using it.
    pub(super) readers: Option<BTreeSet<CalculationCellId>>,
    /// The formulas of the deleted sheets.
    pub(super) removed: BTreeSet<CalculationCellId>,
}

pub(super) fn sheet_order_change(
    before: &WorkbookSnapshot,
    after: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    cancelled: &impl Fn() -> bool,
) -> Result<SheetOrderChange, ()> {
    let indices = before
        .sheets()
        .iter()
        .map(|sheet| after.sheet_position(sheet.id()))
        .collect::<Vec<_>>();
    let added = after
        .sheets()
        .iter()
        .filter(|sheet| before.sheet_by_id(sheet.id()).is_none())
        .map(crate::Sheet::id)
        .collect::<Vec<_>>();
    let mut names = Vec::new();
    let mut moved_tables = Vec::new();
    for (index, sheet) in before.sheets().iter().enumerate() {
        if indices[index] != Some(index) {
            names.push(sheet.name().clone());
            if indices[index].is_some() {
                moved_tables.extend(sheet.tables().iter().map(crate::Table::id));
            }
        }
    }
    names.extend(
        after
            .sheets()
            .iter()
            .filter(|sheet| added.contains(&sheet.id()))
            .map(|sheet| sheet.name().clone()),
    );
    let removed = compiled
        .formula_cells()
        .filter(|cell| indices.get(cell.0).copied().flatten().is_none())
        .filter_map(|cell| internal_to_public(before, cell))
        .collect();
    let keys = names
        .iter()
        .map(|name| name.lookup_key().to_owned())
        .collect::<BTreeSet<_>>();
    let readers = match (
        sheet_name_readers(before, compiled, &names, cancelled)?,
        table_readers(before, after, compiled, &moved_tables, cancelled)?,
    ) {
        (Some(mut readers), Some(tables)) => {
            readers.extend(tables);
            readers.extend(
                compiled
                    .sheet_name_readers(&keys, true, cancelled)?
                    .into_iter()
                    .filter_map(|cell| internal_to_public(before, cell)),
            );
            Some(readers)
        }
        _ => None,
    };
    Ok(SheetOrderChange {
        indices,
        added,
        readers,
        removed,
    })
}
