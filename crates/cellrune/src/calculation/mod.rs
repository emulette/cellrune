//! Deterministic workbook formula capability scanning and calculation.

use std::collections::BTreeMap;

use crate::{
    CellAddress, CellValue, FiniteNumber, Provenance, ProviderIdentity, SheetId, WorkbookSnapshot,
};
use decimal::DecimalTrace;

mod ast;
mod coerce;
mod convert;
mod criteria;
mod decimal;
mod error;
mod eval;
mod functions;
mod graph;
mod identity;
mod lambda;
mod lexer;
mod limits;
mod operators;
mod parser;
mod pipeline;
mod runtime;
mod scope;
mod session;
mod sheet_span;
mod textfmt;
mod value;

use error::{
    MESSAGE_BLOCKED_BY_UPSTREAM, MESSAGE_CIRCULAR_REFERENCE, MESSAGE_MISSING_FORMULA_TEXT,
    MESSAGE_PARSE_ERROR, MESSAGE_RESOURCE_LIMIT_EXCEEDED, MESSAGE_UNSUPPORTED_EXPRESSION,
    MESSAGE_UNSUPPORTED_FUNCTION, MESSAGE_UNSUPPORTED_NAME, MESSAGE_UNSUPPORTED_SHEET_RANGE,
    MESSAGE_UNSUPPORTED_STRUCTURED_REFERENCE, MESSAGE_VOLATILE_INPUT_MISSING,
};

pub(super) use crate::{EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};
pub(crate) use error::{
    ERROR_LEX_UNEXPECTED_CHARACTER, ERROR_LEX_UNKNOWN_ERROR_LITERAL,
    ERROR_LEX_UNTERMINATED_SHEET_NAME, ERROR_LEX_UNTERMINATED_STRING,
    ERROR_PARSE_INVALID_REFERENCE, ERROR_PARSE_MISMATCHED_RANGE, ERROR_PARSE_UNEXPECTED_END,
    ERROR_PARSE_UNEXPECTED_TOKEN,
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
    /// A structured table reference, for example `Table1[Amount]`, is recognized but not
    /// yet resolved.
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

/// Immutable formula results, separate from source literals and saved XLSX values.
#[derive(Debug, Clone)]
pub struct CalculationSnapshot {
    cells: BTreeMap<CalculationCellId, CalculationCellResult>,
    materialized_cells: BTreeMap<CalculationCellId, MaterializedCalculationCell>,
    numeric_decimal_traces: BTreeMap<CalculationCellId, DecimalTrace>,
    options: CalculationOptions,
    provenance: Provenance,
    source_revision: u64,
    source_fingerprint: [u8; 32],
}

impl CalculationSnapshot {
    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        if cancelled() {
            return Err(());
        }
        Ok(Self {
            cells: eval::clone_map_cancellable(&self.cells, cancelled)?,
            materialized_cells: eval::clone_map_cancellable(&self.materialized_cells, cancelled)?,
            numeric_decimal_traces: eval::clone_map_cancellable(
                &self.numeric_decimal_traces,
                cancelled,
            )?,
            options: self.options,
            provenance: self.provenance.clone(),
            source_revision: self.source_revision,
            source_fingerprint: self.source_fingerprint,
        })
    }

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
        Ok(Self {
            cells,
            materialized_cells,
            numeric_decimal_traces,
            options,
            provenance,
            source_revision: source.semantic_revision(),
            source_fingerprint: identity::workbook_fingerprint_cancellable(source, cancelled)?,
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
        self.cells.iter().map(|(cell, result)| (*cell, result))
    }

    /// Returns one result from the complete formula and array materialization view.
    pub fn materialized_cell(
        &self,
        cell: CalculationCellId,
    ) -> Option<&MaterializedCalculationCell> {
        self.materialized_cells.get(&cell)
    }

    /// Iterates the complete materialization view in sheet-ID and row-major address order.
    pub fn materialized_cells(
        &self,
    ) -> impl ExactSizeIterator<Item = (CalculationCellId, &MaterializedCalculationCell)> {
        self.materialized_cells
            .iter()
            .map(|(cell, result)| (*cell, result))
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
            && self.source_fingerprint == identity::workbook_fingerprint(workbook)
            && self.provenance.input_hash() == workbook.provenance().input_hash()
    }
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
