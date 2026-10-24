//! Reference rewrites for structural edits of a sheet's grid and for copied formulas.
//!
//! A grid edit moves the cells a reference names, so the reference body is rewritten by the
//! range rule of [`GridEdit`]; a reference whose cells are deleted becomes `#REF!`. A copy shifts
//! only the relative parts of every reference by the copy offset. Both edit reference bodies in
//! place and leave the rest of the formula text as it was written.

use super::ast::{CellRef, ColRef, Expr, ExternalReferenceTarget, RefBody, Reference, RowRef};
use super::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteRequest, rewrite_formula,
};
use super::syntax::{NodeSpanTree, SourceComponent, SourceComponentKind, SourceEdit, SourceSpan};
use crate::case_insensitive_eq;
use crate::grid::{
    GridAxis, GridEdit, RangeRule, intersect, offset_range, range_contains, subtract,
};
use crate::{CellAddress, CellRange, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS};

/// The text a reference to deleted cells becomes, as Excel saves it.
pub(super) const DELETED_REFERENCE: &str = "#REF!";

/// A structural edit of one sheet's grid, as the formulas of a workbook see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridChange<'a> {
    /// The edited sheet, as formulas name it.
    pub(crate) sheet: &'a str,
    /// Whether the rewritten formula belongs to the edited sheet, so that its unqualified
    /// references name that sheet.
    pub(crate) host_is_sheet: bool,
    pub(crate) edit: GridEdit,
}

/// The row and column distance a copied formula moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopyOffset {
    pub(crate) rows: i64,
    pub(crate) columns: i64,
}

/// Whether the rewrite still has to visit a node's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NodePlan {
    Done,
    Descend,
}

/// A copied reference would leave the grid, which turns the whole formula into `#REF!`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OffGrid;

enum Outcome {
    Unchanged,
    Changed(RefBody),
    Deleted,
}

