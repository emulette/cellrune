//! Deterministic workbook formula capability scanning and calculation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::{
    CellAddress, CellValue, FiniteNumber, Provenance, ProviderIdentity, SheetId, WorkbookSnapshot,
};
use decimal::DecimalTrace;

mod ast;
mod coerce;
mod convert;
mod criteria;
mod decimal;
mod defined_name_analysis;
mod error;
mod eval;
mod formula_rebase;
pub(crate) mod formula_rewrite;
mod functions;
mod graph;
pub(crate) mod identity;
mod lambda;
mod lexer;
mod limits;
mod operators;
mod parser;
pub(crate) mod performance_counters;
pub(crate) mod persistent_store;
mod pipeline;
mod reference_resolution;
mod runtime;
mod scope;
mod session;
mod sheet_span;
mod structured_reference;
mod syntax;
mod textfmt;
mod value;
#[cfg(test)]
pub(crate) mod work_counter;

use crate::calculation::performance_counters::{WorkCounter, work_counter_add};
use crate::calculation::persistent_store::{
    PersistentRadixEntries, PersistentRadixMap, PersistentValue,
};

use error::{
    MESSAGE_BLOCKED_BY_UPSTREAM, MESSAGE_CIRCULAR_REFERENCE, MESSAGE_MISSING_FORMULA_TEXT,
    MESSAGE_PARSE_ERROR, MESSAGE_RESOURCE_LIMIT_EXCEEDED, MESSAGE_UNSUPPORTED_EXPRESSION,
    MESSAGE_UNSUPPORTED_FUNCTION, MESSAGE_UNSUPPORTED_NAME, MESSAGE_UNSUPPORTED_SHEET_RANGE,
    MESSAGE_UNSUPPORTED_STRUCTURED_REFERENCE, MESSAGE_VOLATILE_INPUT_MISSING,
};

pub(super) use crate::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};
pub use defined_name_analysis::{
    DefinedNameAnalysis, DefinedNameAnalysisError, DefinedNameAnalysisErrorKind,
    DefinedNameAnalysisLimitKind, DefinedNameAnalysisOptions, DefinedNameAnalysisOptionsError,
    DefinedNameDynamicKind, DefinedNameExternalReference, DefinedNameExternalTargetKind,
    DefinedNameInvalidReason, DefinedNameReferenceArea, DefinedNameSheetSpan,
    DefinedNameUnsupportedReason, analyze_defined_name, analyze_defined_name_cancellable,
    analyze_defined_name_with_options,
};
use limits::CalculationLimitKind;
pub use limits::{CalculationLimits, CalculationOptionsError};
pub use session::{
    ApplyChangesError, CalculationDecisionReason, CalculationDelta, CalculationDeltaCell,
    CalculationDeltaPage, CalculationExecutionMode, CancellationToken, CompletedCalculation,
    PreparedCalculation, PreparedEditBatch, RecalculationMode, SessionError, SessionErrorCode,
    SessionLimits, WorkbookCalculationSession,
};

/// Stable identity of a formula cell within one workbook snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalculationCellId {
    sheet_id: SheetId,
    address: CellAddress,
}

impl CalculationCellId {
    /// Constructs a formula cell identity from validated workbook coordinates.
    pub const fn new(sheet_id: SheetId, address: CellAddress) -> Self {
        Self { sheet_id, address }
    }

    /// Returns the workbook-local sheet ID.
    pub const fn sheet_id(self) -> SheetId {
        self.sheet_id
    }

    /// Returns the formula cell address.
    pub const fn address(self) -> CellAddress {
        self.address
    }
}

/// Stable machine-readable reason that a formula was not calculated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CalculationIssueCode {
    /// The workbook contains formula metadata without formula text.
    MissingFormulaText,
    /// Formula text is outside the supported grammar.
    ParseError,
    /// The parsed formula calls a function without an implemented kernel.
    UnsupportedFunction,
    /// A referenced defined name cannot be resolved or evaluated.
    UnsupportedName,
    /// A reference spans a 3-D sheet range, for example `Sheet1:Sheet3!A1`.
    UnsupportedSheetRange,
    /// A structured table reference reached a context without resolvable table geometry.
    ///
    /// This legacy classification remains stable for serialized reports. Supported structured
    /// references now calculate normally or produce the corresponding Excel error.
    UnsupportedStructuredReference,
    /// A parsed expression is not supported by the current engine.
    UnsupportedExpression,
    /// Formula parsing, dependency scheduling, or array evaluation exceeded a configured limit.
    ResourceLimitExceeded,
    /// A supported volatile function lacks an explicit deterministic input.
    VolatileInputMissing,
    /// The formula belongs to a dependency cycle.
    CircularReference,
    /// An upstream formula did not produce a value.
    BlockedByUpstream,
}

