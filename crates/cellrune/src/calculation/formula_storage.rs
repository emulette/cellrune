//! Excel storage spelling for authored formula text.
//!
//! Excel saves some authored syntax in a different spelling: future functions carry `_xlfn.`
//! (`_xlfn._xlws.` for FILTER and SORT), LET and LAMBDA locals carry `_xlpm.`, a function passed
//! as a LAMBDA argument carries `_xleta.`, `B1#` becomes `_xlfn.ANCHORARRAY(B1)`, and `@operand`
//! becomes `_xlfn.SINGLE(operand)`. The parser locates those spots and only that spelling is
//! inserted, so numeric literals, whitespace, and every other character stay as authored.
//! Re-printing the AST would normalize them.

use std::borrow::Cow;

use super::CalculationLimits;
use super::ast::{Expr, storage_argument_needs_grouping};
use super::formula_rewrite::push_children;
use super::functions::{
    DynamicFunction, Evaluator, function_evaluator, future_function_prefix,
    storage_builtin_callable,
};
use super::lambda::is_local_name;
use super::parser::parse_formula_with_limits;
use super::scope::canonical_local_name;
use super::syntax::{NodeSpanTree, SourceSpan};

const LOCAL_PREFIX: &str = "_xlpm.";

/// Returns `authored` in Excel storage spelling.
///
/// Text that does not parse is returned unchanged; calculation reports it as unparsed.
pub(crate) fn storage_formula_text(authored: &str) -> Cow<'_, str> {
    let Ok(parsed) = parse_formula_with_limits(authored, CalculationLimits::default()) else {
        return Cow::Borrowed(authored);
    };
    let root = parsed.source_map().node_tree();
    // The parser folds a leading `+` into the root span; the storage spelling follows it.
    let start = match parsed.source_map().token_spans() {
        [first, second, ..] if authored.as_bytes()[first.start] == b'+' => second.start,
        _ => root.span().start,
    };
    let mut planner = StoragePlanner {
        source: authored,
        scope: Vec::new(),
        edits: Vec::new(),
    };
    planner.visit(parsed.root(), root, start);
    if planner.edits.is_empty() {
        return Cow::Borrowed(authored);
    }
    // Edits are planned in source order: a node's openings before its children, its closings
    // after them. Same-offset edits therefore already nest correctly.
    let inserted: usize = planner.edits.iter().map(|edit| edit.text.len()).sum();
    let mut stored = String::with_capacity(authored.len() + inserted);
    let mut cursor = 0;
    for edit in planner.edits {
        stored.push_str(&authored[cursor..edit.span.start]);
        stored.push_str(edit.text);
        cursor = edit.span.end;
    }
    stored.push_str(&authored[cursor..]);
    Cow::Owned(stored)
}

struct StorageEdit {
    span: SourceSpan,
    text: &'static str,
}

struct StoragePlanner<'a> {
    source: &'a str,
    scope: Vec<String>,
    edits: Vec<StorageEdit>,
}

