//! Replays a draft's structural grid edits on every preserved part that holds the coordinates
//! they move: the edited worksheet, its drawings, legacy notes, comments, and threaded comments,
//! and the references other worksheets, charts, and pivot caches make to it.

use std::collections::{BTreeMap, BTreeSet};

use super::grid_worksheet::{parse_range, patch_worksheet_grid, unrewritable};
use super::grid_xml::{XmlDocument, XmlElement, XmlNode};
use super::package::PackageReader;
use super::sheet_reference_patch::{
    DETAIL_MOVED_LINK, PartAccess, links_rewritten_cells, patch_grid_references,
    rewrite_grid_references,
};
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::calculation::formula_rewrite::FormulaRewriteRequest;
use crate::calculation::grid_rewrite::GridChange;
use crate::grid::{GridAxis, GridEdit, GridHistory, RangeRule};
use crate::xlsx::package::{PartPath, RelationshipTarget, parse_relationships};
use crate::{CellAddress, CellRange, WorkbookDraft, XlsxDocument};

const DETAIL_MISSING_SHEET_PART: &str = "source worksheet part mapping was not found";
const DETAIL_UNREWRITABLE_PART: &str =
    "a structural edit cannot rewrite the coordinates of a preserved part";
const DETAIL_MOVED_PIVOT_TABLE: &str = "a structural edit moves or splits a preserved pivot table";
const DETAIL_FORM_CONTROL: &str = "a structural edit cannot move a preserved form control";
const DETAIL_ANCHOR_OFF_GRID: &str = "a drawing anchor names a cell outside the worksheet grid";

/// How a part related to an edited worksheet holds its coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelatedKind {
    Drawing,
    LegacyDrawing,
    Comments,
    ThreadedComments,
    PivotTable,
}

/// The relationship types of a worksheet whose targets hold no cell coordinates, or whose
/// coordinates the workbook model owns, as the last segment of the type.
const KEPT_RELATIONSHIPS: &[&str] = &[
    "table",
    "hyperlink",
    "printerSettings",
    "image",
    "customProperty",
    "slicer",
    "timeline",
    "webextension",
];

/// Replays every structural edit the draft recorded for a source worksheet on the parts that
/// hold the coordinates it moves, adding the rewritten parts to `replacements`.
pub(super) fn replay_grid_edits(
    draft: &WorkbookDraft,
    document: &XlsxDocument,
    source_reader: &mut PackageReader<'_>,
    replacements: &mut BTreeMap<PartPath, Vec<u8>>,
    removed_parts: &BTreeSet<PartPath>,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    for (sheet_id, history) in draft.grid_histories() {
        let (Some(draft_sheet), Some(source_sheet)) = (
            draft.workbook().sheet_by_id(*sheet_id),
            document.workbook().sheet_by_id(*sheet_id),
        ) else {
            continue;
        };
        let part = document
            .worksheet_part_path(*sheet_id)
            .cloned()
            .ok_or_else(|| {
                XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                    .with_detail(DETAIL_MISSING_SHEET_PART)
            })?;
        let related = related_parts(document, &part, source_reader)?;
        let name = source_sheet.name().as_str();
        let mut bytes = match replacements.get(&part) {
            Some(bytes) => bytes.clone(),
            None => source_reader.read_part(&part)?,
        };
        for edit in history.edits() {
            bytes =
                patch_worksheet_grid(&bytes, &part, *edit, name, draft_sheet.used_range(), limits)?;
            for (related_part, kind) in &related {
                let current = match replacements.get(related_part) {
                    Some(bytes) => bytes.clone(),
                    None => source_reader.read_part(related_part)?,
                };
                // The sheet's own linked shapes and VML controls name its cells unqualified.
                if matches!(kind, RelatedKind::Drawing | RelatedKind::LegacyDrawing)
                    && links_rewritten_cells(
                        &current,
                        related_part,
                        &FormulaRewriteRequest::Grid(GridChange {
                            sheet: name,
                            host_is_sheet: true,
                            edit: *edit,
                        }),
                        limits,
                    )?
                {
                    return Err(unrewritable(
                        related_part,
                        DETAIL_MOVED_LINK,
                        related_part.as_str(),
                    ));
                }
                if let Some(patched) =
                    patch_related_part(&current, related_part, *kind, *edit, limits)?
                {
                    replacements.insert(related_part.clone(), patched);
                }
            }
            rewrite_grid_references(
                document,
                &part,
                GridChange {
                    sheet: name,
                    host_is_sheet: false,
                    edit: *edit,
                },
                PartAccess {
                    source_reader,
                    replacements,
                    removed_parts,
                },
                limits,
            )?;
        }
        replacements.insert(part, bytes);
    }
    Ok(())
}