impl CalculationIssueCode {
    /// Returns the stable dotted identifier used in reports and logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingFormulaText => "calculation.missing_formula_text",
            Self::ParseError => "calculation.parse_error",
            Self::UnsupportedFunction => "calculation.unsupported_function",
            Self::UnsupportedName => "calculation.unsupported_name",
            Self::UnsupportedSheetRange => "calculation.unsupported_sheet_range",
            Self::UnsupportedStructuredReference => "calculation.unsupported_structured_reference",
            Self::UnsupportedExpression => "calculation.unsupported_expression",
            Self::ResourceLimitExceeded => "calculation.resource_limit_exceeded",
            Self::VolatileInputMissing => "calculation.volatile_input_missing",
            Self::CircularReference => "calculation.circular_reference",
            Self::BlockedByUpstream => "calculation.blocked_by_upstream",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::MissingFormulaText => MESSAGE_MISSING_FORMULA_TEXT,
            Self::ParseError => MESSAGE_PARSE_ERROR,
            Self::UnsupportedFunction => MESSAGE_UNSUPPORTED_FUNCTION,
            Self::UnsupportedName => MESSAGE_UNSUPPORTED_NAME,
            Self::UnsupportedSheetRange => MESSAGE_UNSUPPORTED_SHEET_RANGE,
            Self::UnsupportedStructuredReference => MESSAGE_UNSUPPORTED_STRUCTURED_REFERENCE,
            Self::UnsupportedExpression => MESSAGE_UNSUPPORTED_EXPRESSION,
            Self::ResourceLimitExceeded => MESSAGE_RESOURCE_LIMIT_EXCEEDED,
            Self::VolatileInputMissing => MESSAGE_VOLATILE_INPUT_MISSING,
            Self::CircularReference => MESSAGE_CIRCULAR_REFERENCE,
            Self::BlockedByUpstream => MESSAGE_BLOCKED_BY_UPSTREAM,
        }
    }
}

/// A structured formula calculation issue with optional source-specific detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalculationIssue {
    code: CalculationIssueCode,
    detail: Option<Box<str>>,
}

impl CalculationIssue {
    pub(crate) fn new(code: CalculationIssueCode, detail: Option<String>) -> Self {
        Self {
            code,
            detail: detail.map(String::into_boxed_str),
        }
    }

    /// Returns the stable issue code.
    pub const fn code(&self) -> CalculationIssueCode {
        self.code
    }

    /// Returns the shared human-readable message.
    pub const fn message(&self) -> &'static str {
        self.code.message()
    }

    /// Returns source-specific context such as a function name or parser position.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// Static grammar and function-surface capability for one formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaCapability {
    /// The formula grammar and referenced function surface are supported.
    Supported,
    /// One or more capabilities required by the formula are unavailable.
    Unsupported(Vec<CalculationIssue>),
}

/// Capability status for one formula cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaCapabilityEntry {
    cell: CalculationCellId,
    capability: FormulaCapability,
}

impl FormulaCapabilityEntry {
    pub(crate) const fn new(cell: CalculationCellId, capability: FormulaCapability) -> Self {
        Self { cell, capability }
    }

    /// Returns the formula cell identity.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns the formula's capability status.
    pub const fn capability(&self) -> &FormulaCapability {
        &self.capability
    }
}

/// Deterministically ordered capability report for all formula cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaCapabilityReport {
    entries: Vec<FormulaCapabilityEntry>,
    supported_count: usize,
}

impl FormulaCapabilityReport {
    pub(crate) fn new(entries: Vec<FormulaCapabilityEntry>) -> Self {
        let supported_count = entries
            .iter()
            .filter(|entry| matches!(entry.capability(), FormulaCapability::Supported))
            .count();
        Self {
            entries,
            supported_count,
        }
    }

    /// Returns entries in workbook sheet order and row-major cell order.
    pub fn entries(&self) -> &[FormulaCapabilityEntry] {
        &self.entries
    }

    /// Returns the total number of formula cells scanned.
    pub fn formula_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of formulas accepted by the static capability scan.
    pub const fn supported_count(&self) -> usize {
        self.supported_count
    }

    /// Returns the number of formulas requiring unsupported capabilities.
    pub fn unsupported_count(&self) -> usize {
        self.entries.len() - self.supported_count
    }

    /// Returns whether every formula passed the capability scan.
    pub fn is_supported(&self) -> bool {
        self.supported_count == self.entries.len()
    }
}

/// Whether a function name found in a workbook is implemented by the current engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionSupport {
    /// The normalized function name is connected to a validated calculation kernel.
    Supported,
    /// The function is syntactically valid but has no calculation kernel.
    Unsupported,
}

