use std::fmt;

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
    /// End sheet of a 3-D sheet range (`Sheet1:Sheet3!A1`); calculation does not support these.
    pub end_name: Option<String>,
    pub quoted: bool,
}

impl SheetPrefix {
    pub fn sheet_range_detail(&self) -> Option<String> {
        self.end_name
            .as_ref()
            .map(|end| format!("{}:{}", self.name, end))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub sheet: Option<SheetPrefix>,
    pub body: RefBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Logical(bool),
    ErrorLit(ErrorKind),
    Ref(Reference),
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
    },
    Name(String),
    Call {
        name: String,
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

impl fmt::Display for Reference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(sheet) = &self.sheet {
            let label = match &sheet.end_name {
                Some(end) => format!("{}:{}", sheet.name, end),
                None => sheet.name.clone(),
            };
            if sheet.quoted {
                write!(formatter, "'{}'!", label.replace('\'', "''"))?;
            } else {
                write!(formatter, "{label}!")?;
            }
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

impl fmt::Display for Expr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Number(number) => {
                formatter.write_str(&super::value::number_to_general_text(*number))
            }
            Expr::Text(text) => write!(formatter, "\"{}\"", text.replace('"', "\"\"")),
            Expr::Logical(true) => formatter.write_str("TRUE"),
            Expr::Logical(false) => formatter.write_str("FALSE"),
            Expr::ErrorLit(kind) => formatter.write_str(kind.as_str()),
            Expr::Ref(reference) => reference.fmt(formatter),
            Expr::Range { start, end } => write!(formatter, "{start}:{end}"),
            Expr::Name(name) => formatter.write_str(name),
            Expr::Call { name, args } => {
                formatter.write_str(name)?;
                formatter.write_str("(")?;
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(",")?;
                    }
                    arg.fmt(formatter)?;
                }
                formatter.write_str(")")
            }
            Expr::ImplicitIntersection(operand) => write!(formatter, "@{operand}"),
            Expr::Unary {
                op: UnaryOp::Negate,
                operand,
            } => write!(formatter, "-{operand}"),
            Expr::Unary {
                op: UnaryOp::Plus,
                operand,
            } => write!(formatter, "+{operand}"),
            Expr::Unary {
                op: UnaryOp::Percent,
                operand,
            } => write!(formatter, "{operand}%"),
            Expr::Binary { op, left, right } => {
                write!(formatter, "{left}{}{right}", op.symbol())
            }
            Expr::Paren(inner) => write!(formatter, "({inner})"),
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
                        element.fmt(formatter)?;
                    }
                }
                formatter.write_str("}")
            }
            Expr::Missing => Ok(()),
        }
    }
}
