use crate::{CalculationCellId, SheetId, TableId};

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
    pub(super) calculation_metadata_changed: bool,
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
            calculation_metadata_changed: self.calculation_metadata_changed,
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

    /// Returns whether workbook-wide calculation interpretation changed.
    pub const fn calculation_metadata_changed(&self) -> bool {
        self.calculation_metadata_changed
    }
}