/// Moves cells whose formulas read relative to their first cell, as a conditional format or a
/// data validation does, through a structural edit of their own sheet. Returns the rules the cells
/// make afterwards: none when the edit deletes every cell, else the rule itself, and for a range
/// move that cuts away part of its cells, that part as a second rule.
///
/// While the first cell survives, the formulas move by the range rule. Once it is deleted, they
/// are first shifted to the cell that now comes first, as if copied there, so each cell keeps
/// reading the cells it read. A part a range move cuts away reads as the rule did there: its
/// formulas are shifted to its first cell before the move, then follow the move, as Excel 16.114
/// splits the rule (excel-measure I4: cutting A2:A3 of `A1>B1` on A1:A6 to D2 leaves the rule on A1
/// and A4:A6 and adds `D2>B2` on D2:D3).
pub(crate) fn move_anchored_formulas(
    ranges: &[CellRange],
    formulas: &[&str],
    change: GridChange<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Vec<AnchoredFormulas>, FormulaRewriteError> {
    let edit = change.edit;
    let Some(old_anchor) = ranges.first().map(|range| range.start()) else {
        return Ok(Vec::new());
    };
    let request = FormulaRewriteRequest::Grid(GridChange {
        host_is_sheet: true,
        ..change
    });
    let (kept, cut) = match edit {
        GridEdit::MoveRange { source, .. } => {
            let overwritten = edit.overwritten();
            let mut kept = Vec::new();
            for range in ranges {
                let mut pieces = subtract(*range, source);
                for piece in &overwritten {
                    pieces = pieces
                        .into_iter()
                        .flat_map(|left| subtract(left, *piece))
                        .collect();
                }
                kept.extend(pieces);
            }
            let cut = ranges
                .iter()
                .filter_map(|range| intersect(*range, source))
                .collect::<Vec<_>>();
            (kept, cut)
        }
        GridEdit::Insert { .. } | GridEdit::Delete { .. } | GridEdit::Move { .. } => {
            (edit.map_ranges(ranges, RangeRule::Format), Vec::new())
        }
    };
    let mut rules = Vec::new();
    if let Some(new_anchor) = kept.first().map(|range| range.start()) {
        // A range move keeps the cells it leaves where they are.
        let origin = match edit {
            GridEdit::MoveRange { .. } => Some(new_anchor),
            _ => (edit.map_address(old_anchor) != Some(new_anchor))
                .then(|| edit.unmap_address(new_anchor))
                .flatten(),
        };
        let rebase = origin
            .filter(|origin| *origin != old_anchor)
            .map(|origin| offset_between(old_anchor, origin));
        rules.push(AnchoredFormulas {
            formulas: rebased_and_moved(formulas, rebase, &request, budget)?,
            ranges: kept,
        });
    }
    if let Some(origin) = cut.first().map(|range| range.start()) {
        let rebase = (origin != old_anchor).then(|| offset_between(old_anchor, origin));
        let ranges = cut
            .iter()
            .filter_map(|range| offset_range(*range, edit.range_offset()))
            .collect();
        rules.push(AnchoredFormulas {
            formulas: rebased_and_moved(formulas, rebase, &request, budget)?,
            ranges,
        });
    }
    Ok(rules)
}

fn offset_between(from: CellAddress, to: CellAddress) -> CopyOffset {
    CopyOffset {
        rows: i64::from(to.row().get()) - i64::from(from.row().get()),
        columns: i64::from(to.column().get()) - i64::from(from.column().get()),
    }
}

/// Shifts each formula as if copied by `rebase`, then moves it by the edit; `None` where the text
/// stays the same.
fn rebased_and_moved(
    formulas: &[&str],
    rebase: Option<CopyOffset>,
    request: &FormulaRewriteRequest<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Vec<Option<String>>, FormulaRewriteError> {
    let mut rewritten = Vec::with_capacity(formulas.len());
    for formula in formulas {
        let mut text = None::<String>;
        if let Some(offset) = rebase {
            text = rewrite_formula(formula, &FormulaRewriteRequest::Copy(offset), budget)?;
        }
        let current = text.as_deref().unwrap_or(formula);
        if let Some(moved) = rewrite_formula(current, request, budget)? {
            text = Some(moved);
        }
        rewritten.push(text);
    }
    Ok(rewritten)
}

/// Cells with formulas that read relative to their first cell, after a structural edit.
pub(crate) struct AnchoredFormulas {
    pub(crate) ranges: Vec<CellRange>,
    /// Each formula's rewritten text, `None` where it is unchanged.
    pub(crate) formulas: Vec<Option<String>>,
}

/// Reports whether a formula holds a spill reference (`A1#` or `ANCHORARRAY`), whose copying
/// Excel was not measured for. A formula that does not parse holds none.
pub(crate) fn contains_spill_reference(source: &str) -> bool {
    let Ok(parsed) =
        super::parser::parse_formula_with_limits(source, super::CalculationLimits::default())
    else {
        return false;
    };
    let mut stack = vec![(parsed.root(), parsed.source_map().node_tree())];
    while let Some((expr, node)) = stack.pop() {
        if matches!(expr, Expr::SpillRef(_)) {
            return true;
        }
        super::formula_rewrite::push_children(expr, node, &mut stack);
    }
    false
}

/// Plans the edits a grid change makes to one syntax node.
pub(super) fn plan_grid_node(
    source: &str,
    expr: &Expr,
    node: &NodeSpanTree,
    change: &GridChange<'_>,
    edits: &mut Vec<SourceEdit>,
) -> NodePlan {
    match expr {
        Expr::Ref(reference) => {
            if !targets(reference, change) {
                return NodePlan::Done;
            }
            match outcome(reference.body, change.edit) {
                Outcome::Unchanged => {}
                Outcome::Changed(body) => {
                    if let Some(span) = body_span(node.components()) {
                        edits.push(SourceEdit::new(span, render_body_over(source, span, body)));
                    }
                }
                Outcome::Deleted => edits.push(deleted_reference(node)),
            }
            NodePlan::Done
        }
        Expr::Range { start, end } => {
            let children = node.children();
            let (Some(start_node), Some(end_node)) = (
                children.first().map(|child| unparenthesized(start, child)),
                children.get(1).map(|child| unparenthesized(end, child)),
            ) else {
                return NodePlan::Descend;
            };
            let start_ref = targeted_reference(start, change);
            let end_ref = targeted_reference(end, change);
            if let (
                Some(Reference {
                    body: RefBody::Cell(first),
                    ..
                }),
                Some(Reference {
                    body: RefBody::Cell(last),
                    ..
                }),
            ) = (start_ref, end_ref)
            {
                // `A1:Sheet1!B2` and `(A1):B2` name one area, which the range rule changes as a
                // whole.
                match outcome(RefBody::Area(*first, *last), change.edit) {
                    Outcome::Unchanged => {}
                    Outcome::Changed(RefBody::Area(first, last)) => {
                        for (child, cell) in [(start_node, first), (end_node, last)] {
                            if let Some(span) = body_span(child.components()) {
                                edits.push(SourceEdit::new(span, render_body(RefBody::Cell(cell))));
                            }
                        }
                    }
                    Outcome::Changed(_) => unreachable!("an area stays an area"),
                    Outcome::Deleted => {
                        edits.push(SourceEdit::new(
                            node.span(),
                            format!("{}{DELETED_REFERENCE}", qualifier_text(source, start_node)),
                        ));
                    }
                }
                return NodePlan::Done;
            }
            for (reference, child) in [(start_ref, start_node), (end_ref, end_node)] {
                if let Some(reference) = reference
                    && matches!(outcome(reference.body, change.edit), Outcome::Deleted)
                {
                    edits.push(SourceEdit::new(
                        node.span(),
                        format!("{}{DELETED_REFERENCE}", qualifier_text(source, child)),
                    ));
                    return NodePlan::Done;
                }
            }
            NodePlan::Descend
        }
        Expr::SpillRef(operand)
        | Expr::Invoke {
            callee: operand, ..
        } => {
            let Some(child) = node.children().first() else {
                return NodePlan::Descend;
            };
            if let Some(reference) = targeted_reference(operand, change)
                && matches!(outcome(reference.body, change.edit), Outcome::Deleted)
            {
                // `A1#` cannot spill from `#REF!`, so the whole spill reference becomes one.
                edits.push(SourceEdit::new(
                    node.span(),
                    format!("{}{DELETED_REFERENCE}", qualifier_text(source, child)),
                ));
                return NodePlan::Done;
            }
            NodePlan::Descend
        }
        Expr::ExternalReference(_) => NodePlan::Done,
        _ => NodePlan::Descend,
    }
}

/// Plans the edits a copy makes to one syntax node: every relative row and column part moves by
/// the offset. Structured references, names, and text stay as they are.
pub(super) fn plan_copy_node(
    source: &str,
    expr: &Expr,
    node: &NodeSpanTree,
    offset: CopyOffset,
    edits: &mut Vec<SourceEdit>,
) -> Result<NodePlan, OffGrid> {
    let body = match expr {
        Expr::Ref(reference) => reference.body,
        Expr::ExternalReference(reference) => match &reference.target {
            ExternalReferenceTarget::Reference(body) => *body,
            ExternalReferenceTarget::DefinedName(_)
            | ExternalReferenceTarget::StructuredReference(_) => return Ok(NodePlan::Done),
        },
        _ => return Ok(NodePlan::Descend),
    };
    let shifted = shift_body(body, offset)?;
    if shifted != body
        && let Some(span) = body_span(node.components())
    {
        edits.push(SourceEdit::new(
            span,
            render_body_over(source, span, shifted),
        ));
    }
    Ok(NodePlan::Done)
}

/// Reports whether a reference names the edited sheet: unqualified on that sheet, or qualified
/// with its name. A 3-D reference spans tabs, not cells of one grid, and is never changed.
fn targets(reference: &Reference, change: &GridChange<'_>) -> bool {
    match &reference.sheet {
        None => change.host_is_sheet,
        Some(prefix) => {
            prefix.end_name.is_none()
                && prefix.external_workbook_detail().is_none()
                && case_insensitive_eq(&prefix.name, change.sheet)
        }
    }
}

/// Returns the node of the expression inside any parentheses, so the end of `(A1):B2` is edited
/// where its reference is written.
fn unparenthesized<'a>(mut expr: &Expr, mut node: &'a NodeSpanTree) -> &'a NodeSpanTree {
    while let (Expr::Paren(inner), Some(child)) = (expr, node.children().first()) {
        expr = inner;
        node = child;
    }
    node
}

