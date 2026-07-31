use std::error::Error;
use std::fmt;

const MAX_FORMULA_TOKENS: &str = "max_formula_tokens";
const MAX_FORMULA_SOURCE_BYTES: &str = "max_formula_source_bytes";
const MAX_FORMULA_AST_NODES: &str = "max_formula_ast_nodes";
const MAX_FORMULA_NESTING_DEPTH: &str = "max_formula_nesting_depth";
const MAX_DEPENDENCY_EDGES: &str = "max_dependency_edges";
const MAX_ARRAY_CELLS: &str = "max_array_cells";
const MAX_TEXT_BYTES: &str = "max_text_bytes";
const MAX_FUNCTION_ITERATIONS: &str = "max_function_iterations";
const MAX_LET_BINDINGS: &str = "max_let_bindings";
const MAX_LAMBDA_DEPTH: &str = "max_lambda_depth";
const MAX_LAMBDA_INVOCATIONS: &str = "max_lambda_invocations";
const MESSAGE_ZERO_LIMIT: &str = "calculation limit must be greater than zero";
pub(super) const SAFE_FORMULA_NESTING_DEPTH: u64 = 256;

/// Resource limits applied while formulas are parsed, scheduled, and evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalculationLimits {
    max_formula_tokens: u64,
    max_formula_source_bytes: u64,
    max_formula_ast_nodes: u64,
    max_formula_nesting_depth: u64,
    max_dependency_edges: u64,
    max_array_cells: u64,
    max_text_bytes: u64,
    max_function_iterations: u64,
    max_let_bindings: u64,
    max_lambda_depth: u64,
    max_lambda_invocations: u64,
}

impl CalculationLimits {
    /// Returns the maximum lexical token count of one formula.
    pub const fn max_formula_tokens(self) -> u64 {
        self.max_formula_tokens
    }

    /// Returns the maximum UTF-8 byte length of one formula source.
    pub const fn max_formula_source_bytes(self) -> u64 {
        self.max_formula_source_bytes
    }

    /// Returns the maximum AST node count of one formula.
    pub const fn max_formula_ast_nodes(self) -> u64 {
        self.max_formula_ast_nodes
    }

    /// Returns the caller-configured maximum AST nesting depth of one formula.
    ///
    /// The parser can apply a lower implementation safety ceiling while preserving this configured
    /// value for API compatibility.
    pub const fn max_formula_nesting_depth(self) -> u64 {
        self.max_formula_nesting_depth
    }

    /// Returns the maximum formula dependency-edge count in one workbook calculation.
    pub const fn max_dependency_edges(self) -> u64 {
        self.max_dependency_edges
    }

    /// Returns the maximum number of cells materialized or traversed by one array operation.
    pub const fn max_array_cells(self) -> u64 {
        self.max_array_cells
    }

    /// Returns the maximum UTF-8 byte length of one calculated text value.
    pub const fn max_text_bytes(self) -> u64 {
        self.max_text_bytes
    }

    /// Returns the maximum data-dependent loop iterations of one function call.
    pub const fn max_function_iterations(self) -> u64 {
        self.max_function_iterations
    }

    /// Returns the maximum number of name/value pairs in one `LET` expression.
    pub const fn max_let_bindings(self) -> u64 {
        self.max_let_bindings
    }

    /// Returns the maximum simultaneously active lambda-body depth in one cell evaluation.
    pub const fn max_lambda_depth(self) -> u64 {
        self.max_lambda_depth
    }

    /// Returns the maximum cumulative lambda-body invocations in one cell evaluation.
    pub const fn max_lambda_invocations(self) -> u64 {
        self.max_lambda_invocations
    }

    /// Replaces the per-formula lexical token limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_formula_tokens(mut self, value: u64) -> Result<Self, CalculationOptionsError> {
        self.max_formula_tokens = nonzero(MAX_FORMULA_TOKENS, value)?;
        Ok(self)
    }

    /// Replaces the per-formula UTF-8 source byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_formula_source_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, CalculationOptionsError> {
        self.max_formula_source_bytes = nonzero(MAX_FORMULA_SOURCE_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the per-formula AST node limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_formula_ast_nodes(
        mut self,
        value: u64,
    ) -> Result<Self, CalculationOptionsError> {
        self.max_formula_ast_nodes = nonzero(MAX_FORMULA_AST_NODES, value)?;
        Ok(self)
    }

    /// Replaces the caller-configured per-formula AST nesting-depth limit.
    ///
    /// Values above the parser's implementation safety ceiling remain accepted for compatibility;
    /// parsing still fails closed at that ceiling instead of constructing an unsafe recursive AST.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_formula_nesting_depth(
        mut self,
        value: u64,
    ) -> Result<Self, CalculationOptionsError> {
        self.max_formula_nesting_depth = nonzero(MAX_FORMULA_NESTING_DEPTH, value)?;
        Ok(self)
    }

    /// Replaces the workbook-wide dependency-edge limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_dependency_edges(
        mut self,
        value: u64,
    ) -> Result<Self, CalculationOptionsError> {
        self.max_dependency_edges = nonzero(MAX_DEPENDENCY_EDGES, value)?;
        Ok(self)
    }

    /// Replaces the per-operation array-cell limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_array_cells(mut self, value: u64) -> Result<Self, CalculationOptionsError> {
        self.max_array_cells = nonzero(MAX_ARRAY_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the calculated text-value byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_text_bytes(mut self, value: u64) -> Result<Self, CalculationOptionsError> {
        self.max_text_bytes = nonzero(MAX_TEXT_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the per-function data-dependent iteration limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_function_iterations(
        mut self,
        value: u64,
    ) -> Result<Self, CalculationOptionsError> {
        self.max_function_iterations = nonzero(MAX_FUNCTION_ITERATIONS, value)?;
        Ok(self)
    }

    /// Replaces the name/value pair limit for one `LET` expression.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_let_bindings(mut self, value: u64) -> Result<Self, CalculationOptionsError> {
        self.max_let_bindings = nonzero(MAX_LET_BINDINGS, value)?;
        Ok(self)
    }

    /// Replaces the active lambda-body depth limit for one cell evaluation.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_lambda_depth(mut self, value: u64) -> Result<Self, CalculationOptionsError> {
        self.max_lambda_depth = nonzero(MAX_LAMBDA_DEPTH, value)?;
        Ok(self)
    }

    /// Replaces the cumulative lambda-body invocation limit for one cell evaluation.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_lambda_invocations(
        mut self,
        value: u64,
    ) -> Result<Self, CalculationOptionsError> {
        self.max_lambda_invocations = nonzero(MAX_LAMBDA_INVOCATIONS, value)?;
        Ok(self)
    }
}

