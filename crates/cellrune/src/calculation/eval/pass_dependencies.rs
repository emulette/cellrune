//! The dependencies a pass over new values resolves again: only those of the formulas whose
//! targets can follow values, while the rest keep the nodes the last collection built.

use std::collections::{BTreeSet, HashMap};

use super::dependency_areas::AreaNodes;
use super::{CellId, Engine, EvalContext, EvaluationBudget};
use crate::calculation::ast::Expr;
use crate::calculation::functions::descriptor::DependencyKind;
use crate::calculation::functions::{
    DynamicFunction, Evaluator, function_dependency_kind, function_evaluator, normalize_name,
};
use crate::calculation::graph::{AreaId, DependencyGraph, DependencyNode, Schedule};

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

    /// The formulas a pass after one whose evaluations of `stale` read a formula before it was
    /// calculated evaluates again: those and every formula that waits for one of them in `graph`,
    /// scheduled as `scheduled`. The others read only results that are final. Returns `None` when
    /// the pass evaluates every formula: `graph` cycles or blocks other formulas than the last
    /// pass did, or an array formula would calculate again.
    pub(super) fn pass_dirty(
        &self,
        graph: &DependencyGraph,
        scheduled: &Schedule,
        stale: &BTreeSet<CellId>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<BTreeSet<CellId>>, ()> {
        if scheduled.cycle_cells != *self.cycle_cells
            || scheduled.blocked_cells != *self.blocked_cells
        {
            return Ok(None);
        }
        let mut dirty = BTreeSet::new();
        // Whether an area holds a dirty cell. The schedule puts every formula an area holds
        // before the formulas waiting for it, so an area's answer is known when first asked.
        let mut dirty_areas = HashMap::new();
        for cell in &scheduled.order {
            if cancelled() {
                return Err(());
            }
            if stale.contains(cell)
                || graph.dependencies(*cell).is_some_and(|nodes| {
                    nodes
                        .iter()
                        .any(|node| node_is_dirty(graph, *node, &dirty, &mut dirty_areas))
                })
            {
                if self.legacy_array_range(*cell).is_some()
                    || self.dynamic_array_range(*cell).is_some()
                {
                    return Ok(None);
                }
                dirty.insert(*cell);
            }
        }
        Ok(Some(dirty))
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

fn node_is_dirty(
    graph: &DependencyGraph,
    node: DependencyNode,
    dirty: &BTreeSet<CellId>,
    dirty_areas: &mut HashMap<AreaId, bool>,
) -> bool {
    match node {
        DependencyNode::Cell(cell) => dirty.contains(&cell),
        DependencyNode::Area(area) => {
            if let Some(known) = dirty_areas.get(&area) {
                return *known;
            }
            let found = crate::calculation::stack::grow(|| {
                graph
                    .area(area)
                    .iter()
                    .any(|member| node_is_dirty(graph, *member, dirty, dirty_areas))
            });
            dirty_areas.insert(area, found);
            found
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Engine;
    use crate::calculation::value::Value;
    use crate::{CalculationOptions, CellAddress, CellValue, FormulaText, WorkbookDraft};

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

    #[test]
    fn a_pass_after_one_that_read_formulas_too_early_evaluates_only_those_and_their_readers() {
        let calculate = |with_dynamic_reference: bool| {
            let mut draft = WorkbookDraft::new();
            let sheet = draft.workbook().sheets()[0].id();
            let address = |row, column| CellAddress::from_indices(row, column).expect("address");
            for row in 1..=200 {
                draft
                    .set_cell_value(
                        sheet,
                        address(row, 1),
                        CellValue::number(f64::from(row)).expect("number"),
                    )
                    .expect("value edit");
                draft
                    .set_cell_formula(
                        sheet,
                        address(row, 2),
                        FormulaText::from_xlsx(format!("A{row}*2")).expect("formula"),
                    )
                    .expect("formula edit");
            }
            draft
                .set_cell_formula(
                    sheet,
                    address(1, 3),
                    FormulaText::from_xlsx("5+5").expect("formula"),
                )
                .expect("formula edit");
            if with_dynamic_reference {
                // The first pass orders D1 by C1 alone and reads B2:B10 before they calculate.
                for (column, text) in [(4, "SUM(INDIRECT(\"B1:B\"&C1))"), (5, "D1+1")] {
                    draft
                        .set_cell_formula(
                            sheet,
                            address(1, column),
                            FormulaText::from_xlsx(text).expect("formula"),
                        )
                        .expect("formula edit");
                }
            }
            let engine = Engine::evaluate(draft.workbook(), CalculationOptions::default());
            (engine.reference_cells(), engine.cell_value((0, 1, 5)))
        };
        let (single_pass, _) = calculate(false);
        let (reads, reader) = calculate(true);
        assert_eq!(reader, Value::Number(111.0));
        // Evaluating every formula again would read each cell the first pass read a second time.
        assert!(
            reads < single_pass + 50,
            "{reads} reads against {single_pass}"
        );
    }
}