fn targeted_reference<'a>(mut expr: &'a Expr, change: &GridChange<'_>) -> Option<&'a Reference> {
    while let Expr::Paren(inner) = expr {
        expr = inner;
    }
    match expr {
        Expr::Ref(reference) if targets(reference, change) => Some(reference),
        _ => None,
    }
}

fn outcome(body: RefBody, edit: GridEdit) -> Outcome {
    match edit.line_axis() {
        Some(axis) => line_outcome(body, edit, axis),
        None => moved_range_outcome(body, edit),
    }
}

fn line_outcome(body: RefBody, edit: GridEdit, axis: GridAxis) -> Outcome {
    let changed = match body {
        RefBody::Cell(mut cell) => {
            let coordinate = cell_coordinate(&mut cell, axis);
            let Some(mapped) = edit.map_line(*coordinate) else {
                return Outcome::Deleted;
            };
            *coordinate = mapped;
            RefBody::Cell(cell)
        }
        RefBody::Area(mut first, mut last) => {
            let first_coordinate = *cell_coordinate(&mut first, axis);
            let last_coordinate = *cell_coordinate(&mut last, axis);
            let Some((first_value, last_value)) =
                mapped_ordered(edit, first_coordinate, last_coordinate)
            else {
                return Outcome::Deleted;
            };
            *cell_coordinate(&mut first, axis) = first_value;
            *cell_coordinate(&mut last, axis) = last_value;
            RefBody::Area(first, last)
        }
        RefBody::Columns(mut first, mut last) if axis == GridAxis::Columns => {
            let Some((first_value, last_value)) = mapped_ordered(edit, first.column, last.column)
            else {
                return Outcome::Deleted;
            };
            first.column = first_value;
            last.column = last_value;
            RefBody::Columns(first, last)
        }
        RefBody::Rows(mut first, mut last) if axis == GridAxis::Rows => {
            let Some((first_value, last_value)) = mapped_ordered(edit, first.row, last.row) else {
                return Outcome::Deleted;
            };
            first.row = first_value;
            last.row = last_value;
            RefBody::Rows(first, last)
        }
        RefBody::Columns(..) | RefBody::Rows(..) => body,
    };
    if changed == body {
        Outcome::Unchanged
    } else {
        Outcome::Changed(changed)
    }
}