/// One deterministic entry in the supported function catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCatalogEntry {
    name: Box<str>,
    canonical_name: Box<str>,
    alias: bool,
    array_result: bool,
    official: bool,
}

impl FunctionCatalogEntry {
    pub(crate) fn new(
        name: String,
        canonical_name: String,
        alias: bool,
        array_result: bool,
        official: bool,
    ) -> Self {
        Self {
            name: name.into_boxed_str(),
            canonical_name: canonical_name.into_boxed_str(),
            alias,
            array_result,
            official,
        }
    }

    /// Returns the accepted Excel-facing name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the calculation kernel name after prefix and legacy-alias normalization.
    pub fn canonical_name(&self) -> &str {
        &self.canonical_name
    }

    /// Returns whether this entry is a legacy compatibility alias.
    pub const fn is_alias(&self) -> bool {
        self.alias
    }

    /// Returns whether the kernel can produce a multi-cell array result.
    pub const fn returns_array(&self) -> bool {
        self.array_result
    }

    /// Returns whether the name belongs to the tracked Microsoft function-list snapshot.
    pub const fn is_official(&self) -> bool {
        self.official
    }
}

/// Aggregated use of one normalized function name in a workbook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionUsageEntry {
    name: Box<str>,
    support: FunctionSupport,
    call_count: u64,
    formula_count: u64,
    sample_cells: Vec<CalculationCellId>,
}

impl FunctionUsageEntry {
    pub(crate) fn new(
        name: String,
        support: FunctionSupport,
        call_count: u64,
        formula_count: u64,
        sample_cells: Vec<CalculationCellId>,
    ) -> Self {
        Self {
            name: name.into_boxed_str(),
            support,
            call_count,
            formula_count,
            sample_cells,
        }
    }

    /// Returns the normalized uppercase function name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether the current engine implements the function.
    pub const fn support(&self) -> FunctionSupport {
        self.support
    }

    /// Returns the total number of calls, including repeated calls in one formula.
    pub const fn call_count(&self) -> u64 {
        self.call_count
    }

    /// Returns the number of distinct formula cells containing the function.
    pub const fn formula_count(&self) -> u64 {
        self.formula_count
    }

    /// Returns up to eight deterministic formula-cell samples.
    pub fn sample_cells(&self) -> &[CalculationCellId] {
        &self.sample_cells
    }
}

/// Workbook-level function demand report for prioritizing compatibility work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionUsageReport {
    entries: Vec<FunctionUsageEntry>,
    formula_count: usize,
    parsed_formula_count: usize,
}

impl FunctionUsageReport {
    pub(crate) const fn new(
        entries: Vec<FunctionUsageEntry>,
        formula_count: usize,
        parsed_formula_count: usize,
    ) -> Self {
        Self {
            entries,
            formula_count,
            parsed_formula_count,
        }
    }

    /// Returns entries ordered by normalized function name.
    pub fn entries(&self) -> &[FunctionUsageEntry] {
        &self.entries
    }

    /// Returns the number of formula cells inspected.
    pub const fn formula_count(&self) -> usize {
        self.formula_count
    }

    /// Returns the number of formulas whose syntax could be analyzed.
    pub const fn parsed_formula_count(&self) -> usize {
        self.parsed_formula_count
    }

    /// Returns the number of formulas excluded from usage counts by a parse failure.
    pub const fn unparsed_formula_count(&self) -> usize {
        self.formula_count - self.parsed_formula_count
    }

    /// Returns whether every observed function name has an implemented kernel.
    pub fn is_fully_supported(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.support() == FunctionSupport::Supported)
    }
}

/// How arithmetic treats Excel's narrow near-zero cancellation case.
///
/// Excel corrects some addition and subtraction residues to exactly zero, but preserves others
/// even when the decimal expression cancels exactly. IEEE-754 keeps every such residue. The
/// difference is visible beyond the number itself, because a residue of
/// `5.551115123125783e-17` makes `=(0.1+0.2-0.3)=0` false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ArithmeticSemantics {
    /// Apply Excel's observed relative correction when an exact trace proves cancellation.
    #[default]
    ExcelNearZero,
    /// Return the IEEE-754 result unchanged, as releases up to 0.1.2 did.
    Ieee754,
}

/// How the iterative financial solvers decide they have failed.
///
/// `IRR`, `XIRR`, and `RATE` have no closed form, so their answers depend on the search that
/// produced them. Excel's search method is undocumented; what Microsoft does document is the
/// iteration budget and the convergence tolerance, and that budget is what decides whether a given
/// input yields a number or `#NUM!`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FinancialSolverSemantics {
    /// Stop at the iteration budget and tolerance Microsoft documents per function, so inputs
    /// Excel abandons produce `#NUM!` here too.
    #[default]
    ExcelIterationBudget,
    /// Search longer and converge tighter than Excel, as releases up to 0.1.2 did. Returns a value
    /// for some inputs where Excel returns `#NUM!`.
    ExtendedSearch,
}

