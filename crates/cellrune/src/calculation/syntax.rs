use std::fmt;
use std::sync::Arc;

use super::ast::{Expr, FormulaDisplayMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn empty(offset: usize) -> Self {
        Self::new(offset, offset)
    }

    pub const fn merge(self, other: Self) -> Self {
        Self::new(
            if self.start < other.start {
                self.start
            } else {
                other.start
            },
            if self.end > other.end {
                self.end
            } else {
                other.end
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSpanTree {
    span: SourceSpan,
    components: Arc<[SourceComponent]>,
    children: Arc<[NodeSpanTree]>,
}

impl NodeSpanTree {
    pub fn span(&self) -> SourceSpan {
        self.span
    }

    pub fn children(&self) -> &[Self] {
        &self.children
    }

    #[allow(dead_code)]
    pub fn components(&self) -> &[SourceComponent] {
        &self.components
    }

    pub(super) fn from_postorder<'a>(
        expr: &Expr,
        sources: &mut impl Iterator<Item = &'a PendingNodeSource>,
    ) -> Option<Self> {
        let source = sources.next()?;
        let mut children = match expr {
            Expr::Call { args, .. } => args
                .iter()
                .rev()
                .map(|child| Self::from_postorder(child, sources))
                .collect::<Option<Vec<_>>>()?,
            Expr::Invoke { callee, args } => {
                let mut children = args
                    .iter()
                    .rev()
                    .map(|child| Self::from_postorder(child, sources))
                    .collect::<Option<Vec<_>>>()?;
                children.push(Self::from_postorder(callee, sources)?);
                children
            }
            Expr::ImplicitIntersection(child)
            | Expr::SpillRef(child)
            | Expr::Unary { operand: child, .. }
            | Expr::Paren(child) => vec![Self::from_postorder(child, sources)?],
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right } => vec![
                Self::from_postorder(right, sources)?,
                Self::from_postorder(left, sources)?,
            ],
            Expr::Range { start, end } => vec![
                Self::from_postorder(end, sources)?,
                Self::from_postorder(start, sources)?,
            ],
            Expr::Array(rows) => rows
                .iter()
                .flatten()
                .rev()
                .map(|child| Self::from_postorder(child, sources))
                .collect::<Option<Vec<_>>>()?,
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Name(_)
            | Expr::Missing => Vec::new(),
        };
        children.reverse();
        Some(Self {
            span: source.span,
            components: source.components.clone().into(),
            children: children.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceComponentKind {
    StructuredTable,
    StructuredItem(u16),
    StructuredColumn,
    StructuredColumnStart,
    StructuredColumnEnd,
    SheetQualifier,
    SheetRangeEnd,
    DefinedName,
    ExternalWorkbook,
    ExternalSheet,
    ExternalSheetRangeEnd,
    ReferenceBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceComponent {
    kind: SourceComponentKind,
    span: SourceSpan,
}

impl SourceComponent {
    pub const fn new(kind: SourceComponentKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }

    #[allow(dead_code)]
    pub const fn kind(self) -> SourceComponentKind {
        self.kind
    }

    #[allow(dead_code)]
    pub const fn span(self) -> SourceSpan {
        self.span
    }

    pub(super) const fn shifted(self, offset: usize) -> Self {
        Self::new(
            self.kind,
            SourceSpan::new(self.span.start + offset, self.span.end + offset),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingNodeSource {
    pub span: SourceSpan,
    pub components: Vec<SourceComponent>,
    pub depth: u64,
    pub subtree_nodes: usize,
}

impl PendingNodeSource {
    pub const fn new(
        span: SourceSpan,
        components: Vec<SourceComponent>,
        depth: u64,
        subtree_nodes: usize,
    ) -> Self {
        Self {
            span,
            components,
            depth,
            subtree_nodes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaSourceMap {
    token_spans: Arc<[SourceSpan]>,
    trivia_spans: Arc<[SourceSpan]>,
    node_tree: NodeSpanTree,
}

impl FormulaSourceMap {
    pub(super) fn new(
        token_spans: Vec<SourceSpan>,
        trivia_spans: Vec<SourceSpan>,
        node_tree: NodeSpanTree,
    ) -> Self {
        debug_assert!(
            node_tree
                .children()
                .iter()
                .all(|child| child.span().start >= node_tree.span().start
                    && child.span().end <= node_tree.span().end)
        );
        Self {
            token_spans: token_spans.into(),
            trivia_spans: trivia_spans.into(),
            node_tree,
        }
    }

    #[allow(dead_code)]
    pub fn token_spans(&self) -> &[SourceSpan] {
        &self.token_spans
    }

    #[allow(dead_code)]
    pub fn trivia_spans(&self) -> &[SourceSpan] {
        &self.trivia_spans
    }

    #[allow(dead_code)]
    pub fn node_tree(&self) -> &NodeSpanTree {
        &self.node_tree
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFormula {
    original: Arc<str>,
    root: Expr,
    source_map: FormulaSourceMap,
}

impl ParsedFormula {
    pub(super) fn new(original: Arc<str>, root: Expr, source_map: FormulaSourceMap) -> Self {
        Self {
            original,
            root,
            source_map,
        }
    }

    #[allow(dead_code)]
    pub fn original(&self) -> &str {
        &self.original
    }

    pub fn root(&self) -> &Expr {
        &self.root
    }

    #[allow(dead_code)]
    pub fn source_map(&self) -> &FormulaSourceMap {
        &self.source_map
    }

    #[allow(dead_code)]
    pub(super) const fn display_with_mode(
        &self,
        mode: FormulaDisplayMode,
    ) -> ParsedFormulaDisplay<'_> {
        ParsedFormulaDisplay {
            formula: self,
            mode,
        }
    }
}

#[allow(dead_code)]
pub(super) struct ParsedFormulaDisplay<'a> {
    formula: &'a ParsedFormula,
    mode: FormulaDisplayMode,
}

impl fmt::Display for ParsedFormulaDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.formula
            .root
            .display_with_mode(self.mode)
            .fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct SourceEdit {
    pub span: SourceSpan,
    pub replacement: Box<str>,
}

impl SourceEdit {
    #[allow(dead_code)]
    pub fn new(span: SourceSpan, replacement: impl Into<Box<str>>) -> Self {
        Self {
            span,
            replacement: replacement.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SourceEditError {
    InvalidSpan,
    NonUtf8Boundary,
    OverlappingEdits,
}

impl SourceEditError {
    #[allow(dead_code)]
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidSpan => "source edit span is outside the formula",
            Self::NonUtf8Boundary => "source edit span is not on UTF-8 boundaries",
            Self::OverlappingEdits => "source edits overlap",
        }
    }
}

#[allow(dead_code)]
pub fn apply_source_edits(original: &str, edits: &[SourceEdit]) -> Result<String, SourceEditError> {
    let mut ordered: Vec<&SourceEdit> = edits.iter().collect();
    ordered.sort_by_key(|edit| (edit.span.start, edit.span.end));
    let mut previous: Option<SourceSpan> = None;
    for edit in &ordered {
        if edit.span.start > edit.span.end || edit.span.end > original.len() {
            return Err(SourceEditError::InvalidSpan);
        }
        if !original.is_char_boundary(edit.span.start) || !original.is_char_boundary(edit.span.end)
        {
            return Err(SourceEditError::NonUtf8Boundary);
        }
        if let Some(previous_span) = previous {
            let duplicate_insertion = previous_span.start == previous_span.end
                && edit.span.start == edit.span.end
                && previous_span.start == edit.span.start;
            if edit.span.start < previous_span.end || duplicate_insertion {
                return Err(SourceEditError::OverlappingEdits);
            }
        }
        previous = Some(edit.span);
    }

    let mut result = original.to_owned();
    for edit in ordered.into_iter().rev() {
        result.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{SourceEdit, SourceEditError, SourceSpan, apply_source_edits};

    #[test]
    fn applies_non_overlapping_utf8_edits_from_the_end() {
        let edits = [
            SourceEdit::new(SourceSpan::new(0, 3), "합계"),
            SourceEdit::new(SourceSpan::new(8, 11), "B2"),
        ];
        assert_eq!(
            apply_source_edits("SUM(A1)+표", &edits).expect("valid edits"),
            "합계(A1)+B2"
        );
    }

    #[test]
    fn rejects_non_boundary_and_overlapping_edits() {
        assert_eq!(
            apply_source_edits("표A1", &[SourceEdit::new(SourceSpan::new(1, 3), "x")]),
            Err(SourceEditError::NonUtf8Boundary)
        );
        assert_eq!(
            apply_source_edits(
                "A1+B1",
                &[
                    SourceEdit::new(SourceSpan::new(0, 3), "x"),
                    SourceEdit::new(SourceSpan::new(2, 4), "y"),
                ]
            ),
            Err(SourceEditError::OverlappingEdits)
        );
        assert_eq!(
            apply_source_edits(
                "A1",
                &[
                    SourceEdit::new(SourceSpan::empty(1), "x"),
                    SourceEdit::new(SourceSpan::empty(1), "y"),
                ]
            ),
            Err(SourceEditError::OverlappingEdits)
        );
    }
}
