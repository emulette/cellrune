use super::EditInverse;
use crate::{CalculationCellId, SheetId, SheetName, TableId};

/// Result of committing one atomic edit batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditReceipt {
    pub(super) base_revision: u64,
    pub(super) result_revision: u64,
    pub(super) applied_change_count: usize,
    pub(super) changed_cells: Vec<CalculationCellId>,
    pub(super) calculation_changed_cells: Vec<CalculationCellId>,
    pub(super) created_sheet_ids: Vec<SheetId>,
    pub(super) changed_table_ids: Vec<TableId>,
    pub(super) topology_changed: bool,
    /// Whether every topology change came from a cell gaining, losing, or replacing a formula.
    pub(super) cell_formulas_changed_topology_only: bool,
    pub(super) calculation_metadata_changed: bool,
    /// Whether a cell's number format changed with its value and formula kept, which only
    /// formulas reading number formats see.
    pub(super) number_formats_changed: bool,
    /// The sheets whose cells a row, column, or range move shifted without changing a defined
    /// name or a table. The cells it vacated and filled are not in `calculation_changed_cells`,
    /// which the sheets' contents before and after tell.
    pub(super) moved_cell_sheets: Vec<SheetId>,
    /// The names that find another sheet, or a sheet where none was, because a sheet was added
    /// or renamed without changing a defined name or a table.
    pub(super) sheet_names_changed: Vec<SheetName>,
    /// Whether the tables in `changed_table_ids` changed by table edits that changed no defined
    /// name, which change only the formulas reading those tables.
    pub(super) local_table_edits: bool,
    pub(super) inverse: Option<Box<EditInverse>>,
}

impl EditReceipt {
    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        fn clone_copy_slice<T: Copy>(
            values: &[T],
            cancelled: &impl Fn() -> bool,
        ) -> Result<Vec<T>, ()> {
            let mut cloned = Vec::with_capacity(values.len());
            for value in values {
                if cancelled() {
                    return Err(());
                }
                cloned.push(*value);
            }
            Ok(cloned)
        }

        Ok(Self {
            base_revision: self.base_revision,
            result_revision: self.result_revision,
            applied_change_count: self.applied_change_count,
            changed_cells: clone_copy_slice(&self.changed_cells, cancelled)?,
            calculation_changed_cells: clone_copy_slice(
                &self.calculation_changed_cells,
                cancelled,
            )?,
            created_sheet_ids: clone_copy_slice(&self.created_sheet_ids, cancelled)?,
            changed_table_ids: clone_copy_slice(&self.changed_table_ids, cancelled)?,
            topology_changed: self.topology_changed,
            cell_formulas_changed_topology_only: self.cell_formulas_changed_topology_only,
            calculation_metadata_changed: self.calculation_metadata_changed,
            number_formats_changed: self.number_formats_changed,
            moved_cell_sheets: clone_copy_slice(&self.moved_cell_sheets, cancelled)?,
            sheet_names_changed: self.sheet_names_changed.clone(),
            local_table_edits: self.local_table_edits,
            inverse: self.inverse.clone(),
        })
    }

    /// Returns the revision checked before the batch.
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// Returns the revision installed after the batch.
    pub const fn result_revision(&self) -> u64 {
        self.result_revision
    }

    /// Returns the number of ordered operations applied.
    pub const fn applied_change_count(&self) -> usize {
        self.applied_change_count
    }

    /// Returns cells whose source content or format changed in deterministic order.
    pub fn changed_cells(&self) -> &[CalculationCellId] {
        &self.changed_cells
    }

    /// Returns cells whose source value or formula changed calculation semantics.
    pub fn calculation_changed_cells(&self) -> &[CalculationCellId] {
        &self.calculation_changed_cells
    }

    /// Returns stable IDs allocated for `AddSheet` operations in operation order.
    pub fn created_sheet_ids(&self) -> &[SheetId] {
        &self.created_sheet_ids
    }

    /// Returns stable IDs of tables whose metadata or materialized worksheet content changed.
    pub fn changed_table_ids(&self) -> &[TableId] {
        &self.changed_table_ids
    }

    /// Returns whether formula, name, or sheet topology must be recompiled.
    pub const fn topology_changed(&self) -> bool {
        self.topology_changed
    }

    /// Returns whether the topology changed only because cells gained, lost, or replaced a
    /// formula, which a calculation session can apply to its compiled state locally.
    pub(crate) const fn cell_formulas_changed_topology_only(&self) -> bool {
        self.topology_changed && self.cell_formulas_changed_topology_only
    }

    /// Returns whether workbook-wide calculation interpretation changed.
    pub const fn calculation_metadata_changed(&self) -> bool {
        self.calculation_metadata_changed
    }

    /// Returns whether a cell's number format changed while its value and formula stayed.
    /// Returns the sheets whose cells moved, as [`Self::cell_formulas_changed_topology_only`] counts
    /// them: the cells that moved changed as formula and value edits would change them.
    pub(crate) fn moved_cell_sheets(&self) -> &[SheetId] {
        &self.moved_cell_sheets
    }

    /// Returns the sheet names that find a different sheet than before, as
    /// [`Self::cell_formulas_changed_topology_only`] counts them: formulas naming them, or reading
    /// sheet names or the number of sheets, read differently.
    pub(crate) fn sheet_names_changed(&self) -> &[SheetName] {
        &self.sheet_names_changed
    }

    /// Returns whether table edits changed the tables in [`Self::changed_table_ids`] as
    /// [`Self::cell_formulas_changed_topology_only`] counts them: the formulas reading those tables
    /// read differently.
    pub(crate) const fn local_table_edits(&self) -> bool {
        self.local_table_edits
    }

    pub(crate) const fn number_formats_changed(&self) -> bool {
        self.number_formats_changed
    }

    /// Returns the changes that undo the batch and what they cannot restore, for a batch that
    /// inserted, deleted, or moved rows, columns, or ranges, copied ranges, deleted, moved, or
    /// copied a sheet, renamed a defined name, created a table, changed its range, converted it to
    /// a range, or set a sheet auto-filter. `None` for any other batch, and for one whose
    /// structural edits cannot be undone by changes, as when a later change in it filled the rows
    /// a deletion freed.
    pub fn inverse(&self) -> Option<&EditInverse> {
        self.inverse.as_deref()
    }
}
