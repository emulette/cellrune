use std::fmt;

use super::decimal::DecimalTrace;
use super::functions::BuiltinCallable;
use super::value::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Plus,
    Percent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinaryOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Power => "^",
            BinaryOp::Concat => "&",
            BinaryOp::Eq => "=",
            BinaryOp::Ne => "<>",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    pub column: u32,
    pub row: u32,
    pub column_absolute: bool,
    pub row_absolute: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColRef {
    pub column: u32,
    pub absolute: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowRef {
    pub row: u32,
    pub absolute: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefBody {
    Cell(CellRef),
    Area(CellRef, CellRef),
    Columns(ColRef, ColRef),
    Rows(RowRef, RowRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetPrefix {
    pub name: String,
    /// End sheet of a 3-D sheet range (`Sheet1:Sheet3!A1`).
    pub end_name: Option<String>,
    pub quoted: bool,
}

impl SheetPrefix {
    pub fn sheet_range_detail(&self) -> Option<String> {
        self.end_name
            .as_ref()
            .map(|end| format!("{}:{}", self.name, end))
    }

    /// Returns the offending name when this prefix addresses another workbook.
    ///
    /// Excel forbids `[` and `]` in sheet names, so a bracket in the sheet-name position is always
    /// an external-workbook prefix. The typed parser routes authored external spellings to
    /// `ExternalWorkbookReference`; this check remains a fail-closed guard for references created
    /// by dynamic parsing or older internal paths. Without it, an external reference could resolve
    /// as an ordinary missing sheet and produce a catchable `#REF!`.
    pub fn external_workbook_detail(&self) -> Option<String> {
        [Some(&self.name), self.end_name.as_ref()]
            .into_iter()
            .flatten()
            .find(|name| name.contains('['))
            .cloned()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub sheet: Option<SheetPrefix>,
    pub body: RefBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredItem {
    All,
    Data,
    Headers,
    Totals,
    ThisRow,
}

impl StructuredItem {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "#All",
            Self::Data => "#Data",
            Self::Headers => "#Headers",
            Self::Totals => "#Totals",
            Self::ThisRow => "#This Row",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredColumns {
    Single(Box<str>),
    Range { start: Box<str>, end: Box<str> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredReference {
    pub table: Option<Box<str>>,
    pub items: Vec<StructuredItem>,
    pub columns: Option<StructuredColumns>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExternalReferenceTarget {
    Reference(RefBody),
    DefinedName(Box<str>),
    StructuredReference(StructuredReference),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalWorkbookReference {
    pub workbook: Box<str>,
    pub sheet: Option<Box<str>>,
    pub sheet_end: Option<Box<str>>,
    pub sheet_quoted: bool,
    pub quoted: bool,
    pub target: ExternalReferenceTarget,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormulaDisplayMode {
    Authored,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NumberLiteral {
    value: f64,
    decimal_trace: Option<DecimalTrace>,
}

impl NumberLiteral {
    pub(super) fn from_literal(value: f64, literal: &str) -> Self {
        Self {
            value,
            decimal_trace: DecimalTrace::from_literal(literal),
        }
    }

    pub(super) fn from_number(value: f64) -> Self {
        Self {
            value,
            decimal_trace: DecimalTrace::from_number(value),
        }
    }

    pub(super) const fn value(self) -> f64 {
        self.value
    }

    pub(super) const fn decimal_trace(self) -> Option<DecimalTrace> {
        self.decimal_trace
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(NumberLiteral),
    Text(String),
    Logical(bool),
    ErrorLit(ErrorKind),
    Ref(Reference),
    StructuredRef(StructuredReference),
    ReferenceUnion {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    ReferenceIntersection {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    SpillRef(Box<Expr>),
    ExternalReference(ExternalWorkbookReference),
    QualifiedName {
        sheet: SheetPrefix,
        name: Box<str>,
    },
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },
    Name(String),
    BuiltinCallable(BuiltinCallable),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Invoke {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    ImplicitIntersection(Box<Expr>),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Paren(Box<Expr>),
    Array(Vec<Vec<Expr>>),
    Missing,
}

impl Expr {
    pub(super) fn number(value: f64) -> Self {
        Self::Number(NumberLiteral::from_number(value))
    }

    #[allow(dead_code)]
    pub(super) const fn display_with_mode(&self, mode: FormulaDisplayMode) -> ExprDisplay<'_> {
        ExprDisplay { expr: self, mode }
    }
}

#[allow(dead_code)]
pub(super) struct ExprDisplay<'a> {
    expr: &'a Expr,
    mode: FormulaDisplayMode,
}

pub fn column_label(column: u32) -> String {
    let mut remaining = column;
    let mut label = String::new();
    while remaining > 0 {
        let index = ((remaining - 1) % 26) as u8;
        label.insert(0, char::from(b'A' + index));
        remaining = (remaining - 1) / 26;
    }
    label
}

pub fn column_number(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut column = 0_u32;
    for character in letters.chars() {
        let upper = character.to_ascii_uppercase();
        if !upper.is_ascii_uppercase() {
            return None;
        }
        column = column * 26 + (upper as u32 - 'A' as u32 + 1);
    }
    if column <= super::EXCEL_MAX_COLUMNS {
        Some(column)
    } else {
        None
    }
}

fn write_cell(formatter: &mut fmt::Formatter<'_>, cell: &CellRef) -> fmt::Result {
    if cell.column_absolute {
        formatter.write_str("$")?;
    }
    formatter.write_str(&column_label(cell.column))?;
    if cell.row_absolute {
        formatter.write_str("$")?;
    }
    write!(formatter, "{}", cell.row)
}

fn write_sheet_prefix(formatter: &mut fmt::Formatter<'_>, sheet: &SheetPrefix) -> fmt::Result {
    let label = match &sheet.end_name {
        Some(end) => format!("{}:{}", sheet.name, end),
        None => sheet.name.clone(),
    };
    if sheet.quoted {
        write!(formatter, "'{}'!", label.replace('\'', "''"))
    } else {
        write!(formatter, "{label}!")
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(sheet) = &self.sheet {
            write_sheet_prefix(formatter, sheet)?;
        }
        match &self.body {
            RefBody::Cell(cell) => write_cell(formatter, cell),
            RefBody::Area(start, end) => {
                write_cell(formatter, start)?;
                formatter.write_str(":")?;
                write_cell(formatter, end)
            }
            RefBody::Columns(start, end) => {
                if start.absolute {
                    formatter.write_str("$")?;
                }
                formatter.write_str(&column_label(start.column))?;
                formatter.write_str(":")?;
                if end.absolute {
                    formatter.write_str("$")?;
                }
                formatter.write_str(&column_label(end.column))
            }
            RefBody::Rows(start, end) => {
                if start.absolute {
                    formatter.write_str("$")?;
                }
                write!(formatter, "{}", start.row)?;
                formatter.write_str(":")?;
                if end.absolute {
                    formatter.write_str("$")?;
                }
                write!(formatter, "{}", end.row)
            }
        }
    }
}

fn write_structured_name(formatter: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    for character in name.chars() {
        if matches!(character, '[' | ']' | '#' | '\'' | '@') {
            formatter.write_str("'")?;
        }
        write!(formatter, "{character}")?;
    }
    Ok(())
}

fn write_structured_component(formatter: &mut fmt::Formatter<'_>, component: &str) -> fmt::Result {
    formatter.write_str("[")?;
    write_structured_name(formatter, component)?;
    formatter.write_str("]")
}

pub(super) fn structured_column_needs_grouping(column: &str) -> bool {
    column
        .chars()
        .any(structured_column_character_needs_grouping)
}

pub(super) const fn structured_column_character_needs_grouping(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n'
            | '\r'
            | ','
            | ':'
            | '.'
            | '['
            | ']'
            | '#'
            | '\''
            | '"'
            | '{'
            | '}'
            | '$'
            | '^'
            | '&'
            | '*'
            | '+'
            | '='
            | '-'
            | '>'
            | '<'
            | '/'
            | '@'
            | '\\'
            | '!'
            | '('
            | ')'
            | '%'
            | '?'
            | '`'
            | ';'
            | '~'
            | '_'
    )
}

fn write_structured_reference(
    formatter: &mut fmt::Formatter<'_>,
    reference: &StructuredReference,
) -> fmt::Result {
    if let Some(table) = &reference.table {
        formatter.write_str(table)?;
    }
    formatter.write_str("[")?;
    if reference.items == [StructuredItem::ThisRow] {
        formatter.write_str("@")?;
        match &reference.columns {
            None => {}
            Some(StructuredColumns::Single(column)) => {
                if structured_column_needs_grouping(column) {
                    write_structured_component(formatter, column)?;
                } else {
                    write_structured_name(formatter, column)?;
                }
            }
            Some(StructuredColumns::Range { start, end }) => {
                write_structured_component(formatter, start)?;
                formatter.write_str(":")?;
                write_structured_component(formatter, end)?;
            }
        }
        return formatter.write_str("]");
    }
    let needs_grouping = !reference.items.is_empty() && reference.columns.is_some()
        || reference.items.len() > 1
        || matches!(reference.columns, Some(StructuredColumns::Range { .. }))
        || matches!(
            &reference.columns,
            Some(StructuredColumns::Single(column))
                if structured_column_needs_grouping(column)
        );
    if needs_grouping {
        let mut wrote = false;
        for item in &reference.items {
            if wrote {
                formatter.write_str(",")?;
            }
            write!(formatter, "[{}]", item.as_str())?;
            wrote = true;
        }
        if let Some(columns) = &reference.columns {
            if wrote {
                formatter.write_str(",")?;
            }
            match columns {
                StructuredColumns::Single(column) => {
                    write_structured_component(formatter, column)?;
                }
                StructuredColumns::Range { start, end } => {
                    write_structured_component(formatter, start)?;
                    formatter.write_str(":")?;
                    write_structured_component(formatter, end)?;
                }
            }
        }
    } else if let Some(item) = reference.items.first() {
        formatter.write_str(item.as_str())?;
    } else if let Some(StructuredColumns::Single(column)) = &reference.columns {
        write_structured_name(formatter, column)?;
    }
    formatter.write_str("]")
}

impl fmt::Display for StructuredReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_structured_reference(formatter, self)
    }
}

fn write_arguments(
    formatter: &mut fmt::Formatter<'_>,
    args: &[Expr],
    mode: FormulaDisplayMode,
) -> fmt::Result {
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            formatter.write_str(",")?;
        }
        arg.fmt_mode(formatter, mode)?;
    }
    Ok(())
}

fn write_storage_wrapper(
    formatter: &mut fmt::Formatter<'_>,
    function: &str,
    operand: &Expr,
    mode: FormulaDisplayMode,
) -> fmt::Result {
    write!(formatter, "{function}(")?;
    let needs_argument_grouping = storage_argument_needs_grouping(operand);
    if needs_argument_grouping {
        formatter.write_str("(")?;
    }
    operand.fmt_mode(formatter, mode)?;
    if needs_argument_grouping {
        formatter.write_str(")")?;
    }
    formatter.write_str(")")
}

fn storage_argument_needs_grouping(operand: &Expr) -> bool {
    match operand {
        Expr::ReferenceUnion { .. } => true,
        Expr::Unary { operand, .. } => storage_argument_needs_grouping(operand),
        Expr::Paren(_) => false,
        _ => false,
    }
}

impl Expr {
    fn fmt_mode(
        &self,
        formatter: &mut fmt::Formatter<'_>,
        mode: FormulaDisplayMode,
    ) -> fmt::Result {
        match self {
            Expr::Number(number) => {
                formatter.write_str(&super::value::number_to_general_text(number.value()))
            }
            Expr::Text(text) => write!(formatter, "\"{}\"", text.replace('"', "\"\"")),
            Expr::Logical(true) => formatter.write_str("TRUE"),
            Expr::Logical(false) => formatter.write_str("FALSE"),
            Expr::ErrorLit(kind) => formatter.write_str(kind.as_str()),
            Expr::Ref(reference) => fmt::Display::fmt(reference, formatter),
            Expr::StructuredRef(reference) => write_structured_reference(formatter, reference),
            Expr::ReferenceUnion { left, right } => {
                left.fmt_mode(formatter, mode)?;
                formatter.write_str(",")?;
                right.fmt_mode(formatter, mode)
            }
            Expr::ReferenceIntersection { left, right } => {
                left.fmt_mode(formatter, mode)?;
                formatter.write_str(" ")?;
                right.fmt_mode(formatter, mode)
            }
            Expr::SpillRef(anchor) => match mode {
                FormulaDisplayMode::Authored => {
                    let needs_grouping = matches!(
                        anchor.as_ref(),
                        Expr::ReferenceUnion { .. }
                            | Expr::ReferenceIntersection { .. }
                            | Expr::Range { .. }
                            | Expr::ImplicitIntersection(_)
                            | Expr::Unary { .. }
                            | Expr::Binary { .. }
                    );
                    if needs_grouping {
                        formatter.write_str("(")?;
                    }
                    anchor.fmt_mode(formatter, mode)?;
                    if needs_grouping {
                        formatter.write_str(")")?;
                    }
                    formatter.write_str("#")
                }
                FormulaDisplayMode::Storage => {
                    write_storage_wrapper(formatter, "_xlfn.ANCHORARRAY", anchor, mode)
                }
            },
            Expr::ExternalReference(reference) => {
                if reference.quoted {
                    let mut prefix = reference.workbook.to_string();
                    if let Some(sheet) = &reference.sheet {
                        prefix.push_str(sheet);
                        if let Some(sheet_end) = &reference.sheet_end {
                            prefix.push(':');
                            prefix.push_str(sheet_end);
                        }
                    }
                    write!(formatter, "'{}'!", prefix.replace('\'', "''"))?;
                } else {
                    formatter.write_str(&reference.workbook)?;
                    if let Some(sheet) = &reference.sheet {
                        let mut label = sheet.to_string();
                        if let Some(sheet_end) = &reference.sheet_end {
                            label.push(':');
                            label.push_str(sheet_end);
                        }
                        if reference.sheet_quoted {
                            write!(formatter, "'{}'", label.replace('\'', "''"))?;
                        } else {
                            formatter.write_str(&label)?;
                        }
                    }
                    formatter.write_str("!")?;
                }
                match &reference.target {
                    ExternalReferenceTarget::Reference(body) => fmt::Display::fmt(
                        &Reference {
                            sheet: None,
                            body: *body,
                        },
                        formatter,
                    ),
                    ExternalReferenceTarget::DefinedName(name) => formatter.write_str(name),
                    ExternalReferenceTarget::StructuredReference(structured) => {
                        write_structured_reference(formatter, structured)
                    }
                }
            }
            Expr::QualifiedName { sheet, name } => {
                write_sheet_prefix(formatter, sheet)?;
                formatter.write_str(name)
            }
            Expr::Range { start, end } => {
                start.fmt_mode(formatter, mode)?;
                formatter.write_str(":")?;
                end.fmt_mode(formatter, mode)
            }
            Expr::Name(name) => formatter.write_str(name),
            Expr::BuiltinCallable(callable) => match mode {
                FormulaDisplayMode::Authored => formatter.write_str(callable.canonical_name()),
                FormulaDisplayMode::Storage => {
                    formatter.write_str("_xleta.")?;
                    formatter.write_str(callable.canonical_name())
                }
            },
            Expr::Call { name, args } => {
                formatter.write_str(name)?;
                formatter.write_str("(")?;
                write_arguments(formatter, args, mode)?;
                formatter.write_str(")")
            }
            Expr::Invoke { callee, args } => {
                callee.fmt_mode(formatter, mode)?;
                formatter.write_str("(")?;
                write_arguments(formatter, args, mode)?;
                formatter.write_str(")")
            }
            Expr::ImplicitIntersection(operand) => match mode {
                FormulaDisplayMode::Authored => {
                    formatter.write_str("@")?;
                    operand.fmt_mode(formatter, mode)
                }
                FormulaDisplayMode::Storage => {
                    write_storage_wrapper(formatter, "_xlfn.SINGLE", operand, mode)
                }
            },
            Expr::Unary {
                op: UnaryOp::Negate,
                operand,
            } => {
                formatter.write_str("-")?;
                operand.fmt_mode(formatter, mode)
            }
            Expr::Unary {
                op: UnaryOp::Plus,
                operand,
            } => {
                formatter.write_str("+")?;
                operand.fmt_mode(formatter, mode)
            }
            Expr::Unary {
                op: UnaryOp::Percent,
                operand,
            } => {
                operand.fmt_mode(formatter, mode)?;
                formatter.write_str("%")
            }
            Expr::Binary { op, left, right } => {
                left.fmt_mode(formatter, mode)?;
                formatter.write_str(op.symbol())?;
                right.fmt_mode(formatter, mode)
            }
            Expr::Paren(inner) => {
                formatter.write_str("(")?;
                inner.fmt_mode(formatter, mode)?;
                formatter.write_str(")")
            }
            Expr::Array(rows) => {
                formatter.write_str("{")?;
                for (row_index, row) in rows.iter().enumerate() {
                    if row_index > 0 {
                        formatter.write_str(";")?;
                    }
                    for (col_index, element) in row.iter().enumerate() {
                        if col_index > 0 {
                            formatter.write_str(",")?;
                        }
                        element.fmt_mode(formatter, mode)?;
                    }
                }
                formatter.write_str("}")
            }
            Expr::Missing => Ok(()),
        }
    }
}

impl fmt::Display for ExprDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.expr.fmt_mode(formatter, self.mode)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_mode(formatter, FormulaDisplayMode::Authored)
    }
}
