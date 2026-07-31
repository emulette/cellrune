use std::error::Error;
use std::fmt;

use crate::{CellRange, FormulaText, SheetId, WorkbookSnapshot};

use super::{CalculationOptions, CancellationToken};

mod analyzer;
#[cfg(test)]
mod tests;

const MESSAGE_ZERO_LIMIT: &str = "defined-name analysis limit must be greater than zero";
const MESSAGE_UNKNOWN_SHEET: &str = "current sheet does not exist in the workbook";
const MESSAGE_RESOURCE_LIMIT: &str = "defined-name analysis exceeded a configured resource limit";
const MESSAGE_CANCELLED: &str = "defined-name analysis was cancelled";

/// Stable workbook-order identity of a continuous 3-D sheet span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefinedNameSheetSpan {
    start: SheetId,
    end: SheetId,
}

impl DefinedNameSheetSpan {
    pub(super) const fn new(start: SheetId, end: SheetId) -> Self {
        Self { start, end }
    }

    /// Returns the first sheet in workbook order.
    pub const fn start(self) -> SheetId {
        self.start
    }

    /// Returns the last sheet in workbook order.
    pub const fn end(self) -> SheetId {
        self.end
    }
}

/// One area in an ordered non-rectangular defined-name reference.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DefinedNameReferenceArea {
    /// One rectangle on one worksheet.
    Rectangular {
        /// Stable worksheet identity.
        sheet_id: SheetId,
        /// Resolved A1 rectangle.
        range: CellRange,
    },
    /// One rectangle repeated across a continuous worksheet span.
    ThreeDimensional {
        /// Stable workbook-order sheet span.
        sheet_span: DefinedNameSheetSpan,
        /// Resolved A1 rectangle shared by each sheet.
        range: CellRange,
    },
}

/// Dynamic reference construct that determines a name's reference shape at calculation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefinedNameDynamicKind {
    /// The terminal reference expression is `OFFSET`.
    Offset,
    /// The terminal reference expression is `INDIRECT`.
    Indirect,
    /// The terminal reference expression is a spill reference.
    Spill,
    /// Multiple dynamic reference constructs contribute to the result.
    Mixed,
}

impl DefinedNameDynamicKind {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offset => "offset",
            Self::Indirect => "indirect",
            Self::Spill => "spill",
            Self::Mixed => "mixed",
        }
    }
}

/// Typed target category of an external-workbook reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefinedNameExternalTargetKind {
    /// A cell, area, whole-row, or whole-column reference.
    Reference,
    /// An external defined name.
    DefinedName,
    /// An external structured table reference.
    StructuredReference,
}

impl DefinedNameExternalTargetKind {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::DefinedName => "defined_name",
            Self::StructuredReference => "structured_reference",
        }
    }
}

/// Typed detail retained from an external-workbook reference node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedNameExternalReference {
    locator: Option<Box<str>>,
    workbook: Box<str>,
    sheet: Option<Box<str>>,
    sheet_end: Option<Box<str>>,
    target: DefinedNameExternalTargetKind,
    target_text: Box<str>,
}

impl DefinedNameExternalReference {
    pub(super) fn new(
        locator: Option<Box<str>>,
        workbook: Box<str>,
        sheet: Option<Box<str>>,
        sheet_end: Option<Box<str>>,
        target: DefinedNameExternalTargetKind,
        target_text: Box<str>,
    ) -> Self {
        Self {
            locator,
            workbook,
            sheet,
            sheet_end,
            target,
            target_text,
        }
    }

    /// Returns the optional path or URI prefix before the bracketed workbook token.
    pub fn locator(&self) -> Option<&str> {
        self.locator.as_deref()
    }

    /// Returns the external workbook token without its surrounding brackets or locator.
    pub fn workbook(&self) -> &str {
        &self.workbook
    }

    /// Returns the optional first external sheet token.
    pub fn sheet(&self) -> Option<&str> {
        self.sheet.as_deref()
    }

    /// Returns the optional final external sheet token of a 3-D prefix.
    pub fn sheet_end(&self) -> Option<&str> {
        self.sheet_end.as_deref()
    }

    /// Returns the typed external target category.
    pub const fn target(&self) -> DefinedNameExternalTargetKind {
        self.target
    }

    /// Returns the canonical external target text without its workbook or sheet prefix.
    pub fn target_text(&self) -> &str {
        &self.target_text
    }
}

/// Reason a reachable defined-name formula is invalid against the immutable workbook snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefinedNameInvalidReason {
    /// The selected or reachable formula does not parse.
    ParseError,
    /// A non-callable value-name chain contains a cycle.
    CircularReference,
    /// A reachable name is absent from its applicable scope chain.
    UnresolvedName,
    /// A static reference names an absent sheet, table, column, or invalid range.
    InvalidReference,
}

impl DefinedNameInvalidReason {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParseError => "parse_error",
            Self::CircularReference => "circular_reference",
            Self::UnresolvedName => "unresolved_name",
            Self::InvalidReference => "invalid_reference",
        }
    }
}

