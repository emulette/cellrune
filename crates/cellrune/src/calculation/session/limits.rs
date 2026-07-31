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
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_batch_operations: 100_000,
            max_evaluated_cells: 1_000_000,
            max_delta_cells: 1_000_000,
            max_retained_deltas: 256,
            max_delta_page: 100,
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