/// Deterministic inputs for volatile calculation behavior.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CalculationOptions {
    today_serial: Option<FiniteNumber>,
    now_serial: Option<FiniteNumber>,
    limits: CalculationLimits,
    arithmetic: ArithmeticSemantics,
    financial_solver: FinancialSolverSemantics,
}

impl CalculationOptions {
    /// Supplies the Excel serial date returned by `TODAY()`.
    pub const fn with_today_serial(mut self, today_serial: FiniteNumber) -> Self {
        self.today_serial = Some(today_serial);
        self
    }

    /// Returns the configured deterministic `TODAY()` value, when present.
    pub const fn today_serial(self) -> Option<FiniteNumber> {
        self.today_serial
    }

    /// Supplies the Excel date-time serial returned by `NOW()`.
    pub const fn with_now_serial(mut self, now_serial: FiniteNumber) -> Self {
        self.now_serial = Some(now_serial);
        self
    }

    /// Returns the configured deterministic `NOW()` value, when present.
    pub const fn now_serial(self) -> Option<FiniteNumber> {
        self.now_serial
    }

    /// Replaces the formula calculation resource limits.
    pub const fn with_limits(mut self, limits: CalculationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the configured formula calculation resource limits.
    pub const fn limits(self) -> CalculationLimits {
        self.limits
    }

    /// Selects how a cancelling sum or difference is treated.
    pub const fn with_arithmetic_semantics(mut self, arithmetic: ArithmeticSemantics) -> Self {
        self.arithmetic = arithmetic;
        self
    }

    /// Returns the configured near-zero arithmetic policy.
    pub const fn arithmetic_semantics(self) -> ArithmeticSemantics {
        self.arithmetic
    }

    /// Selects how the iterative financial solvers decide they have failed.
    pub const fn with_financial_solver_semantics(
        mut self,
        financial_solver: FinancialSolverSemantics,
    ) -> Self {
        self.financial_solver = financial_solver;
        self
    }

    /// Returns the configured financial solver policy.
    pub const fn financial_solver_semantics(self) -> FinancialSolverSemantics {
        self.financial_solver
    }
}

/// Result of calculating one formula cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CalculationCellResult {
    /// A typed Excel-compatible value was produced.
    Value(CellValue),
    /// Calculation was deliberately withheld with a structured reason.
    Unavailable(CalculationIssue),
}

/// Why a calculated cell is present in the complete materialization view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MaterializedResultOrigin {
    /// The cell directly stores a formula.
    DirectFormula,
    /// The cell belongs to a declared legacy array formula result region.
    LegacyArray {
        /// Formula cell that owns the array result region.
        anchor: CalculationCellId,
        /// Complete declared result region.
        range: crate::CellRange,
    },
    /// The cell belongs to a calculated dynamic spill result.
    DynamicSpill {
        /// Formula cell that owns the dynamic result region.
        anchor: CalculationCellId,
        /// Complete resolved spill region.
        range: crate::CellRange,
    },
}

/// One typed or unavailable result in the complete calculation materialization view.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedCalculationCell {
    origin: MaterializedResultOrigin,
    result: CalculationCellResult,
}

impl MaterializedCalculationCell {
    pub(crate) const fn new(
        origin: MaterializedResultOrigin,
        result: CalculationCellResult,
    ) -> Self {
        Self { origin, result }
    }

    /// Returns why this cell is present in the materialization view.
    pub const fn origin(&self) -> MaterializedResultOrigin {
        self.origin
    }

    /// Returns the typed value or the calculation issue that prevented one.
    pub const fn result(&self) -> &CalculationCellResult {
        &self.result
    }
}

#[derive(Debug)]
struct CalculationStore<V> {
    cells: PersistentRadixMap<V>,
    len: usize,
    count_result_work: bool,
}

struct CalculationStoreIter<'a, V> {
    inner: PersistentRadixEntries<'a, V>,
    remaining: usize,
}