/// Reason a valid formula cannot be represented as static reference geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefinedNameUnsupportedReason {
    /// The formula is a callable or general non-reference expression.
    NonReferenceExpression,
    /// The result needs a current cell, calculated value, or other runtime state.
    ContextDependent,
    /// The typed AST is valid but outside the current inspection resolver.
    UnsupportedExpression,
}

impl DefinedNameUnsupportedReason {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonReferenceExpression => "non_reference_expression",
            Self::ContextDependent => "context_dependent",
            Self::UnsupportedExpression => "unsupported_expression",
        }
    }
}

/// Typed analysis of one workbook or sheet-local defined name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DefinedNameAnalysis {
    /// The name resolves to one rectangle on one worksheet.
    Rectangular {
        /// Stable worksheet identity.
        sheet_id: SheetId,
        /// Resolved A1 rectangle.
        range: CellRange,
    },
    /// The name resolves to one rectangle across a continuous worksheet span.
    ThreeDimensional {
        /// Stable workbook-order sheet span.
        sheet_span: DefinedNameSheetSpan,
        /// Resolved A1 rectangle shared by each sheet.
        range: CellRange,
    },
    /// The name resolves to multiple ordered areas.
    NonRectangular {
        /// Ordered areas with source multiplicity preserved.
        areas: Vec<DefinedNameReferenceArea>,
    },
    /// The name resolves to a valid empty reference such as a table without data rows.
    EmptyReference,
    /// The terminal reference shape depends on calculation state.
    DynamicFormula {
        /// Dynamic construct classification.
        kind: DefinedNameDynamicKind,
        /// Formula of the terminal dynamic definition.
        formula: FormulaText,
    },
    /// The terminal definition is dependency-free constant syntax.
    Constant {
        /// Formula of the terminal constant definition.
        formula: FormulaText,
    },
    /// The typed syntax addresses another workbook.
    ExternalReference {
        /// Typed external target detail.
        detail: DefinedNameExternalReference,
    },
    /// The selected name or a reachable value-name definition is invalid.
    Invalid {
        /// Stable invalidity reason.
        reason: DefinedNameInvalidReason,
        /// Optional source-specific context.
        detail: Option<Box<str>>,
    },
    /// The formula is valid but cannot be represented as static reference geometry.
    Unsupported {
        /// Stable unsupported reason.
        reason: DefinedNameUnsupportedReason,
        /// Optional source-specific context.
        detail: Option<Box<str>>,
    },
    /// No root name exists in the selected sheet-local/workbook lookup chain.
    NotFound,
}

/// Resource unit enforced by defined-name analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefinedNameAnalysisLimitKind {
    /// Per-formula token count.
    FormulaTokens,
    /// Per-formula UTF-8 source bytes.
    FormulaSourceBytes,
    /// Per-formula AST nodes.
    FormulaAstNodes,
    /// Per-formula AST nesting depth.
    FormulaNestingDepth,
    /// Simultaneously active value-name definitions.
    NameChainDepth,
    /// Cumulative AST nodes scanned across reachable definitions.
    ScanNodes,
    /// Ordered areas retained by the analysis result.
    ReferenceAreas,
    /// Pairwise area operations used by intersection.
    FunctionIterations,
}

impl DefinedNameAnalysisLimitKind {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FormulaTokens => "formula_tokens",
            Self::FormulaSourceBytes => "formula_source_bytes",
            Self::FormulaAstNodes => "formula_ast_nodes",
            Self::FormulaNestingDepth => "formula_nesting_depth",
            Self::NameChainDepth => "name_chain_depth",
            Self::ScanNodes => "scan_nodes",
            Self::ReferenceAreas => "reference_areas",
            Self::FunctionIterations => "function_iterations",
        }
    }
}

/// Stable execution-failure category for a defined-name query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DefinedNameAnalysisErrorKind {
    /// The requested current sheet is absent.
    UnknownCurrentSheet,
    /// Analysis exceeded a configured resource limit.
    ResourceLimit,
    /// Cooperative cancellation was requested.
    Cancelled,
}

impl DefinedNameAnalysisErrorKind {
    /// Returns the stable dotted error code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCurrentSheet => "defined_name.unknown_current_sheet",
            Self::ResourceLimit => "defined_name.resource_limit",
            Self::Cancelled => "defined_name.cancelled",
        }
    }
}

/// Execution failure returned separately from a semantic analysis result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedNameAnalysisError {
    kind: DefinedNameAnalysisErrorKind,
    limit: Option<DefinedNameAnalysisLimitKind>,
    detail: Option<Box<str>>,
}

impl DefinedNameAnalysisError {
    pub(super) fn unknown_sheet(sheet_id: SheetId) -> Self {
        Self {
            kind: DefinedNameAnalysisErrorKind::UnknownCurrentSheet,
            limit: None,
            detail: Some(sheet_id.get().to_string().into_boxed_str()),
        }
    }

    pub(super) fn resource(limit: DefinedNameAnalysisLimitKind) -> Self {
        Self {
            kind: DefinedNameAnalysisErrorKind::ResourceLimit,
            limit: Some(limit),
            detail: Some(limit.as_str().into()),
        }
    }