impl Default for CalculationLimits {
    fn default() -> Self {
        Self {
            max_formula_tokens: 8_192,
            max_formula_source_bytes: 1024 * 1024,
            max_formula_ast_nodes: 8_192,
            max_formula_nesting_depth: 256,
            max_dependency_edges: 10_000_000,
            max_array_cells: 1_000_000,
            max_text_bytes: 32_767,
            max_function_iterations: 1_000_000,
            max_let_bindings: 126,
            max_lambda_depth: 256,
            max_lambda_invocations: 1_000_000,
        }
    }
}

/// Invalid caller-provided calculation configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CalculationOptionsError {
    /// A resource limit was set to zero.
    ZeroLimit {
        /// Stable name of the limit that was set to zero.
        name: &'static str,
    },
}

impl fmt::Display for CalculationOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit { name } => write!(formatter, "{MESSAGE_ZERO_LIMIT}: {name}"),
        }
    }
}

impl Error for CalculationOptionsError {}

fn nonzero(name: &'static str, value: u64) -> Result<u64, CalculationOptionsError> {
    if value == 0 {
        return Err(CalculationOptionsError::ZeroLimit { name });
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum CalculationLimitKind {
    FormulaTokens,
    FormulaSourceBytes,
    FormulaAstNodes,
    FormulaNestingDepth,
    DependencyEdges,
    ArrayCells,
    TextBytes,
    FunctionIterations,
    LetBindings,
    LambdaDepth,
    LambdaInvocations,
}

impl CalculationLimitKind {
    pub(super) const fn detail(self) -> &'static str {
        match self {
            Self::FormulaTokens => MAX_FORMULA_TOKENS,
            Self::FormulaSourceBytes => MAX_FORMULA_SOURCE_BYTES,
            Self::FormulaAstNodes => MAX_FORMULA_AST_NODES,
            Self::FormulaNestingDepth => MAX_FORMULA_NESTING_DEPTH,
            Self::DependencyEdges => MAX_DEPENDENCY_EDGES,
            Self::ArrayCells => MAX_ARRAY_CELLS,
            Self::TextBytes => MAX_TEXT_BYTES,
            Self::FunctionIterations => MAX_FUNCTION_ITERATIONS,
            Self::LetBindings => MAX_LET_BINDINGS,
            Self::LambdaDepth => MAX_LAMBDA_DEPTH,
            Self::LambdaInvocations => MAX_LAMBDA_INVOCATIONS,
        }
    }

    pub(super) fn from_detail(value: &str) -> Option<Self> {
        match value {
            MAX_FORMULA_TOKENS => Some(Self::FormulaTokens),
            MAX_FORMULA_SOURCE_BYTES => Some(Self::FormulaSourceBytes),
            MAX_FORMULA_AST_NODES => Some(Self::FormulaAstNodes),
            MAX_FORMULA_NESTING_DEPTH => Some(Self::FormulaNestingDepth),
            MAX_DEPENDENCY_EDGES => Some(Self::DependencyEdges),
            MAX_ARRAY_CELLS => Some(Self::ArrayCells),
            MAX_TEXT_BYTES => Some(Self::TextBytes),
            MAX_FUNCTION_ITERATIONS => Some(Self::FunctionIterations),
            MAX_LET_BINDINGS => Some(Self::LetBindings),
            MAX_LAMBDA_DEPTH => Some(Self::LambdaDepth),
            MAX_LAMBDA_INVOCATIONS => Some(Self::LambdaInvocations),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_limit_kind_round_trips_through_its_detail() {
        let kinds = [
            CalculationLimitKind::FormulaTokens,
            CalculationLimitKind::FormulaSourceBytes,
            CalculationLimitKind::FormulaAstNodes,
            CalculationLimitKind::FormulaNestingDepth,
            CalculationLimitKind::DependencyEdges,
            CalculationLimitKind::ArrayCells,
            CalculationLimitKind::TextBytes,
            CalculationLimitKind::FunctionIterations,
            CalculationLimitKind::LetBindings,
            CalculationLimitKind::LambdaDepth,
            CalculationLimitKind::LambdaInvocations,
        ];

        for kind in kinds {
            assert_eq!(CalculationLimitKind::from_detail(kind.detail()), Some(kind));
        }
    }

    #[test]
    fn nesting_configuration_remains_source_compatible_above_internal_safe_depth() {
        let limits = CalculationLimits::default()
            .with_max_formula_nesting_depth(SAFE_FORMULA_NESTING_DEPTH + 1)
            .expect("existing nonzero configuration remains accepted");
        assert_eq!(
            limits.max_formula_nesting_depth(),
            SAFE_FORMULA_NESTING_DEPTH + 1
        );
    }
}