impl<'a, V> Iterator for CalculationStoreIter<'a, V> {
    type Item = (CalculationCellId, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, value) = self.inner.next()?;
        self.remaining -= 1;
        Some((CalculationStore::<V>::cell_from_key(key), value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<V> ExactSizeIterator for CalculationStoreIter<'_, V> {}

impl<V> CalculationStore<V> {
    fn key(cell: CalculationCellId) -> u128 {
        let row_major_index = u128::from(cell.address().row().get() - 1)
            * u128::from(EXCEL_MAX_COLUMNS)
            + u128::from(cell.address().column().get() - 1);
        (u128::from(cell.sheet_id().get()) << 34) | row_major_index
    }

    fn cell_from_key(key: u128) -> CalculationCellId {
        let sheet = SheetId::new((key >> 34) as u32).expect("stored sheet ID is non-zero");
        let address_key = (key & ((1_u128 << 34) - 1)) as u64;
        let row = address_key / u64::from(EXCEL_MAX_COLUMNS) + 1;
        let column = address_key % u64::from(EXCEL_MAX_COLUMNS) + 1;
        CalculationCellId::new(
            sheet,
            CellAddress::from_indices(row as u32, column as u32)
                .expect("stored calculation cell address is valid"),
        )
    }

    #[cfg(test)]
    fn from_map(values: BTreeMap<CalculationCellId, V>) -> Self {
        Self::from_map_with_result_work(values, false)
    }

    #[cfg(test)]
    fn from_map_with_result_work(
        values: BTreeMap<CalculationCellId, V>,
        count_result_work: bool,
    ) -> Self {
        Self::from_map_with_result_work_cancellable(values, count_result_work, &|| false)
            .expect("non-cancellable calculation-store construction cannot be cancelled")
    }

    fn from_map_with_result_work_cancellable(
        values: BTreeMap<CalculationCellId, V>,
        count_result_work: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let len = values.len();
        let cells = PersistentRadixMap::from_sorted_iter_cancellable(
            values
                .into_iter()
                .map(|(cell, value)| (Self::key(cell), value)),
            cancelled,
        )?;
        Ok(Self {
            cells,
            len,
            count_result_work,
        })
    }

    fn get(&self, cell: &CalculationCellId) -> Option<&V> {
        self.cells.get(Self::key(*cell))
    }

    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        if cancelled() {
            return Err(());
        }
        Ok(Self {
            cells: self.cells.clone(),
            len: self.len,
            count_result_work: self.count_result_work,
        })
    }

    fn insert_cancellable(
        &mut self,
        cell: CalculationCellId,
        value: V,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<PersistentValue<V>>, ()> {
        if cancelled() {
            return Err(());
        }
        let (previous, copied) = self.cells.insert(Self::key(cell), value);
        if self.count_result_work && previous.is_some() {
            work_counter_add(WorkCounter::ResultStoreLeavesRebuilt, 1);
            work_counter_add(WorkCounter::ResultStoreEntriesReindexed, 1);
        }
        if self.count_result_work {
            work_counter_add(WorkCounter::ResultStoreNodesCopied, copied);
        }
        if previous.is_none() {
            self.len += 1;
        }
        Ok(previous)
    }

    fn remove_cancellable(
        &mut self,
        cell: &CalculationCellId,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<PersistentValue<V>>, ()> {
        if cancelled() {
            return Err(());
        }
        if self.cells.get(Self::key(*cell)).is_none() {
            return Ok(None);
        }
        if self.count_result_work {
            work_counter_add(WorkCounter::ResultStoreLeavesRebuilt, 1);
            work_counter_add(WorkCounter::ResultStoreEntriesReindexed, 1);
        }
        let (removed, copied) = self.cells.remove(Self::key(*cell));
        if self.count_result_work {
            work_counter_add(WorkCounter::ResultStoreNodesCopied, copied);
        }
        if removed.is_some() {
            self.len -= 1;
        }
        Ok(removed)
    }

    fn iter(&self) -> CalculationStoreIter<'_, V> {
        CalculationStoreIter {
            inner: self.cells.ordered_entries(),
            remaining: self.len,
        }
    }

    const fn len(&self) -> usize {
        self.len
    }

    const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

fn calculation_store_mut_cancellable<'a, V>(
    store: &'a mut Arc<CalculationStore<V>>,
    cancelled: &impl Fn() -> bool,
) -> Result<&'a mut CalculationStore<V>, ()> {
    if Arc::get_mut(store).is_none() {
        *store = Arc::new(store.clone_cancellable(cancelled)?);
    }
    Ok(Arc::get_mut(store).expect("calculation store was made unique"))
}

#[cfg(test)]
mod calculation_store_tests {
    use super::*;

    #[test]
    fn copy_on_write_shares_payloads_without_deep_clones() {
        let sheet = SheetId::new(1).expect("valid test sheet");
        let first = CalculationCellId::new(
            sheet,
            CellAddress::from_indices(1, 1).expect("valid first address"),
        );
        let second = CalculationCellId::new(
            sheet,
            CellAddress::from_indices(2, 1).expect("valid second address"),
        );
        let mut values = BTreeMap::new();
        values.insert(first, 1_u32);
        values.insert(second, 2_u32);
        let mut store = Arc::new(CalculationStore::from_map(values));
        let installed = Arc::clone(&store);
        work_counter::reset();
        let cancelled = || false;

        let mutable = calculation_store_mut_cancellable(&mut store, &cancelled)
            .expect("outer store clone completes");
        assert_eq!(
            mutable
                .insert_cancellable(first, 3, &cancelled)
                .map(|value| value.as_deref().copied()),
            Ok(Some(1)),
            "insert returns the previous value without deep-cloning it"
        );
        assert_eq!(work_counter::snapshot().deep_cloned_results, 0);
        assert_eq!(installed.get(&first), Some(&1));
        assert_eq!(installed.get(&second), Some(&2));
        assert_eq!(store.get(&first), Some(&3));
        assert_eq!(store.get(&second), Some(&2));
    }

    #[test]
    fn packed_key_preserves_sheet_and_row_major_boundaries_without_collision() {
        let low_sheet = SheetId::new(1).expect("low sheet");
        let high_sheet = SheetId::new(u32::MAX).expect("high sheet");
        let cells = [
            CalculationCellId::new(
                low_sheet,
                CellAddress::from_indices(1, EXCEL_MAX_COLUMNS).expect("wide first row"),
            ),
            CalculationCellId::new(
                low_sheet,
                CellAddress::from_indices(2, 1).expect("second row"),
            ),
            CalculationCellId::new(
                high_sheet,
                CellAddress::from_indices(EXCEL_MAX_ROWS, EXCEL_MAX_COLUMNS).expect("last cell"),
            ),
        ];
        let store = CalculationStore::from_map(
            cells
                .into_iter()
                .enumerate()
                .map(|(index, cell)| (cell, index))
                .collect(),
        );
        assert_eq!(
            store.iter().map(|(cell, _)| cell).collect::<Vec<_>>(),
            cells
        );
        for (index, cell) in cells.into_iter().enumerate() {
            assert_eq!(store.get(&cell), Some(&index));
        }
    }
}

/// Immutable formula results, separate from source literals and saved XLSX values.
#[derive(Debug, Clone)]
pub struct CalculationSnapshot {
    cells: Arc<CalculationStore<CalculationCellResult>>,
    materialized_cells: Arc<CalculationStore<MaterializedCalculationCell>>,
    materialized_cells_by_owner: Arc<CalculationStore<Vec<CalculationCellId>>>,
    numeric_decimal_traces: Arc<CalculationStore<DecimalTrace>>,
    options: CalculationOptions,
    provenance: Provenance,
    source_revision: u64,
    source_fingerprint: [u8; 32],
}

pub(crate) struct IncrementalCalculationPatch<'a> {
    dirty: &'a BTreeSet<CalculationCellId>,
    cell_results: BTreeMap<CalculationCellId, CalculationCellResult>,
    materialized_results: BTreeMap<CalculationCellId, MaterializedCalculationCell>,
    decimal_traces: BTreeMap<CalculationCellId, DecimalTrace>,
    source: &'a WorkbookSnapshot,
    options: CalculationOptions,
}

impl<'a> IncrementalCalculationPatch<'a> {
    pub(crate) const fn new(
        dirty: &'a BTreeSet<CalculationCellId>,
        cell_results: BTreeMap<CalculationCellId, CalculationCellResult>,
        materialized_results: BTreeMap<CalculationCellId, MaterializedCalculationCell>,
        decimal_traces: BTreeMap<CalculationCellId, DecimalTrace>,
        source: &'a WorkbookSnapshot,
        options: CalculationOptions,
    ) -> Self {
        Self {
            dirty,
            cell_results,
            materialized_results,
            decimal_traces,
            source,
            options,
        }
    }
}

impl CalculationSnapshot {
    #[cfg(test)]
    pub(crate) fn new(
        cells: BTreeMap<CalculationCellId, CalculationCellResult>,
        materialized_cells: BTreeMap<CalculationCellId, MaterializedCalculationCell>,
        numeric_decimal_traces: BTreeMap<CalculationCellId, DecimalTrace>,
        source: &WorkbookSnapshot,
        options: CalculationOptions,
    ) -> Self {
        Self::new_cancellable(
            cells,
            materialized_cells,
            numeric_decimal_traces,
            source,
            options,
            &|| false,
        )
        .expect("non-cancellable snapshot construction cannot be cancelled")
    }