fn mapped_ordered(edit: GridEdit, first: u32, last: u32) -> Option<(u32, u32)> {
    let (low, high) = ordered(first, last);
    let (low, high) = edit.map_interval(low, high, RangeRule::Reference)?;
    Some(if first <= last {
        (low, high)
    } else {
        (high, low)
    })
}

/// Moves a reference through a range move. A reference wholly inside the moved cells follows them.
/// A cell or area the paste overwrites outside the moved cells, as Excel 16.114 measures
/// (excel-measure I3): becomes `#REF!` when the overwritten cells cover it, shrinks to its part
/// left when they cover its whole width (or height) and run past one of its ends, and otherwise
/// keeps its text, as it does when they only reach its end or lie inside it.
fn moved_range_outcome(body: RefBody, edit: GridEdit) -> Outcome {
    let GridEdit::MoveRange { source, .. } = edit else {
        return Outcome::Unchanged;
    };
    let Some(area) = body_area(body) else {
        return Outcome::Unchanged;
    };
    if range_contains(source, area) {
        let (rows, columns) = edit.range_offset();
        let shift = |value: u32, delta: i64| {
            u32::try_from(i64::from(value) + delta).expect("a validated move stays on the sheet")
        };
        let shift_cell = |cell: CellRef| CellRef {
            row: shift(cell.row, rows),
            column: shift(cell.column, columns),
            ..cell
        };
        return Outcome::Changed(match body {
            RefBody::Cell(cell) => RefBody::Cell(shift_cell(cell)),
            RefBody::Area(first, last) => RefBody::Area(shift_cell(first), shift_cell(last)),
            RefBody::Columns(first, last) => RefBody::Columns(
                ColRef {
                    column: shift(first.column, columns),
                    ..first
                },
                ColRef {
                    column: shift(last.column, columns),
                    ..last
                },
            ),
            RefBody::Rows(first, last) => RefBody::Rows(
                RowRef {
                    row: shift(first.row, rows),
                    ..first
                },
                RowRef {
                    row: shift(last.row, rows),
                    ..last
                },
            ),
        });
    }
    // A whole row or column keeps its text however its cells change.
    if !matches!(body, RefBody::Cell(_) | RefBody::Area(..)) {
        return Outcome::Unchanged;
    }
    let (mut top, mut left, mut bottom, mut right) = (
        area.start().row().get(),
        area.start().column().get(),
        area.end().row().get(),
        area.end().column().get(),
    );
    for piece in edit.overwritten() {
        let (piece_top, piece_left, piece_bottom, piece_right) = (
            piece.start().row().get(),
            piece.start().column().get(),
            piece.end().row().get(),
            piece.end().column().get(),
        );
        if piece_top <= top && bottom <= piece_bottom && piece_left <= left && right <= piece_right
        {
            return Outcome::Deleted;
        }
        if piece_left <= left && right <= piece_right {
            (top, bottom) = shrunk(top, bottom, piece_top, piece_bottom);
        } else if piece_top <= top && bottom <= piece_bottom {
            (left, right) = shrunk(left, right, piece_left, piece_right);
        }
    }
    let unchanged = (top, left, bottom, right)
        == (
            area.start().row().get(),
            area.start().column().get(),
            area.end().row().get(),
            area.end().column().get(),
        );
    match body {
        RefBody::Area(mut first, mut last) if !unchanged => {
            // Each end keeps its own side, so `A6:A4` shrinks as `A5:A4`.
            let (first_row, last_row) = if first.row <= last.row {
                (top, bottom)
            } else {
                (bottom, top)
            };
            let (first_column, last_column) = if first.column <= last.column {
                (left, right)
            } else {
                (right, left)
            };
            (first.row, first.column, last.row, last.column) =
                (first_row, first_column, last_row, last_column);
            Outcome::Changed(RefBody::Area(first, last))
        }
        _ => Outcome::Unchanged,
    }
}