impl StoragePlanner<'_> {
    fn visit(&mut self, expr: &Expr, node: &NodeSpanTree, start: usize) {
        super::stack::grow(|| match expr {
            Expr::Call { name, args } => {
                if is_local_name(name, &self.scope) {
                    self.prefix_local(name, start);
                    self.visit_children(expr, node);
                    return;
                }
                if let Some(prefix) = future_function_prefix(name) {
                    self.edit(SourceSpan::empty(start), prefix);
                }
                match function_evaluator(name) {
                    Some(Evaluator::Dynamic(DynamicFunction::Let)) => {
                        self.visit_let(args, node.children());
                    }
                    Some(Evaluator::Dynamic(DynamicFunction::Lambda)) => {
                        self.visit_lambda(args, node.children());
                    }
                    _ => self.visit_children(expr, node),
                }
            }
            Expr::Name(name) => {
                if is_local_name(name, &self.scope) {
                    self.prefix_local(name, start);
                }
            }
            Expr::BuiltinCallable(_) => {
                let source = self.source;
                let name = &source[start..node.span().end];
                if is_local_name(name, &self.scope) {
                    self.prefix_local(name, start);
                } else if storage_builtin_callable(name).is_none() {
                    self.edit(SourceSpan::empty(start), "_xleta.");
                }
            }
            Expr::ImplicitIntersection(operand) if self.source[start..].starts_with('@') => {
                let grouped = storage_argument_needs_grouping(operand);
                self.edit(
                    SourceSpan::new(start, start + 1),
                    if grouped {
                        "_xlfn.SINGLE(("
                    } else {
                        "_xlfn.SINGLE("
                    },
                );
                self.visit_children(expr, node);
                self.edit(
                    SourceSpan::empty(node.span().end),
                    if grouped { "))" } else { ")" },
                );
            }
            Expr::SpillRef(_) if self.source[..node.span().end].ends_with('#') => {
                self.edit(SourceSpan::empty(start), "_xlfn.ANCHORARRAY(");
                self.visit_children(expr, node);
                self.edit(SourceSpan::new(node.span().end - 1, node.span().end), ")");
            }
            _ => self.visit_children(expr, node),
        });
    }

    fn visit_children(&mut self, expr: &Expr, node: &NodeSpanTree) {
        let mut children = Vec::new();
        push_children(expr, node, &mut children);
        while let Some((child, child_node)) = children.pop() {
            self.visit(child, child_node, child_node.span().start);
        }
    }

    // Each LET name enters scope after its own value, as in evaluation.
    fn visit_let(&mut self, args: &[Expr], nodes: &[NodeSpanTree]) {
        let outer = self.scope.len();
        let body = args.len().saturating_sub(1);
        let mut declared = None;
        for (index, (arg, node)) in args.iter().zip(nodes).enumerate() {
            match arg {
                Expr::Name(name) if index < body && index % 2 == 0 => {
                    self.prefix_local(name, node.span().start);
                    declared = Some(canonical_local_name(name));
                }
                _ => {
                    self.visit(arg, node, node.span().start);
                    self.scope.extend(declared.take());
                }
            }
        }
        self.scope.truncate(outer);
    }

    fn visit_lambda(&mut self, args: &[Expr], nodes: &[NodeSpanTree]) {
        let outer = self.scope.len();
        let body = args.len().saturating_sub(1);
        for (index, (arg, node)) in args.iter().zip(nodes).enumerate() {
            match arg {
                Expr::Name(name) if index < body => {
                    self.prefix_local(name, node.span().start);
                    self.scope.push(canonical_local_name(name));
                }
                _ => self.visit(arg, node, node.span().start),
            }
        }
        self.scope.truncate(outer);
    }

    fn prefix_local(&mut self, name: &str, start: usize) {
        if !name
            .get(..LOCAL_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(LOCAL_PREFIX))
        {
            self.edit(SourceSpan::empty(start), LOCAL_PREFIX);
        }
    }

    fn edit(&mut self, span: SourceSpan, text: &'static str) {
        self.edits.push(StorageEdit { span, text });
    }
}

#[cfg(test)]
mod tests {
    use super::storage_formula_text;
    use crate::calculation::CalculationLimits;
    use crate::calculation::parser::parse_formula_with_limits;

    #[test]
    fn generated_formulas_keep_parsing_and_convert_once() {
        const PIECES: [&str; 13] = [
            "@",
            "#",
            "(",
            ")",
            ",",
            "A1",
            "x",
            "SUM",
            "LET(x,",
            "LAMBDA(x,",
            "BYROW(A1,",
            ":",
            " ",
        ];
        let mut inputs = vec![String::new()];
        let mut converted = 0;
        for _ in 0..5 {
            let mut next = Vec::with_capacity(inputs.len() * PIECES.len());
            for input in &inputs {
                for piece in PIECES {
                    let candidate = format!("{input}{piece}");
                    let stored = storage_formula_text(&candidate);
                    if stored != candidate {
                        converted += 1;
                        parse_formula_with_limits(&stored, CalculationLimits::default())
                            .unwrap_or_else(|error| panic!("{candidate} -> {stored}: {error:?}"));
                        assert_eq!(storage_formula_text(&stored), stored, "{candidate}");
                    }
                    next.push(candidate);
                }
            }
            inputs = next;
        }
        assert!(converted > 0);
    }
}
