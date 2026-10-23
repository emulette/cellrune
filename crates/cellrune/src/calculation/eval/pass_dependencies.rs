//! The dependencies a pass over new values resolves again: only those of the formulas whose
//! targets can follow values, while the rest keep the nodes the last collection built.

use super::dependency_areas::AreaNodes;
use super::{CellId, Engine, EvalContext, EvaluationBudget};
use crate::calculation::ast::Expr;
use crate::calculation::functions::descriptor::DependencyKind;
use crate::calculation::functions::{
    DynamicFunction, Evaluator, function_dependency_kind, function_evaluator, normalize_name,
};
use crate::calculation::graph::DependencyGraph;

impl Engine<'_> {
    /// The formulas whose dependency targets can depend on values.
    pub(super) fn value_dependent_formulas(
        &self,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<CellId>, ()> {
        let mut formulas = Vec::new();
        for (cell, parsed) in self.asts.iter() {
            if cancelled() {
                return Err(());
            }
            let budget = EvaluationBudget::default();
            if !self.targets_are_structural(
                EvalContext::for_cancellable(*cell, &budget, cancelled),
                parsed.root(),
            ) {
                formulas.push(*cell);
            }
        }
        Ok(formulas)
    }

    /// Resolves the dependencies of `formulas` again in `graph`, whose other formulas' nodes stay
    /// valid while the array regions are those it was collected with. Returns whether the graph
    /// stays within the dependency edge limit; when it does not, a whole collection decides which
    /// formulas the limit stops.
    pub(super) fn recollect_formulas(
        &self,
        graph: &mut DependencyGraph,
        areas: &mut AreaNodes,
        formulas: &[CellId],
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        for cell in formulas {
            let nodes =
                if self.name_cycle_cells.contains(cell) || self.name_limit_cells.contains(cell) {
                    Vec::new()
                } else {
                    let targets = self.formula_dependency_targets(*cell, cancelled)?;
                    self.dependency_nodes(&targets, graph, areas, cancelled)?
                };
            graph.insert(*cell, nodes);
        }
        let renumbered = graph.retain_referenced_areas();
        areas.renumber(&renumbered);
        Ok(graph.edge_count() <= self.options.limits().max_dependency_edges())
    }

    /// Whether the dependency collection resolves `expr` without reading a value: only plain
    /// references, ranges between them, operators, and calls of built-in functions that are not
    /// dynamic references, LET, or LAMBDA scopes. Names, spills, and the rest count as reading
    /// values, so a pass resolves them again.
    fn targets_are_structural(&self, context: EvalContext<'_>, expr: &Expr) -> bool {
        let plain_range = |expr: &Expr| match expr {
            Expr::Ref(_) => true,
            Expr::Range { start, end } => {
                matches!((&**start, &**end), (Expr::Ref(_), Expr::Ref(_)))
            }
            _ => false,
        };
        crate::calculation::stack::grow(|| match expr {
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::ExternalReference(_)
            | Expr::Missing
            | Expr::Ref(_)
            | Expr::StructuredRef(_) => true,
            Expr::Range { .. } => plain_range(expr),
            Expr::ImplicitIntersection(inner) => plain_range(inner),
            Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
                self.targets_are_structural(context, inner)
            }
            Expr::Binary { left, right, .. } => {
                self.targets_are_structural(context, left)
                    && self.targets_are_structural(context, right)
            }
            Expr::Array(rows) => rows
                .iter()
                .flatten()
                .all(|item| self.targets_are_structural(context, item)),
            Expr::Call { name, args } => {
                self.shadowed_call_arguments_are_reachable(context, name, args.len(), &[])
                    .is_none()
                    && matches!(
                        function_dependency_kind(&normalize_name(name)),
                        None | Some(DependencyKind::Standard)
                    )
                    && !matches!(
                        function_evaluator(name),
                        Some(Evaluator::Dynamic(
                            DynamicFunction::Map | DynamicFunction::Lambda | DynamicFunction::Let
                        ))
                    )
                    && args
                        .iter()
                        .all(|arg| self.targets_are_structural(context, arg))
            }
            Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Name(_)
            | Expr::QualifiedName { .. }
            | Expr::BuiltinCallable(_)
            | Expr::Invoke { .. } => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::Engine;
    use crate::{CalculationOptions, CellAddress, FormulaText, WorkbookDraft};

    #[test]
    fn a_pass_resolves_again_only_formulas_whose_references_can_follow_values() {
        let mut draft = WorkbookDraft::new();
        let sheet = draft.workbook().sheets()[0].id();
        let formulas = [
            ("A1*2+1", false),
            ("SUM(A1:A9)/COUNT(A1:A9)", false),
            ("IF(A1>0,A2,A3)", false),
            ("{1,2}+@A1:A3", false),
            ("INDIRECT(\"A1\")", true),
            ("SUM(OFFSET(A1,1,0,2,1))", true),
            ("SUM(A1:INDEX(A1:A9,3))", true),
            ("LET(x,A1,x*2)", true),
            ("SUM(MAP(A1:A2,LAMBDA(v,v)))", true),
            ("Rate*2", true),
            ("SUM(A1#)", true),
        ];
        for (row, (text, _)) in (2..).zip(formulas) {
            draft
                .set_cell_formula(
                    sheet,
                    CellAddress::from_indices(row, 2).expect("address"),
                    FormulaText::from_xlsx(text).expect("formula"),
                )
                .expect("formula edit");
        }
        let engine = Engine::analyze(draft.workbook(), CalculationOptions::default());
        let dependent = engine
            .value_dependent_formulas(&|| false)
            .expect("not cancelled");
        let expected = (2..)
            .zip(formulas)
            .filter(|(_, (_, dependent))| *dependent)
            .map(|(row, _)| (0, row, 2))
            .collect::<Vec<_>>();
        assert_eq!(dependent, expected);
    }
}