/// Shrinks `start..=end` past overwritten lines `from..=to` that cover one of its ends and run past
/// it; lines that reach no further than the interval's ends leave it whole.
fn shrunk(start: u32, end: u32, from: u32, to: u32) -> (u32, u32) {
    if start < from && from <= end && end < to {
        (start, from - 1)
    } else if from < start && start <= to && to < end {
        (to + 1, end)
    } else {
        (start, end)
    }
}

/// Returns the cells a reference body names, with whole rows and columns spanning the sheet.
fn body_area(body: RefBody) -> Option<CellRange> {
    let (top, left, bottom, right) = match body {
        RefBody::Cell(cell) => (cell.row, cell.column, cell.row, cell.column),
        RefBody::Area(first, last) => {
            let (top, bottom) = ordered(first.row, last.row);
            let (left, right) = ordered(first.column, last.column);
            (top, left, bottom, right)
        }
        RefBody::Columns(first, last) => {
            let (left, right) = ordered(first.column, last.column);
            (1, left, EXCEL_MAX_ROWS, right)
        }
        RefBody::Rows(first, last) => {
            let (top, bottom) = ordered(first.row, last.row);
            (top, 1, bottom, EXCEL_MAX_COLUMNS)
        }
    };
    Some(CellRange::from_ordered(
        CellAddress::from_indices(top, left).ok()?,
        CellAddress::from_indices(bottom, right).ok()?,
    ))
}

