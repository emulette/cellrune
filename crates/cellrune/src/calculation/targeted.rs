//! Bounded, immutable calculation of explicitly requested cells.

use super::{CalculationCellId, CalculationCellResult, CalculationOptions, CancellationToken};
use crate::{
    CellRange, Provenance, ProviderIdentity, SheetId, WorkbookFingerprint, WorkbookSnapshot,
};
use std::collections::{BTreeMap, BTreeSet};

mod errors;
pub use errors::{TargetCalculationError, TargetCalculationErrorCode};

/// A rectangular set of requested cells on one sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalculationTarget {
    sheet_id: SheetId,
    range: CellRange,
}

impl CalculationTarget {
    /// Constructs a target from validated sheet and range values.
    pub const fn new(sheet_id: SheetId, range: CellRange) -> Self {
        Self { sheet_id, range }
    }
    /// Constructs a single-cell target.
    pub fn cell(cell: CalculationCellId) -> Self {
        Self::new(
            cell.sheet_id(),
            CellRange::from_ordered(cell.address(), cell.address()),
        )
    }
    /// Returns the target sheet.
    pub const fn sheet_id(self) -> SheetId {
        self.sheet_id
    }
    /// Returns the requested range.
    pub const fn range(self) -> CellRange {
        self.range
    }
}

/// Request-wide limits independent of per-formula calculation limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetCalculationLimits {
    max_targets: usize,
    max_result_cells: usize,
    max_evaluated_cells: usize,
}

impl Default for TargetCalculationLimits {
    fn default() -> Self {
        Self {
            max_targets: 1_024,
            max_result_cells: 10_000,
            max_evaluated_cells: 100_000,
        }
    }
}

impl TargetCalculationLimits {
    /// Constructs non-zero request limits.
    ///
    /// # Errors
    /// Returns an invalid-limits error for zero limits.
    pub fn new(
        max_targets: usize,
        max_result_cells: usize,
        max_evaluated_cells: usize,
    ) -> Result<Self, TargetCalculationError> {
        if [max_targets, max_result_cells, max_evaluated_cells].contains(&0) {
            return Err(TargetCalculationError::new(
                TargetCalculationErrorCode::InvalidLimits,
            ));
        }
        Ok(Self {
            max_targets,
            max_result_cells,
            max_evaluated_cells,
        })
    }
    /// Returns the maximum number of input rectangles.
    pub const fn max_targets(self) -> usize {
        self.max_targets
    }
    /// Returns the maximum number of distinct requested cells.
    pub const fn max_result_cells(self) -> usize {
        self.max_result_cells
    }
    /// Returns the maximum number of formula evaluations.
    pub const fn max_evaluated_cells(self) -> usize {
        self.max_evaluated_cells
    }
}

/// Immutable values for explicitly requested cells, never a complete workbook calculation.
#[derive(Debug, Clone)]
pub struct TargetCalculationResult {
    pub(super) cells: BTreeMap<CalculationCellId, CalculationCellResult>,
    pub(super) source_revision: u64,
    pub(super) source_fingerprint: WorkbookFingerprint,
    pub(super) provenance: Provenance,
    pub(super) options: CalculationOptions,
    pub(super) limits: TargetCalculationLimits,
    pub(super) evaluated_count: usize,
    pub(super) parsed_formula_count: usize,
    pub(super) reused_count: usize,
}

impl TargetCalculationResult {
    /// Returns one requested cell; unrequested cells are absent.
    pub fn cell(&self, cell: CalculationCellId) -> Option<&CalculationCellResult> {
        self.cells.get(&cell)
    }
    /// Iterates distinct requested cells in sheet-ID and row-major order.
    pub fn cells(
        &self,
    ) -> impl ExactSizeIterator<Item = (CalculationCellId, &CalculationCellResult)> {
        self.cells.iter().map(|(cell, result)| (*cell, result))
    }
    /// Returns the number of requested results.
    pub fn len(&self) -> usize {
        self.cells.len()
    }
    /// Returns whether the result is empty.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }
    /// Returns the immutable source revision.
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }
    /// Returns the semantic identity of the source.
    pub const fn source_fingerprint(&self) -> WorkbookFingerprint {
        self.source_fingerprint
    }
    /// Returns source and calculation provider provenance.
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }
    /// Returns the deterministic calculation options.
    pub const fn options(&self) -> CalculationOptions {
        self.options
    }
    /// Returns the request-wide resource limits.
    pub const fn limits(&self) -> TargetCalculationLimits {
        self.limits
    }
    /// Returns evaluator invocations, including precedents and dynamic-reference retries.
    pub const fn evaluated_count(&self) -> usize {
        self.evaluated_count
    }
    /// Returns the number of formula cells parsed by this request.
    pub const fn parsed_formula_count(&self) -> usize {
        self.parsed_formula_count
    }
    /// Returns the number of requested formula results reused from a current complete snapshot.
    pub const fn reused_count(&self) -> usize {
        self.reused_count
    }
}

/// Calculates requested cells and their required precedents without changing the source.
///
/// # Errors
/// Returns a stable request error for invalid targets, resource limits or cancellation.
pub fn calculate_targets(
    workbook: &WorkbookSnapshot,
    targets: &[CalculationTarget],
    options: CalculationOptions,
    limits: TargetCalculationLimits,
    cancellation: CancellationToken,
) -> Result<TargetCalculationResult, TargetCalculationError> {
    let cells = normalize_targets(workbook, targets, limits, &cancellation)?;
    super::eval::Engine::calculate_targets(workbook, &cells, options, limits, &cancellation)
}

pub(super) fn normalize_targets(
    workbook: &WorkbookSnapshot,
    targets: &[CalculationTarget],
    limits: TargetCalculationLimits,
    cancellation: &CancellationToken,
) -> Result<BTreeSet<CalculationCellId>, TargetCalculationError> {
    use TargetCalculationErrorCode as Code;
    if cancellation.is_cancelled() {
        return Err(TargetCalculationError::new(Code::Cancelled));
    }
    if targets.is_empty() {
        return Err(TargetCalculationError::new(Code::EmptyTargets));
    }
    if targets.len() > limits.max_targets {
        return Err(TargetCalculationError::new(Code::TargetLimitExceeded));
    }
    let mut cells = BTreeSet::new();
    for target in targets {
        if workbook.sheet_by_id(target.sheet_id).is_none() {
            return Err(TargetCalculationError::new(Code::SheetNotFound));
        }
        let count = u64::from(target.range.height()) * u64::from(target.range.width());
        if count > limits.max_result_cells as u64 {
            return Err(TargetCalculationError::new(Code::TargetLimitExceeded));
        }
        for row in target.range.start().row().get()..=target.range.end().row().get() {
            for column in target.range.start().column().get()..=target.range.end().column().get() {
                if cancellation.is_cancelled() {
                    return Err(TargetCalculationError::new(Code::Cancelled));
                }
                let address = crate::CellAddress::from_indices(row, column)
                    .expect(errors::VALIDATED_COORDINATES);
                cells.insert(CalculationCellId::new(target.sheet_id, address));
                if cells.len() > limits.max_result_cells {
                    return Err(TargetCalculationError::new(Code::TargetLimitExceeded));
                }
            }
        }
    }
    Ok(cells)
}

pub(super) fn source_provenance(workbook: &WorkbookSnapshot) -> Provenance {
    Provenance::new(
        ProviderIdentity::calculator(),
        workbook.provenance().input_hash(),
    )
}