    pub(crate) fn new_cancellable(
        cells: BTreeMap<CalculationCellId, CalculationCellResult>,
        materialized_cells: BTreeMap<CalculationCellId, MaterializedCalculationCell>,
        numeric_decimal_traces: BTreeMap<CalculationCellId, DecimalTrace>,
        source: &WorkbookSnapshot,
        options: CalculationOptions,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let provenance = Provenance::new(
            ProviderIdentity::calculator(),
            source.provenance().input_hash(),
        );
        let materialized_cells_by_owner =
            build_materialized_owner_index(&materialized_cells, cancelled)?;
        Ok(Self {
            cells: Arc::new(CalculationStore::from_map_with_result_work_cancellable(
                cells, true, cancelled,
            )?),
            materialized_cells: Arc::new(CalculationStore::from_map_with_result_work_cancellable(
                materialized_cells,
                false,
                cancelled,
            )?),
            materialized_cells_by_owner: Arc::new(
                CalculationStore::from_map_with_result_work_cancellable(
                    materialized_cells_by_owner,
                    false,
                    cancelled,
                )?,
            ),
            numeric_decimal_traces: Arc::new(
                CalculationStore::from_map_with_result_work_cancellable(
                    numeric_decimal_traces,
                    false,
                    cancelled,
                )?,
            ),
            options,
            provenance,
            source_revision: source.semantic_revision(),
            source_fingerprint: source.semantic_fingerprint_cancellable(cancelled)?,
        })
    }