/// Replays a copied sheet's structural edits, the ones its source had before the copy included,
/// on the copy's worksheet as the source package holds its origin, whose preserved references
/// still give the copy its origin's name.
pub(super) fn replay_copy_worksheet(
    history: &GridHistory,
    mut worksheet: Vec<u8>,
    part: &PartPath,
    origin_name: &str,
    used_range: Option<CellRange>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    for edit in history.edits() {
        worksheet = patch_worksheet_grid(&worksheet, part, *edit, origin_name, used_range, limits)?;
    }
    Ok(worksheet)
}

/// The parts duplicated for a copied sheet, before the copy is retargeted.
pub(super) struct CopyReplay<'a> {
    /// The origin's name, which the duplicated parts still give the copy.
    pub(super) origin_name: &'a str,
    /// The duplicated parts other than the worksheet.
    pub(super) duplicates: &'a mut BTreeMap<PartPath, Vec<u8>>,
    /// The relationship type that reached each duplicated part, as the last segment of the type.
    pub(super) kinds: &'a BTreeMap<PartPath, Box<str>>,
    pub(super) charts: &'a [PartPath],
}

/// Replays a copied sheet's structural edits on its duplicated drawings, notes, and comments and
/// on the references its duplicated charts make to it.
pub(super) fn replay_copy_parts(
    history: &GridHistory,
    copy: CopyReplay<'_>,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    for edit in history.edits() {
        let change = GridChange {
            sheet: copy.origin_name,
            host_is_sheet: true,
            edit: *edit,
        };
        for (part, bytes) in copy.duplicates.iter_mut() {
            let kind = match copy.kinds.get(part).map(AsRef::as_ref) {
                Some("drawing") => RelatedKind::Drawing,
                Some("vmlDrawing") => RelatedKind::LegacyDrawing,
                Some("comments") => RelatedKind::Comments,
                _ if copy.charts.contains(part) => {
                    if let Some(patched) = patch_grid_references(bytes, part, None, change, limits)?
                    {
                        *bytes = patched;
                    }
                    continue;
                }
                _ => continue,
            };
            if matches!(kind, RelatedKind::Drawing | RelatedKind::LegacyDrawing)
                && links_rewritten_cells(bytes, part, &FormulaRewriteRequest::Grid(change), limits)?
            {
                return Err(unrewritable(part, DETAIL_MOVED_LINK, part.as_str()));
            }
            if let Some(patched) = patch_related_part(bytes, part, kind, *edit, limits)? {
                *bytes = patched;
            }
        }
    }
    Ok(())
}

/// Returns the parts a worksheet relates to whose coordinates a structural edit moves, and
/// refuses relationships whose targets may hold coordinates no pass rewrites.
fn related_parts(
    document: &XlsxDocument,
    part: &PartPath,
    reader: &mut PackageReader<'_>,
) -> Result<Vec<(PartPath, RelatedKind)>, XlsxWriteError> {
    let relationship_part = part.relationship_part().map_err(|error| {
        XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
            .at_source(part.source_id())
            .with_cause(error)
    })?;
    if !reader.contains(&relationship_part) {
        return Ok(Vec::new());
    }
    let bytes = reader.read_part(&relationship_part)?;
    let relationships = parse_relationships(
        &bytes,
        &relationship_part,
        Some(part),
        document.read_options().limits(),
    )
    .map_err(|error| {
        XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
            .at_source(relationship_part.source_id())
            .with_cause(error)
    })?;
    let mut related = Vec::new();
    for relationship in relationships {
        let RelationshipTarget::Internal(target) = relationship.target else {
            continue;
        };
        let kind = relationship
            .kind
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let related_kind = match kind.as_str() {
            "drawing" => RelatedKind::Drawing,
            "vmlDrawing" => RelatedKind::LegacyDrawing,
            "comments" => RelatedKind::Comments,
            "threadedComment" => RelatedKind::ThreadedComments,
            "pivotTable" => RelatedKind::PivotTable,
            _ if KEPT_RELATIONSHIPS.contains(&kind.as_str()) => continue,
            _ => {
                return Err(unrewritable(
                    part,
                    DETAIL_UNREWRITABLE_PART,
                    target.as_str(),
                ));
            }
        };
        if reader.contains(&target) {
            related.push((target, related_kind));
        }
    }
    Ok(related)
}

