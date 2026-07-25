use std::error::Error;
use std::fmt;

const MAX_FORMULA_TOKENS: &str = "max_formula_tokens";
const MAX_FORMULA_AST_NODES: &str = "max_formula_ast_nodes";
const MAX_FORMULA_NESTING_DEPTH: &str = "max_formula_nesting_depth";
const MAX_DEPENDENCY_EDGES: &str = "max_dependency_edges";
const MAX_ARRAY_CELLS: &str = "max_array_cells";
const MAX_TEXT_BYTES: &str = "max_text_bytes";
const MAX_FUNCTION_ITERATIONS: &str = "max_function_iterations";
const MESSAGE_ZERO_LIMIT: &str = "calculation limit must be greater than zero";

/// Resource limits applied while formulas are parsed, scheduled, and evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalculationLimits {
    max_formula_tokens: u64,
    max_formula_ast_nodes: u64,
    max_formula_nesting_depth: u64,
    max_dependency_edges: u64,
    max_array_cells: u64,
    max_text_bytes: u64,
    max_function_iterations: u64,
}

impl CalculationLimits {
    /// Returns the maximum lexical token count of one formula.
    pub const fn max_formula_tokens(self) -> u64 {
        self.max_formula_tokens
    }

    /// Returns the maximum AST node count of one formula.
    pub const fn max_formula_ast_nodes(self) -> u64 {
        self.max_formula_ast_nodes
    }

    /// Returns the maximum AST nesting depth of one formula.
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

    /// Replaces the per-formula lexical token limit.
    ///
    /// # Errors
    ///
    /// Returns [`CalculationOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_formula_tokens(mut self, value: u64) -> Result<Self, CalculationOptionsError> {
        self.max_formula_tokens = nonzero(MAX_FORMULA_TOKENS, value)?;
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

    /// Replaces the per-formula AST nesting-depth limit.
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
}

impl Default for CalculationLimits {
    fn default() -> Self {
        Self {
            max_formula_tokens: 8_192,
            max_formula_ast_nodes: 8_192,
            max_formula_nesting_depth: 256,
            max_dependency_edges: 10_000_000,
            max_array_cells: 1_000_000,
            max_text_bytes: 32_767,
            max_function_iterations: 1_000_000,
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
    FormulaAstNodes,
    FormulaNestingDepth,
    DependencyEdges,
    ArrayCells,
    TextBytes,
    FunctionIterations,
}

impl CalculationLimitKind {
    pub(super) const fn detail(self) -> &'static str {
        match self {
            Self::FormulaTokens => MAX_FORMULA_TOKENS,
            Self::FormulaAstNodes => MAX_FORMULA_AST_NODES,
            Self::FormulaNestingDepth => MAX_FORMULA_NESTING_DEPTH,
            Self::DependencyEdges => MAX_DEPENDENCY_EDGES,
            Self::ArrayCells => MAX_ARRAY_CELLS,
            Self::TextBytes => MAX_TEXT_BYTES,
            Self::FunctionIterations => MAX_FUNCTION_ITERATIONS,
        }
    }

    pub(super) fn from_detail(value: &str) -> Option<Self> {
        match value {
            MAX_FORMULA_TOKENS => Some(Self::FormulaTokens),
            MAX_FORMULA_AST_NODES => Some(Self::FormulaAstNodes),
            MAX_FORMULA_NESTING_DEPTH => Some(Self::FormulaNestingDepth),
            MAX_DEPENDENCY_EDGES => Some(Self::DependencyEdges),
            MAX_ARRAY_CELLS => Some(Self::ArrayCells),
            MAX_TEXT_BYTES => Some(Self::TextBytes),
            MAX_FUNCTION_ITERATIONS => Some(Self::FunctionIterations),
            _ => None,
        }
    }
}
