mod auto_filter_edit;
mod change;
mod copy_edit;
mod executor;
mod formula_edit;
mod grid_edit;
mod inverse;
mod name_rename;
mod receipt;
mod sheet_copy;
mod sheet_delete;
mod sheet_move;
mod staged;
mod tab_order;
mod table_convert;
mod table_create;
mod table_edit;
mod table_range;
mod widen_edit;

pub use change::{EditBatch, WorkbookChange};
pub use inverse::{EditInverse, UnrestoredContent, UnrestoredContentKind};
pub(crate) use name_rename::DefinedNameRename;
pub use receipt::EditReceipt;

use crate::ValidationError;
use crate::calculation::formula_rewrite::{FormulaRewriteError, FormulaRewriteLimitKind};

#[derive(Debug)]
pub(crate) enum BatchExecutionError {
    Validation(ValidationError),
    Rewrite(FormulaRewriteError),
    Materialization(TableMaterializationError),
}

impl From<ValidationError> for BatchExecutionError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

impl From<FormulaRewriteError> for BatchExecutionError {
    fn from(error: FormulaRewriteError) -> Self {
        Self::Rewrite(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableMaterializationError {
    Cancelled,
    LimitExceeded { limit: usize, actual: usize },
}

pub(crate) struct TableMaterializationBudget<'a> {
    limit: usize,
    cells: usize,
    cancelled: &'a dyn Fn() -> bool,
}

impl<'a> TableMaterializationBudget<'a> {
    pub(crate) fn new(limit: usize, cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            limit,
            cells: 0,
            cancelled,
        }
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), TableMaterializationError> {
        if (self.cancelled)() {
            Err(TableMaterializationError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn charge_cell(&mut self) -> Result<(), TableMaterializationError> {
        self.charge_cells(1)
    }

    pub(crate) fn charge_cells(&mut self, cells: usize) -> Result<(), TableMaterializationError> {
        self.check_cancelled()?;
        let actual = self.cells.saturating_add(cells);
        if actual > self.limit {
            return Err(TableMaterializationError::LimitExceeded {
                limit: self.limit,
                actual,
            });
        }
        self.cells = actual;
        Ok(())
    }
}

impl From<TableMaterializationError> for BatchExecutionError {
    fn from(error: TableMaterializationError) -> Self {
        Self::Materialization(error)
    }
}

impl BatchExecutionError {
    pub(crate) fn into_validation(self) -> ValidationError {
        match self {
            Self::Validation(error) => error,
            Self::Rewrite(FormulaRewriteError::Parse { code, span, owner }) => {
                ValidationError::FormulaRewriteParseFailed {
                    parse_code: code.as_str().to_owned(),
                    start: span.start,
                    end: span.end,
                    owner,
                }
            }
            Self::Rewrite(FormulaRewriteError::SourceEdit(_)) => {
                ValidationError::FormulaRewriteParseFailed {
                    parse_code: "formula.rewrite.source_edit".to_owned(),
                    start: 0,
                    end: 0,
                    owner: None,
                }
            }
            Self::Rewrite(
                FormulaRewriteError::Cancelled | FormulaRewriteError::LimitExceeded { .. },
            ) => unreachable!(
                "unbounded non-cancellable draft editing cannot exhaust rewrite control"
            ),
            Self::Materialization(
                TableMaterializationError::Cancelled
                | TableMaterializationError::LimitExceeded { .. },
            ) => unreachable!(
                "unbounded non-cancellable draft editing cannot exhaust materialization control"
            ),
        }
    }
}

pub(crate) fn rewrite_limit_detail(
    kind: FormulaRewriteLimitKind,
    limit: usize,
    actual: usize,
) -> String {
    format!("kind={}, actual={actual}, limit={limit}", kind.as_str())
}