    pub(crate) fn rebase_source_cancellable(
        &self,
        source: &WorkbookSnapshot,
        options: CalculationOptions,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        if cancelled() {
            return Err(());
        }
        Ok(Self {
            cells: Arc::clone(&self.cells),
            materialized_cells: Arc::clone(&self.materialized_cells),
            materialized_cells_by_owner: Arc::clone(&self.materialized_cells_by_owner),
            numeric_decimal_traces: Arc::clone(&self.numeric_decimal_traces),
            options,
            provenance: Provenance::new(
                ProviderIdentity::calculator(),
                source.provenance().input_hash(),
            ),
            source_revision: source.semantic_revision(),
            source_fingerprint: source.semantic_fingerprint_cancellable(cancelled)?,
        })
    }

    pub(crate) fn apply_incremental_patch_cancellable(
        &self,
        patch: IncrementalCalculationPatch<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, ()> {
        let IncrementalCalculationPatch {
            dirty,
            cell_results,
            materialized_results,
            decimal_traces,
            source,
            options,
        } = patch;
        if cancelled() {
            return Err(());
        }
        let mut cells = Arc::clone(&self.cells);
        let mut materialized_cells = Arc::clone(&self.materialized_cells);
        let mut materialized_cells_by_owner = Arc::clone(&self.materialized_cells_by_owner);
        let mut numeric_decimal_traces = Arc::clone(&self.numeric_decimal_traces);

        let cells_mut = calculation_store_mut_cancellable(&mut cells, cancelled)?;
        for (cell, result) in cell_results {
            if cancelled() {
                return Err(());
            }
            cells_mut.insert_cancellable(cell, result, cancelled)?;
        }

        let owners_mut =
            calculation_store_mut_cancellable(&mut materialized_cells_by_owner, cancelled)?;
        let mut removed = Vec::new();
        for owner in dirty {
            if let Some(cells) = owners_mut.remove_cancellable(owner, cancelled)? {
                removed.extend(cells.iter().copied());
            }
        }
        let materialized_mut =
            calculation_store_mut_cancellable(&mut materialized_cells, cancelled)?;
        for cell in &removed {
            if cancelled() {
                return Err(());
            }
            materialized_mut.remove_cancellable(cell, cancelled)?;
        }
        let mut added_by_owner = BTreeMap::<CalculationCellId, Vec<CalculationCellId>>::new();
        for (cell, result) in materialized_results {
            if cancelled() {
                return Err(());
            }
            let owner = materialized_owner(cell, &result);
            added_by_owner.entry(owner).or_default().push(cell);
            materialized_mut.insert_cancellable(cell, result, cancelled)?;
        }
        for (owner, mut cells) in added_by_owner {
            cells.sort_unstable();
            owners_mut.insert_cancellable(owner, cells, cancelled)?;
        }

        let traces_mut = calculation_store_mut_cancellable(&mut numeric_decimal_traces, cancelled)?;
        for cell in removed {
            traces_mut.remove_cancellable(&cell, cancelled)?;
        }
        for (cell, trace) in decimal_traces {
            if cancelled() {
                return Err(());
            }
            traces_mut.insert_cancellable(cell, trace, cancelled)?;
        }

        Ok(Self {
            cells,
            materialized_cells,
            materialized_cells_by_owner,
            numeric_decimal_traces,
            options,
            provenance: Provenance::new(
                ProviderIdentity::calculator(),
                source.provenance().input_hash(),
            ),
            source_revision: source.semantic_revision(),
            source_fingerprint: source.semantic_fingerprint_cancellable(cancelled)?,
        })
    }

    /// Returns one calculated formula result.
    pub fn cell(&self, cell: CalculationCellId) -> Option<&CalculationCellResult> {
        self.cells.get(&cell)
    }

    /// Iterates formula results in sheet-ID and row-major address order.
    pub fn cells(
        &self,
    ) -> impl ExactSizeIterator<Item = (CalculationCellId, &CalculationCellResult)> {
        self.cells.iter()
    }

    /// Returns one result from the complete formula and array materialization view.
    pub fn materialized_cell(
        &self,
        cell: CalculationCellId,
    ) -> Option<&MaterializedCalculationCell> {
        self.materialized_cells.get(&cell)
    }

    pub(in crate::calculation) fn materialized_cells_owned_by(
        &self,
        owner: CalculationCellId,
    ) -> &[CalculationCellId] {
        self.materialized_cells_by_owner
            .get(&owner)
            .map_or(&[], Vec::as_slice)
    }

    /// Iterates the complete materialization view in sheet-ID and row-major address order.
    pub fn materialized_cells(
        &self,
    ) -> impl ExactSizeIterator<Item = (CalculationCellId, &MaterializedCalculationCell)> {
        self.materialized_cells.iter()
    }

    pub(in crate::calculation) fn numeric_decimal_trace(
        &self,
        cell: CalculationCellId,
    ) -> Option<DecimalTrace> {
        self.numeric_decimal_traces.get(&cell).copied()
    }

    /// Returns the number of formula results.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Returns whether no formula results are present.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Returns the deterministic policy inputs used for this calculation.
    pub const fn options(&self) -> CalculationOptions {
        self.options
    }

    /// Returns deterministic calculation provenance.
    pub const fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// Returns the workbook semantic revision used to produce this result.
    pub const fn source_revision(&self) -> u64 {
        self.source_revision
    }

    pub(crate) const fn source_fingerprint(&self) -> &[u8; 32] {
        &self.source_fingerprint
    }

    pub(crate) fn matches_workbook(&self, workbook: &WorkbookSnapshot) -> bool {
        self.source_revision == workbook.semantic_revision()
            && self.source_fingerprint
                == workbook
                    .semantic_fingerprint_cancellable(&|| false)
                    .expect("non-cancellable fingerprinting cannot be cancelled")
            && self.provenance.input_hash() == workbook.provenance().input_hash()
    }
}

fn materialized_owner(
    cell: CalculationCellId,
    materialized: &MaterializedCalculationCell,
) -> CalculationCellId {
    match materialized.origin() {
        MaterializedResultOrigin::DirectFormula => cell,
        MaterializedResultOrigin::LegacyArray { anchor, .. }
        | MaterializedResultOrigin::DynamicSpill { anchor, .. } => anchor,
    }
}

fn build_materialized_owner_index(
    materialized_cells: &BTreeMap<CalculationCellId, MaterializedCalculationCell>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeMap<CalculationCellId, Vec<CalculationCellId>>, ()> {
    let mut owners = BTreeMap::<CalculationCellId, Vec<CalculationCellId>>::new();
    for (cell, materialized) in materialized_cells {
        if cancelled() {
            return Err(());
        }
        owners
            .entry(materialized_owner(*cell, materialized))
            .or_default()
            .push(*cell);
    }
    Ok(owners)
}

pub(crate) use identity::workbook_fingerprint;

/// Scans formula grammar and function-surface support without returning calculated values.
pub fn scan_formula_capabilities(workbook: &WorkbookSnapshot) -> FormulaCapabilityReport {
    scan_formula_capabilities_with_options(workbook, CalculationOptions::default())
}

/// Scans formula support under caller-provided deterministic parse, name, and dependency limits.
///
/// Evaluation-only limits can still produce issues during calculation.
pub fn scan_formula_capabilities_with_options(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> FormulaCapabilityReport {
    pipeline::scan_formula_capabilities(workbook, options)
}

/// Returns the deterministic catalog of function names implemented by this build.
///
/// The catalog includes official legacy aliases and the internal Google-export compatibility
/// function, which is marked as non-official.
pub fn supported_function_catalog() -> Vec<FunctionCatalogEntry> {
    functions::function_catalog()
}

/// Counts normalized function demand using default calculation limits.
pub fn scan_function_usage(workbook: &WorkbookSnapshot) -> FunctionUsageReport {
    scan_function_usage_with_options(workbook, CalculationOptions::default())
}

/// Counts normalized function demand using caller-provided calculation limits.
pub fn scan_function_usage_with_options(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> FunctionUsageReport {
    pipeline::scan_function_usage(workbook, options)
}

/// Calculates formulas without mutating the source snapshot and records runtime issues per cell.
pub fn calculate_workbook(
    workbook: &WorkbookSnapshot,
    options: CalculationOptions,
) -> CalculationSnapshot {
    pipeline::calculate_workbook(workbook, options)
}
