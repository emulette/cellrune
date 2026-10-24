//! What a formula's text refers to outside its own sheet's cells: sheet qualifiers, other
//! workbooks, defined names, and tables.

use std::collections::BTreeSet;

use super::CalculationLimits;
use super::ast::{Expr, SheetPrefix};
use super::parser::parse_formula_with_limits;

/// The references of one formula, by the names its text writes.
#[derive(Debug, Default)]
pub(crate) struct FormulaReferences {
    /// Each sheet a qualifier names, both ends of a 3-D reference included.
    pub(crate) sheets: BTreeSet<String>,
    /// Whether a qualifier spans sheets (`S1:S3!A1`).
    pub(crate) three_d: bool,
    /// Whether the formula reads another workbook.
    pub(crate) external: bool,
    /// Each unqualified name or called name that may be a defined name.
    pub(crate) names: BTreeSet<String>,
    /// Each table a structured reference names.
    pub(crate) tables: BTreeSet<String>,
}

/// Returns the references of `text`, or `None` when it does not parse.
pub(crate) fn formula_references(text: &str) -> Option<FormulaReferences> {
    let parsed = parse_formula_with_limits(text, CalculationLimits::default()).ok()?;
    Some(expression_references(parsed.root()))
}

/// Returns the references of a parsed formula.
pub(in crate::calculation) fn expression_references(root: &Expr) -> FormulaReferences {
    let mut references = FormulaReferences::default();
    visit(root, &mut references);
    references
}

impl FormulaReferences {
    /// Whether the formula names a sheet whose lookup key is in `sheets`, or calls a function that
    /// reads sheet names or the number of sheets, which adding or renaming a sheet changes.
    pub(crate) fn reads_sheet_names(&self, sheets: &BTreeSet<String>) -> bool {
        self.sheets.iter().any(|sheet| {
            crate::SheetName::new(sheet.as_str())
                .is_ok_and(|name| sheets.contains(name.lookup_key()))
        }) || self.names.iter().any(|name| {
            matches!(
                super::functions::normalize_name(name).as_str(),
                "SHEET" | "SHEETS" | "CELL"
            )
        })
    }
}

fn visit(expr: &Expr, references: &mut FormulaReferences) {
    super::stack::grow(|| match expr {
        Expr::Ref(reference) => {
            if let Some(sheet) = &reference.sheet {
                qualifier(sheet, references);
            }
        }
        Expr::QualifiedName { sheet, .. } => qualifier(sheet, references),
        Expr::ExternalReference(_) => references.external = true,
        Expr::StructuredRef(reference) => {
            references
                .tables
                .extend(reference.table.as_deref().map(str::to_owned));
        }
        Expr::Name(name) => {
            references.names.insert(name.clone());
        }
        Expr::Call { name, args } => {
            references.names.insert(name.clone());
            for arg in args {
                visit(arg, references);
            }
        }
        Expr::Invoke { callee, args } => {
            visit(callee, references);
            for arg in args {
                visit(arg, references);
            }
        }
        Expr::ReferenceUnion { left, right }
        | Expr::ReferenceIntersection { left, right }
        | Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            visit(left, references);
            visit(right, references);
        }
        Expr::SpillRef(inner)
        | Expr::ImplicitIntersection(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => visit(inner, references),
        Expr::Array(rows) => {
            for item in rows.iter().flatten() {
                visit(item, references);
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::BuiltinCallable(_)
        | Expr::Missing => {}
    });
}

fn qualifier(sheet: &SheetPrefix, references: &mut FormulaReferences) {
    if sheet.external_workbook_detail().is_some() {
        references.external = true;
    }
    references.sheets.insert(sheet.name.clone());
    if let Some(end) = &sheet.end_name {
        references.three_d = true;
        references.sheets.insert(end.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::formula_references;

    #[test]
    fn references_name_the_sheets_workbooks_names_and_tables_a_formula_reads() {
        let references = formula_references(
            "SUM('S 2'!A1,S3:S5!B2,Rate,T[c],[1]Book!X1)+MyLambda(Local!Name,A1)",
        )
        .expect("parses");
        assert_eq!(
            references.sheets.into_iter().collect::<Vec<_>>(),
            ["Local", "S 2", "S3", "S5"]
        );
        assert!(references.three_d);
        assert!(references.external);
        assert!(references.names.contains("Rate"));
        assert!(references.names.contains("MyLambda"));
        assert_eq!(references.tables.into_iter().collect::<Vec<_>>(), ["T"]);
        let local = formula_references("A1+[@c]").expect("parses");
        assert!(local.sheets.is_empty() && !local.three_d && !local.external);
        assert!(formula_references("SUM(").is_none());
    }
}