fn patch_related_part(
    bytes: &[u8],
    part: &PartPath,
    kind: RelatedKind,
    edit: GridEdit,
    limits: WriteLimits,
) -> Result<Option<Vec<u8>>, XlsxWriteError> {
    let mut document = match XmlDocument::parse(bytes, part, limits) {
        Ok(document) => document,
        // Legacy drawings are VML, which producers do not always write as well-formed XML.
        Err(_) if kind == RelatedKind::LegacyDrawing => {
            return Err(unrewritable(part, DETAIL_UNREWRITABLE_PART, part.as_str()));
        }
        Err(error) => return Err(error),
    };
    let root = document.root_mut();
    match kind {
        RelatedKind::Drawing => {
            move_anchors(root, edit, part)?;
        }
        RelatedKind::LegacyDrawing => move_notes(root, edit, part)?,
        RelatedKind::Comments => {
            if let Some(list) = root.child_mut("commentList") {
                move_cell_references(list, "comment", edit, part)?;
            }
        }
        RelatedKind::ThreadedComments => {
            move_cell_references(root, "threadedComment", edit, part)?;
        }
        RelatedKind::PivotTable => {
            let location = root
                .child("location")
                .map(|location| location.attribute("ref", part))
                .transpose()?
                .flatten()
                .and_then(|reference| parse_range(&reference));
            let unchanged = location.is_some_and(|range| {
                !edit.overwrites(range)
                    && edit.map_range(range, RangeRule::Reference) == Some(range)
            });
            if !unchanged {
                return Err(unrewritable(part, DETAIL_MOVED_PIVOT_TABLE, part.as_str()));
            }
            return Ok(None);
        }
    }
    document.write(part, limits).map(Some)
}

/// Moves the cell of each `element` child of `list`, dropping the children whose cell the edit
/// deletes or overwrites.
fn move_cell_references(
    list: &mut XmlElement,
    element: &str,
    edit: GridEdit,
    part: &PartPath,
) -> Result<(), XlsxWriteError> {
    let mut kept = Vec::new();
    for node in std::mem::take(&mut list.children) {
        let XmlNode::Element(mut child) = node else {
            kept.push(node);
            continue;
        };
        if child.is(element)
            && let Some(reference) = child.attribute("ref", part)?
        {
            let Some(address) = parse_range(&reference).map(|range| range.start()) else {
                kept.push(XmlNode::Element(child));
                continue;
            };
            let Some(moved) = edit.map_address(address) else {
                continue;
            };
            child.set_attribute("ref", Some(&moved.to_string()), part)?;
        }
        kept.push(XmlNode::Element(child));
    }
    list.children = kept;
    Ok(())
}

/// Moves the cell anchors of a drawing. An object that moves and sizes with its cells keeps each
/// corner on its cell; one that only moves keeps its size; an absolutely placed one stays.
fn move_anchors(
    element: &mut XmlElement,
    edit: GridEdit,
    part: &PartPath,
) -> Result<(), XlsxWriteError> {
    for child in element.elements_mut() {
        if child.is("twoCellAnchor") {
            match child.attribute("editAs", part)?.as_deref() {
                Some("absolute") => {}
                Some("oneCell") => translate_anchor(child, edit, part)?,
                _ => {
                    let corners = [corner(child, "from", part)?, corner(child, "to", part)?];
                    if let [Some(from), Some(to)] = corners {
                        let moved = match edit {
                            GridEdit::MoveRange { .. } => moved_with_range(edit, from, to),
                            _ => Some(moved_corners(edit, from, to)),
                        };
                        if let Some((from, to)) = moved {
                            set_corner(child, "from", from)?;
                            set_corner(child, "to", to)?;
                        }
                    }
                }
            }
        } else if child.is("oneCellAnchor") {
            if let Some(from) = corner(child, "from", part)? {
                let moved = match edit {
                    GridEdit::MoveRange { .. } => {
                        moved_with_range(edit, from, from).map(|(from, _)| from)
                    }
                    _ => Some(moved_corner(edit, from)),
                };
                if let Some(from) = moved {
                    set_corner(child, "from", from)?;
                }
            }
        } else if child.is("AlternateContent") || child.is("Choice") || child.is("Fallback") {
            move_anchors(child, edit, part)?;
        }
    }
    Ok(())
}

/// Moves both corners of a move-but-don't-size anchor by the distance its first corner moves.
fn translate_anchor(
    anchor: &mut XmlElement,
    edit: GridEdit,
    part: &PartPath,
) -> Result<(), XlsxWriteError> {
    let (Some(from), Some(to)) = (corner(anchor, "from", part)?, corner(anchor, "to", part)?)
    else {
        return Ok(());
    };
    let moved = match edit {
        GridEdit::MoveRange { .. } => moved_with_range(edit, from, to),
        _ => {
            let target = moved_corner(edit, from);
            let shift = |value: u32, before: u32, after: u32| {
                u32::try_from(i64::from(value) + i64::from(after) - i64::from(before)).unwrap_or(0)
            };
            Some((
                target,
                Corner {
                    column: shift(to.column, from.column, target.column),
                    row: shift(to.row, from.row, target.row),
                    ..to
                },
            ))
        }
    };
    if let Some((from, to)) = moved {
        set_corner(anchor, "from", from)?;
        set_corner(anchor, "to", to)?;
    }
    Ok(())
}