    pub(super) fn cancelled() -> Self {
        Self {
            kind: DefinedNameAnalysisErrorKind::Cancelled,
            limit: None,
            detail: None,
        }
    }

    /// Returns the stable execution-failure category.
    pub const fn kind(&self) -> DefinedNameAnalysisErrorKind {
        self.kind
    }

    /// Returns the exhausted resource unit, when applicable.
    pub const fn limit(&self) -> Option<DefinedNameAnalysisLimitKind> {
        self.limit
    }

    /// Returns source-specific context such as the unknown sheet ID.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Returns the shared human-readable message.
    pub const fn message(&self) -> &'static str {
        match self.kind {
            DefinedNameAnalysisErrorKind::UnknownCurrentSheet => MESSAGE_UNKNOWN_SHEET,
            DefinedNameAnalysisErrorKind::ResourceLimit => MESSAGE_RESOURCE_LIMIT,
            DefinedNameAnalysisErrorKind::Cancelled => MESSAGE_CANCELLED,
        }
    }
}

impl fmt::Display for DefinedNameAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl Error for DefinedNameAnalysisError {}

/// Bounded options for a defined-name inspection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefinedNameAnalysisOptions {
    calculation: CalculationOptions,
    max_name_chain_depth: u64,
    max_scan_nodes: u64,
}

impl DefinedNameAnalysisOptions {
    /// Creates options that reuse the supplied formula and reference limits.
    pub const fn new(calculation: CalculationOptions) -> Self {
        Self {
            calculation,
            max_name_chain_depth: 256,
            max_scan_nodes: 65_536,
        }
    }

    /// Replaces the maximum simultaneously active value-name chain depth.
    ///
    /// # Errors
    ///
    /// Returns [`DefinedNameAnalysisOptionsError`] when `value` is zero.
    pub fn with_max_name_chain_depth(
        mut self,
        value: u64,
    ) -> Result<Self, DefinedNameAnalysisOptionsError> {
        if value == 0 {
            return Err(DefinedNameAnalysisOptionsError);
        }
        self.max_name_chain_depth = value;
        Ok(self)
    }

    /// Replaces the cumulative reachable AST-node scan budget.
    ///
    /// # Errors
    ///
    /// Returns [`DefinedNameAnalysisOptionsError`] when `value` is zero.
    pub fn with_max_scan_nodes(
        mut self,
        value: u64,
    ) -> Result<Self, DefinedNameAnalysisOptionsError> {
        if value == 0 {
            return Err(DefinedNameAnalysisOptionsError);
        }
        self.max_scan_nodes = value;
        Ok(self)
    }

    /// Returns calculation/parser limits reused by the analyzer.
    pub const fn calculation(self) -> CalculationOptions {
        self.calculation
    }

    /// Returns the value-name chain depth limit.
    pub const fn max_name_chain_depth(self) -> u64 {
        self.max_name_chain_depth
    }

    /// Returns the cumulative reachable AST-node scan limit.
    pub const fn max_scan_nodes(self) -> u64 {
        self.max_scan_nodes
    }
}

impl Default for DefinedNameAnalysisOptions {
    fn default() -> Self {
        Self::new(CalculationOptions::default())
    }
}

/// Error returned when a defined-name analysis limit is zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefinedNameAnalysisOptionsError;

impl fmt::Display for DefinedNameAnalysisOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(MESSAGE_ZERO_LIMIT)
    }
}

impl Error for DefinedNameAnalysisOptionsError {}

/// Analyzes one defined name with default bounded options.
///
/// # Errors
///
/// Returns a typed error for an unknown current sheet, exhausted resource budget, or cancellation.
pub fn analyze_defined_name(
    workbook: &WorkbookSnapshot,
    name: &str,
    current_sheet: Option<SheetId>,
) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> {
    analyzer::analyze(
        workbook,
        name,
        current_sheet,
        DefinedNameAnalysisOptions::default(),
        &|| false,
    )
}

/// Analyzes one defined name with explicit bounded options.
///
/// # Errors
///
/// Returns a typed error for an unknown current sheet or exhausted resource budget.
pub fn analyze_defined_name_with_options(
    workbook: &WorkbookSnapshot,
    name: &str,
    current_sheet: Option<SheetId>,
    options: DefinedNameAnalysisOptions,
) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> {
    analyzer::analyze(workbook, name, current_sheet, options, &|| false)
}

/// Analyzes one defined name with explicit options and cooperative cancellation.
///
/// # Errors
///
/// Returns a typed error for an unknown current sheet, exhausted resource budget, or cancellation.
pub fn analyze_defined_name_cancellable(
    workbook: &WorkbookSnapshot,
    name: &str,
    current_sheet: Option<SheetId>,
    options: DefinedNameAnalysisOptions,
    cancellation: &CancellationToken,
) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> {
    analyzer::analyze(workbook, name, current_sheet, options, &|| {
        cancellation.is_cancelled()
    })
}