fn shift_body(body: RefBody, offset: CopyOffset) -> Result<RefBody, OffGrid> {
    let shift = |value: u32, delta: i64, absolute: bool, maximum: u32| {
        if absolute {
            return Ok(value);
        }
        let shifted = i64::from(value) + delta;
        if (1..=i64::from(maximum)).contains(&shifted) {
            Ok(u32::try_from(shifted).expect("a coordinate on the sheet fits in u32"))
        } else {
            Err(OffGrid)
        }
    };
    let cell = |cell: CellRef| -> Result<CellRef, OffGrid> {
        Ok(CellRef {
            row: shift(cell.row, offset.rows, cell.row_absolute, EXCEL_MAX_ROWS)?,
            column: shift(
                cell.column,
                offset.columns,
                cell.column_absolute,
                EXCEL_MAX_COLUMNS,
            )?,
            ..cell
        })
    };
    let column = |column: ColRef| -> Result<ColRef, OffGrid> {
        Ok(ColRef {
            column: shift(
                column.column,
                offset.columns,
                column.absolute,
                EXCEL_MAX_COLUMNS,
            )?,
            ..column
        })
    };
    let row = |row: RowRef| -> Result<RowRef, OffGrid> {
        Ok(RowRef {
            row: shift(row.row, offset.rows, row.absolute, EXCEL_MAX_ROWS)?,
            ..row
        })
    };
    Ok(match body {
        RefBody::Cell(first) => RefBody::Cell(cell(first)?),
        RefBody::Area(first, last) => RefBody::Area(cell(first)?, cell(last)?),
        RefBody::Columns(first, last) => RefBody::Columns(column(first)?, column(last)?),
        RefBody::Rows(first, last) => RefBody::Rows(row(first)?, row(last)?),
    })
}

const fn cell_coordinate(cell: &mut CellRef, axis: GridAxis) -> &mut u32 {
    match axis {
        GridAxis::Rows => &mut cell.row,
        GridAxis::Columns => &mut cell.column,
    }
}

const fn ordered(first: u32, last: u32) -> (u32, u32) {
    if first <= last {
        (first, last)
    } else {
        (last, first)
    }
}

fn render_body(body: RefBody) -> String {
    Reference { sheet: None, body }.to_string()
}

/// Renders a changed body in place of the body written at `span`, keeping the dots of a trim
/// colon such as `A1:.B9`.
fn render_body_over(source: &str, span: SourceSpan, body: RefBody) -> String {
    let rendered = render_body(body);
    let written = &source[span.start..span.end];
    let Some(colon) = written.rfind(':') else {
        return rendered;
    };
    let start = colon - usize::from(written[..colon].ends_with('.'));
    let end = colon + 1 + usize::from(written[colon + 1..].starts_with('.'));
    rendered.replacen(':', &written[start..end], 1)
}

fn body_span(components: &[SourceComponent]) -> Option<SourceSpan> {
    components
        .iter()
        .find(|component| component.kind() == SourceComponentKind::ReferenceBody)
        .map(|component| component.span())
}

/// Returns the edit that turns a reference to deleted cells into `#REF!`, keeping the sheet of
/// a qualified reference as Excel does (`'S1'!#REF!`).
fn deleted_reference(node: &NodeSpanTree) -> SourceEdit {
    match body_span(node.components()) {
        Some(span) if span.start > node.span().start => {
            SourceEdit::new(span, DELETED_REFERENCE.to_owned())
        }
        _ => SourceEdit::new(node.span(), DELETED_REFERENCE.to_owned()),
    }
}

/// Returns the `Sheet!` text in front of a reference's body, or nothing for an unqualified one.
fn qualifier_text<'a>(source: &'a str, node: &NodeSpanTree) -> &'a str {
    body_span(node.components())
        .filter(|span| span.start > node.span().start)
        .and_then(|span| source.get(node.span().start..span.start))
        .unwrap_or("")
}

#[cfg(test)]
#[path = "grid_rewrite_tests.rs"]
mod tests;