/// One corner of a drawing anchor: a zero-based cell and the offset into it.
#[derive(Debug, Clone, Copy)]
struct Corner {
    column: u32,
    column_offset: Option<i64>,
    row: u32,
    row_offset: Option<i64>,
}

fn corner(
    anchor: &XmlElement,
    name: &str,
    part: &PartPath,
) -> Result<Option<Corner>, XlsxWriteError> {
    let Some(point) = anchor.child(name) else {
        return Ok(None);
    };
    let number = |child: &str| -> Result<Option<i64>, XlsxWriteError> {
        Ok(point
            .child(child)
            .map(|element| element.text(part))
            .transpose()?
            .flatten()
            .and_then(|text| text.trim().parse::<i64>().ok()))
    };
    let (Some(column), Some(row)) = (number("col")?, number("row")?) else {
        return Ok(None);
    };
    let (Ok(column), Ok(row)) = (u32::try_from(column), u32::try_from(row)) else {
        return Ok(None);
    };
    // Anchor cells are zero-based; one past the grid cannot follow a line edit.
    if column >= GridAxis::Columns.max() || row >= GridAxis::Rows.max() {
        return Err(unrewritable(part, DETAIL_ANCHOR_OFF_GRID, part.as_str()));
    }
    Ok(Some(Corner {
        column,
        column_offset: number("colOff")?,
        row,
        row_offset: number("rowOff")?,
    }))
}

fn set_corner(anchor: &mut XmlElement, name: &str, corner: Corner) -> Result<(), XlsxWriteError> {
    let Some(point) = anchor.child_mut(name) else {
        return Ok(());
    };
    for (child, value) in [
        ("col", Some(i64::from(corner.column))),
        ("colOff", corner.column_offset),
        ("row", Some(i64::from(corner.row))),
        ("rowOff", corner.row_offset),
    ] {
        if let (Some(element), Some(value)) = (point.child_mut(child), value) {
            element.set_text(&value.to_string())?;
        }
    }
    Ok(())
}

/// Moves both corners of a resizing anchor by a line edit. Each corner follows its line, except
/// that a move that would put the first corner after the second shrinks the anchor as it shrinks
/// a range whose ends cross.
fn moved_corners(edit: GridEdit, from: Corner, to: Corner) -> (Corner, Corner) {
    let (mut moved_from, mut moved_to) = (moved_corner(edit, from), moved_corner(edit, to));
    let (Some(axis), GridEdit::Move { .. }) = (edit.line_axis(), edit) else {
        return (moved_from, moved_to);
    };
    let lines = |corner: Corner| match axis {
        GridAxis::Rows => (corner.row, corner.row_offset),
        GridAxis::Columns => (corner.column, corner.column_offset),
    };
    let ((from_line, from_offset), (to_line, to_offset)) = (lines(moved_from), lines(moved_to));
    if (from_line, from_offset.unwrap_or(0)) <= (to_line, to_offset.unwrap_or(0)) {
        return (moved_from, moved_to);
    }
    let Some((start, end)) =
        edit.map_interval(lines(from).0 + 1, lines(to).0 + 1, RangeRule::Reference)
    else {
        return (moved_from, moved_to);
    };
    let (start, end) = (start - 1, end - 1);
    // Ends that meet on one line keep the anchor's first corner on it.
    let to_offset = if start == end {
        from_offset.max(to_offset)
    } else {
        to_offset
    };
    match axis {
        GridAxis::Rows => {
            moved_from.row = start;
            moved_to.row = end;
            moved_to.row_offset = to_offset;
        }
        GridAxis::Columns => {
            moved_from.column = start;
            moved_to.column = end;
            moved_to.column_offset = to_offset;
        }
    }
    (moved_from, moved_to)
}

