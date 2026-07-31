use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{SessionError, SessionErrorCode};

/// Stateful-session resource limits independent of formula-kernel limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    pub(super) max_batch_operations: usize,
    pub(super) max_evaluated_cells: usize,
    pub(super) max_delta_cells: usize,
    pub(super) max_retained_deltas: usize,
    pub(super) max_delta_page: usize,
    pub(super) max_rewrite_formulas: usize,
    pub(super) max_rewrite_source_bytes: usize,
    pub(super) max_rewrite_ast_nodes: usize,
    pub(super) max_rewrite_source_edits: usize,
    pub(super) max_table_materialized_cells: usize,
}

impl SessionLimits {
    /// Constructs validated non-zero session limits.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::InvalidLimits`] when any configured limit is zero.
    pub fn new(
        max_batch_operations: usize,
        max_evaluated_cells: usize,
        max_delta_cells: usize,
        max_retained_deltas: usize,
        max_delta_page: usize,
    ) -> Result<Self, SessionError> {
        if [
            max_batch_operations,
            max_evaluated_cells,
            max_delta_cells,
            max_retained_deltas,
            max_delta_page,
        ]
        .contains(&0)
        {
            return Err(SessionError::new(SessionErrorCode::InvalidLimits, None));
        }
        Ok(Self {
            max_batch_operations,
            max_evaluated_cells,
            max_delta_cells,
            max_retained_deltas,
            max_delta_page,
            ..Self::default()
        })
    }

    /// Returns the maximum operations in one atomic edit batch.
    pub const fn max_batch_operations(self) -> usize {
        self.max_batch_operations
    }

    /// Returns the maximum evaluated formula cells in one pass.
    pub const fn max_evaluated_cells(self) -> usize {
        self.max_evaluated_cells
    }

    /// Returns the maximum changed and removed cells in one delta.
    pub const fn max_delta_cells(self) -> usize {
        self.max_delta_cells
    }

    /// Returns the maximum installed deltas retained by one session.
    pub const fn max_retained_deltas(self) -> usize {
        self.max_retained_deltas
    }

    /// Returns the maximum deltas returned by one cursor page.
    pub const fn max_delta_page(self) -> usize {
        self.max_delta_page
    }

    /// Replaces the cumulative typed formula-rewrite limits.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::InvalidLimits`] when any supplied limit is zero.
    pub fn with_formula_rewrite_limits(
        mut self,
        max_formulas: usize,
        max_source_bytes: usize,
        max_ast_nodes: usize,
        max_source_edits: usize,
    ) -> Result<Self, SessionError> {
        if [
            max_formulas,
            max_source_bytes,
            max_ast_nodes,
            max_source_edits,
        ]
        .contains(&0)
        {
            return Err(SessionError::new(SessionErrorCode::InvalidLimits, None));
        }
        self.max_rewrite_formulas = max_formulas;
        self.max_rewrite_source_bytes = max_source_bytes;
        self.max_rewrite_ast_nodes = max_ast_nodes;
        self.max_rewrite_source_edits = max_source_edits;
        Ok(self)
    }

    /// Returns the maximum formulas inspected by one edit batch.
    pub const fn max_rewrite_formulas(self) -> usize {
        self.max_rewrite_formulas
    }

    /// Returns the maximum formula source bytes inspected by one edit batch.
    pub const fn max_rewrite_source_bytes(self) -> usize {
        self.max_rewrite_source_bytes
    }

    /// Returns the maximum typed AST nodes inspected by one edit batch.
    pub const fn max_rewrite_ast_nodes(self) -> usize {
        self.max_rewrite_ast_nodes
    }

    /// Returns the maximum source edits produced by one edit batch.
    pub const fn max_rewrite_source_edits(self) -> usize {
        self.max_rewrite_source_edits
    }

    /// Replaces the cumulative table-resize materialization-cell limit.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::InvalidLimits`] when `max_cells` is zero.
    pub fn with_table_materialization_limit(
        mut self,
        max_cells: usize,
    ) -> Result<Self, SessionError> {
        if max_cells == 0 {
            return Err(SessionError::new(SessionErrorCode::InvalidLimits, None));
        }
        self.max_table_materialized_cells = max_cells;
        Ok(self)
    }

    /// Returns the maximum worksheet cells inspected for table materialization by one edit batch.
    pub const fn max_table_materialized_cells(self) -> usize {
        self.max_table_materialized_cells
    }
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_batch_operations: 100_000,
            max_evaluated_cells: 1_000_000,
            max_delta_cells: 1_000_000,
            max_retained_deltas: 256,
            max_delta_page: 100,
            max_rewrite_formulas: 1_000_000,
            max_rewrite_source_bytes: 256 * 1024 * 1024,
            max_rewrite_ast_nodes: 10_000_000,
            max_rewrite_source_edits: 10_000_000,
            max_table_materialized_cells: 1_000_000,
        }
    }
}

/// Thread-safe cooperative cancellation signal for one bounded operation.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a non-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation. Already completed or installed results are not modified.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}
