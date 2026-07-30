use std::collections::BTreeMap;

use super::Engine;
use crate::calculation::ast::Expr;
use crate::calculation::lambda::definition;
use crate::calculation::lambda::{is_local_name, walk_local_scope};
use crate::calculation::runtime::CellId;
use crate::{DefinedName, DefinedNameScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameGraphStatus {
    Supported,
    Cycle,
    LimitExceeded,
}

impl Engine<'_> {
    pub(super) fn classify_name_graphs(&mut self) {
        for (cell, expr) in &self.asts {
            match self.inspect_name_graph(cell.0, expr) {
                NameGraphStatus::Supported => {}
                NameGraphStatus::Cycle => {
                    self.name_cycle_cells.insert(*cell);
                }
                NameGraphStatus::LimitExceeded => {
                    self.name_limit_cells.insert(*cell);
                }
            }
        }
    }

    fn inspect_name_graph(&self, sheet: usize, root: &Expr) -> NameGraphStatus {
        let mut pending: Vec<(String, bool, u64)> = name_references(root)
            .into_iter()
            .map(|name| (name, false, 1))
            .collect();
        let mut states = BTreeMap::<(DefinedNameScope, String), bool>::new();
        while let Some((name, expanded, depth)) = pending.pop() {
            let Some((defined_name_index, defined_name)) = self.resolve_defined_name(sheet, &name)
            else {
                continue;
            };
            let key = (defined_name.scope(), defined_name.lookup_key().to_owned());
            if expanded {
                states.insert(key, true);
                continue;
            }
            match states.get(&key) {
                Some(false) => return NameGraphStatus::Cycle,
                Some(true) => continue,
                None => {}
            }
            if depth > self.options.limits().max_formula_nesting_depth() {
                return NameGraphStatus::LimitExceeded;
            }
            states.insert(key, false);
            pending.push((name, true, depth));
            let Some(expr) = self.defined_name_asts[defined_name_index].as_ref() else {
                continue;
            };
            if definition(expr).is_some() {
                // Callable recursion is resolved at invocation time through the immutable
                // defined-name table. It is not the ordinary value-cycle graph.
                continue;
            }
            pending.extend(
                name_references(expr)
                    .into_iter()
                    .map(|child| (child, false, depth + 1)),
            );
        }
        NameGraphStatus::Supported
    }

    pub(in crate::calculation) fn has_name_cycle(&self, cell: CellId) -> bool {
        self.name_cycle_cells.contains(&cell)
    }

    pub(in crate::calculation) fn has_name_limit(&self, cell: CellId) -> bool {
        self.name_limit_cells.contains(&cell)
    }

    pub(in crate::calculation) fn resolve_name_expr(
        &self,
        sheet: usize,
        name: &str,
    ) -> Option<&Expr> {
        let (index, _) = self.resolve_defined_name(sheet, name)?;
        self.defined_name_asts.get(index)?.as_ref()
    }

    fn resolve_defined_name(&self, sheet: usize, name: &str) -> Option<(usize, &DefinedName)> {
        let sheet_id = self.workbook.sheets().get(sheet)?.id();
        self.workbook
            .defined_name(DefinedNameScope::Sheet(sheet_id), name)
            .or_else(|| self.workbook.defined_name(DefinedNameScope::Workbook, name))
    }
}

fn name_references(expr: &Expr) -> Vec<String> {
    let mut names = Vec::new();
    collect_name_references(expr, &mut Vec::new(), &mut names);
    names
}

fn collect_name_references(expr: &Expr, local_names: &mut Vec<String>, names: &mut Vec<String>) {
    match expr {
        Expr::Name(name) => {
            if !is_local_name(name, local_names) {
                names.push(name.clone());
            }
        }
        Expr::Call { name, args } => {
            if walk_local_scope(name, args, local_names, |arg, scope| {
                collect_name_references(arg, scope, names);
            }) {
                return;
            }
            for arg in args {
                collect_name_references(arg, local_names, names);
            }
        }
        Expr::ImplicitIntersection(inner)
        | Expr::Paren(inner)
        | Expr::Unary { operand: inner, .. } => {
            collect_name_references(inner, local_names, names);
        }
        Expr::Binary { left, right, .. } => {
            collect_name_references(left, local_names, names);
            collect_name_references(right, local_names, names);
        }
        Expr::Range { start, end } => {
            collect_name_references(start, local_names, names);
            collect_name_references(end, local_names, names);
        }
        Expr::Array(rows) => {
            for element in rows.iter().flatten() {
                collect_name_references(element, local_names, names);
            }
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::Missing => {}
    }
}