/// Moves one corner by a line edit: it follows its line, and a corner on a deleted line moves to
/// the start of the line after the deletion.
fn moved_corner(edit: GridEdit, corner: Corner) -> Corner {
    let Some(axis) = edit.line_axis() else {
        return corner;
    };
    let (line, offset) = match axis {
        GridAxis::Rows => (corner.row, corner.row_offset),
        GridAxis::Columns => (corner.column, corner.column_offset),
    };
    let (line, offset) = match edit.map_line(line + 1) {
        Some(moved) => (moved - 1, offset),
        None => match edit {
            GridEdit::Delete { first, .. } => (first - 1, offset.map(|_| 0)),
            _ => (axis.max() - 1, offset),
        },
    };
    match axis {
        GridAxis::Rows => Corner {
            row: line,
            row_offset: offset,
            ..corner
        },
        GridAxis::Columns => Corner {
            column: line,
            column_offset: offset,
            ..corner
        },
    }
}

/// Moves an anchor whose cells all lie inside the cells a range move cuts; `None` leaves it.
fn moved_with_range(edit: GridEdit, from: Corner, to: Corner) -> Option<(Corner, Corner)> {
    let GridEdit::MoveRange { source, .. } = edit else {
        return None;
    };
    let cell = |corner: Corner| CellAddress::from_indices(corner.row + 1, corner.column + 1).ok();
    let (from_cell, to_cell) = (cell(from)?, cell(to)?);
    if !(source.contains(from_cell) && source.contains(to_cell)) {
        return None;
    }
    let (rows, columns) = edit.range_offset();
    let shift = |corner: Corner| -> Option<Corner> {
        Some(Corner {
            row: u32::try_from(i64::from(corner.row) + rows).ok()?,
            column: u32::try_from(i64::from(corner.column) + columns).ok()?,
            ..corner
        })
    };
    Some((shift(from)?, shift(to)?))
}

/// Moves the notes of a legacy drawing: each note follows its cell, and one whose cell is
/// deleted goes. Form controls are refused.
fn move_notes(
    root: &mut XmlElement,
    edit: GridEdit,
    part: &PartPath,
) -> Result<(), XlsxWriteError> {
    let mut kept = Vec::new();
    for node in std::mem::take(&mut root.children) {
        let XmlNode::Element(mut shape) = node else {
            kept.push(node);
            continue;
        };
        let Some(data) = shape.child("ClientData") else {
            kept.push(XmlNode::Element(shape));
            continue;
        };
        if data.attribute("ObjectType", part)?.as_deref() != Some("Note") {
            return Err(unrewritable(part, DETAIL_FORM_CONTROL, part.as_str()));
        }
        let number = |name: &str| -> Result<Option<u32>, XlsxWriteError> {
            Ok(data
                .child(name)
                .map(|element| element.text(part))
                .transpose()?
                .flatten()
                .and_then(|text| text.trim().parse::<u32>().ok()))
        };
        let (Some(row), Some(column)) = (number("Row")?, number("Column")?) else {
            kept.push(XmlNode::Element(shape));
            continue;
        };
        let Some(address) = CellAddress::from_indices(row + 1, column + 1)
            .ok()
            .and_then(|address| edit.map_address(address))
        else {
            continue;
        };
        let anchor = data
            .child("Anchor")
            .map(|element| element.text(part))
            .transpose()?
            .flatten();
        let data = shape
            .child_mut("ClientData")
            .expect("the client data was found above");
        if let Some(element) = data.child_mut("Row") {
            element.set_text(&(address.row().get() - 1).to_string())?;
        }
        if let Some(element) = data.child_mut("Column") {
            element.set_text(&(address.column().get() - 1).to_string())?;
        }
        let rows = i64::from(address.row().get()) - i64::from(row + 1);
        let columns = i64::from(address.column().get()) - i64::from(column + 1);
        if let Some(anchor) = anchor
            && let Some(moved) = moved_note_anchor(&anchor, rows, columns)
            && let Some(element) = data.child_mut("Anchor")
        {
            element.set_text(&moved)?;
        }
        kept.push(XmlNode::Element(shape));
    }
    root.children = kept;
    Ok(())
}

/// Moves a note's box, `left column, offset, top row, offset, right column, offset, bottom row,
/// offset`, as far as its cell moved, keeping its size.
fn moved_note_anchor(anchor: &str, rows: i64, columns: i64) -> Option<String> {
    let values = anchor
        .split(',')
        .map(|value| value.trim().parse::<i64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let [
        left,
        left_offset,
        top,
        top_offset,
        right,
        right_offset,
        bottom,
        bottom_offset,
    ] = values.as_slice()
    else {
        return None;
    };
    let line = |value: i64, delta: i64| (value + delta).max(0);
    Some(format!(
        "{}, {left_offset}, {}, {top_offset}, {}, {right_offset}, {}, {bottom_offset}",
        line(*left, columns),
        line(*top, rows),
        line(*right, columns),
        line(*bottom, rows),
    ))
}
