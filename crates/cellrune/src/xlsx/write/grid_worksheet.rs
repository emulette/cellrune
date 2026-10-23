//! Replays one structural grid edit on a preserved worksheet part.
//!
//! Rows and cells move to their new coordinates with their XML intact; the cells whose formula
//! the edit rewrote are written again from the draft afterwards, as edited cells. Every other
//! coordinate the worksheet holds moves by the same rules as the workbook model: merged ranges,
//! conditional formats and data validations with their formulas, hyperlinks, the auto-filter and
//! sort state, column spans, page breaks, protected ranges, ignored errors, cell watches,
//! selections, and sparklines. An element whose coordinates this pass does not rewrite is refused.

use std::collections::BTreeMap;

use super::grid_xml::{XmlDocument, XmlElement, XmlNode, invalid_xml};
use super::table_patch::range_text;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteLimits, FormulaRewriteRequest, rewrite_formula,
};
use crate::calculation::grid_rewrite::{AnchoredFormulas, GridChange, move_anchored_formulas};
use crate::grid::{GridAxis, GridEdit, RangeRule};
use crate::xlsx::package::PartPath;
use crate::{CellAddress, CellRange, Column, EXCEL_MAX_COLUMNS, Row};

const DETAIL_UNREWRITABLE_ELEMENT: &str =
    "a structural edit cannot rewrite the coordinates of a preserved worksheet element";
const DETAIL_UNREWRITABLE_EXTENSION: &str =
    "a structural edit cannot rewrite the coordinates of a preserved worksheet extension";
const DETAIL_UNREWRITABLE_FORMULA: &str =
    "a preserved formula that a structural edit changes cannot be rewritten";
const DETAIL_REFERENCE: &str = "worksheet coordinate does not parse";

/// Worksheet children that hold coordinates this pass does not rewrite.
const REFUSED_ELEMENTS: &[&str] = &[
    "scenarios",
    "dataConsolidate",
    "customSheetViews",
    "smartTags",
    "oleObjects",
    "controls",
    "webPublishItems",
    "AlternateContent",
];

/// Worksheet extensions that hold no coordinates.
const KEPT_EXTENSIONS: &[&str] = &["slicerList", "timelineRefs", "webExtensions"];

/// Row attributes inserted rows take from the row above them.
const INHERITED_ROW_ATTRIBUTES: &[&str] = &["s", "customFormat", "ht", "customHeight"];

/// Column attributes inserted columns do not take from the column to their left.
const UNINHERITED_COLUMN_ATTRIBUTES: &[&str] = &["min", "max", "hidden", "collapsed", "phonetic"];

struct Pass<'a> {
    edit: GridEdit,
    change: GridChange<'a>,
    source: &'a PartPath,
}

/// Moves the coordinates of a preserved worksheet through `edit` and sets its `dimension` to
/// `used_range`, the used range of the edited draft sheet.
pub(super) fn patch_worksheet_grid(
    bytes: &[u8],
    source: &PartPath,
    edit: GridEdit,
    sheet_name: &str,
    used_range: Option<CellRange>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let mut document = XmlDocument::parse(bytes, source, limits)?;
    let pass = Pass {
        edit,
        change: GridChange {
            sheet: sheet_name,
            host_is_sheet: true,
            edit,
        },
        source,
    };
    let root = document.root_mut();
    let mut kept = Vec::with_capacity(root.children.len());
    for node in std::mem::take(&mut root.children) {
        let XmlNode::Element(mut element) = node else {
            kept.push(node);
            continue;
        };
        if let Some(refused) = REFUSED_ELEMENTS.iter().find(|name| element.is(name)) {
            return Err(unrewritable(source, DETAIL_UNREWRITABLE_ELEMENT, refused));
        }
        let keep = if element.is("dimension") {
            let reference = used_range.map_or_else(|| "A1".to_owned(), range_text);
            element.set_attribute("ref", Some(&reference), source)?;
            true
        } else if element.is("sheetViews") {
            pass.views(&mut element)?;
            true
        } else if element.is("cols") {
            pass.columns(&mut element)?
        } else if element.is("sheetData") {
            pass.sheet_data(&mut element)?;
            true
        } else if element.is("mergeCells") {
            pass.merged_ranges(&mut element)?
        } else if element.is("conditionalFormatting") {
            pass.conditional_format(&mut element, false)?
        } else if element.is("dataValidations") {
            pass.data_validations(&mut element, false)?
        } else if element.is("hyperlinks") {
            pass.hyperlinks(&mut element)?
        } else if element.is("autoFilter") {
            pass.auto_filter(&mut element)?
        } else if element.is("sortState") {
            pass.sort_state(&mut element)?
        } else if element.is("protectedRanges") || element.is("ignoredErrors") {
            pass.sqref_children(&mut element, false)?
        } else if element.is("cellWatches") {
            pass.cell_watches(&mut element)?
        } else if element.is("rowBreaks") {
            pass.breaks(&mut element, GridAxis::Rows)?
        } else if element.is("colBreaks") {
            pass.breaks(&mut element, GridAxis::Columns)?
        } else if element.is("extLst") {
            pass.extensions(&mut element)?
        } else {
            true
        };
        if keep {
            kept.push(XmlNode::Element(element));
        }
    }
    root.children = kept;
    document.write(source, limits)
}

impl Pass<'_> {
    fn views(&self, views: &mut XmlElement) -> Result<(), XlsxWriteError> {
        for view in views.elements_mut() {
            for child in view.elements_mut() {
                if child.is("pane") {
                    if let Some(cell) = child.attribute("topLeftCell", self.source)?
                        && let Some(moved) = self.moved_cell(&cell)?
                    {
                        child.set_attribute("topLeftCell", Some(&moved), self.source)?;
                    }
                } else if child.is("selection") {
                    let active = match child.attribute("activeCell", self.source)? {
                        Some(cell) => self.moved_cell(&cell)?.or(Some(cell)),
                        None => None,
                    };
                    if let Some(active) = &active {
                        child.set_attribute("activeCell", Some(active), self.source)?;
                    }
                    if let Some(sqref) = child.attribute("sqref", self.source)? {
                        let ranges = self
                            .edit
                            .map_ranges(&self.sqref(&sqref)?, RangeRule::Reference);
                        let text = if ranges.is_empty() {
                            active.unwrap_or(sqref)
                        } else {
                            sqref_text(&ranges)
                        };
                        child.set_attribute("sqref", Some(&text), self.source)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Moves column spans line by line. Columns inserted after another take its width and
    /// style, but not whether it is hidden. Returns whether any span remains.
    fn columns(&self, cols: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        if self.edit.line_axis() != Some(GridAxis::Columns) {
            return Ok(true);
        }
        let template = cols.elements().next().cloned();
        let mut lines = vec![None::<Vec<(String, String)>>; EXCEL_MAX_COLUMNS as usize + 1];
        for col in cols.elements().filter(|element| element.is("col")) {
            let attributes = col.attributes(self.source)?;
            let bound = |key: &str| {
                attributes
                    .iter()
                    .find(|(name, _)| name == key)
                    .and_then(|(_, value)| value.parse::<u32>().ok())
                    .filter(|line| (1..=EXCEL_MAX_COLUMNS).contains(line))
            };
            let (Some(min), Some(max)) = (bound("min"), bound("max")) else {
                return Err(invalid_reference(self.source));
            };
            let kept = attributes
                .into_iter()
                .filter(|(name, _)| name != "min" && name != "max")
                .collect::<Vec<_>>();
            for line in min..=max {
                lines[line as usize] = Some(kept.clone());
            }
        }
        let mut moved = vec![None::<Vec<(String, String)>>; EXCEL_MAX_COLUMNS as usize + 1];
        for (line, attributes) in lines.iter().enumerate().skip(1) {
            if let Some(attributes) = attributes
                && let Some(target) = self.edit.map_line(line as u32)
            {
                moved[target as usize] = Some(attributes.clone());
            }
        }
        if let GridEdit::Insert { at, count, .. } = self.edit
            && at > 1
            && let Some(attributes) = &lines[at as usize - 1]
        {
            let inherited = attributes
                .iter()
                .filter(|(name, _)| !UNINHERITED_COLUMN_ATTRIBUTES.contains(&name.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            for line in at..at + count {
                moved[line as usize] = Some(inherited.clone());
            }
        }
        let Some(template) = template else {
            return Ok(false);
        };
        let mut spans = Vec::<(u32, u32, Vec<(String, String)>)>::new();
        for (line, attributes) in moved.into_iter().enumerate().skip(1) {
            let Some(attributes) = attributes else {
                continue;
            };
            let line = line as u32;
            match spans.last_mut() {
                Some((_, last, previous)) if *last + 1 == line && *previous == attributes => {
                    *last = line;
                }
                _ => spans.push((line, line, attributes)),
            }
        }
        if spans.is_empty() {
            return Ok(false);
        }
        cols.children = spans
            .into_iter()
            .map(|(min, max, attributes)| {
                let mut col = XmlElement::sibling(&template, "col");
                let mut all = vec![
                    ("min".to_owned(), min.to_string()),
                    ("max".to_owned(), max.to_string()),
                ];
                all.extend(attributes);
                col.set_attributes(&all);
                XmlNode::Element(col)
            })
            .collect();
        Ok(true)
    }

    fn sheet_data(&self, sheet_data: &mut XmlElement) -> Result<(), XlsxWriteError> {
        let mut rows = BTreeMap::<u32, XmlElement>::new();
        let mut others = Vec::new();
        for node in std::mem::take(&mut sheet_data.children) {
            match node {
                XmlNode::Element(row) if row.is("row") => {
                    let number = self.row_number(&row)?;
                    rows.insert(number, row);
                }
                XmlNode::Element(_) => others.push(node),
                XmlNode::Other(_) => {}
            }
        }
        let placed = match self.edit {
            GridEdit::Insert {
                axis: GridAxis::Rows,
                ..
            }
            | GridEdit::Delete {
                axis: GridAxis::Rows,
                ..
            }
            | GridEdit::Move {
                axis: GridAxis::Rows,
                ..
            } => self.moved_rows(rows)?,
            GridEdit::Insert {
                axis: GridAxis::Columns,
                ..
            }
            | GridEdit::Delete {
                axis: GridAxis::Columns,
                ..
            }
            | GridEdit::Move {
                axis: GridAxis::Columns,
                ..
            } => self.moved_columns(rows)?,
            GridEdit::MoveRange { .. } => self.moved_cells(rows, sheet_data)?,
        };
        sheet_data.children = placed.into_values().map(XmlNode::Element).collect();
        sheet_data.children.extend(others);
        Ok(())
    }

    fn moved_rows(
        &self,
        rows: BTreeMap<u32, XmlElement>,
    ) -> Result<BTreeMap<u32, XmlElement>, XlsxWriteError> {
        let mut placed = BTreeMap::new();
        if let GridEdit::Insert { at, count, .. } = self.edit
            && at > 1
            && let Some(template) = rows.get(&(at - 1))
        {
            for line in at..at + count {
                if let Some(row) = self.inherited_row(template, line)? {
                    placed.insert(line, row);
                }
            }
        }
        for (number, mut row) in rows {
            let Some(target) = self.edit.map_line(number) else {
                continue;
            };
            row.set_attribute("r", Some(&target.to_string()), self.source)?;
            let target_row = Row::new(target).map_err(|error| invalid_xml(self.source, error))?;
            for cell in row.elements_mut().filter(|element| element.is("c")) {
                let address = cell_address(cell, self.source)?;
                let moved = CellAddress::new(target_row, address.column());
                cell.set_attribute("r", Some(&moved.to_string()), self.source)?;
            }
            placed.insert(target, row);
        }
        Ok(placed)
    }

    /// Returns the row an insertion adds at `line` after `template`: its style and height, and
    /// an empty cell with each style its cells carry. `None` when that row would hold nothing.
    fn inherited_row(
        &self,
        template: &XmlElement,
        line: u32,
    ) -> Result<Option<XmlElement>, XlsxWriteError> {
        let mut attributes = vec![("r".to_owned(), line.to_string())];
        attributes.extend(
            template
                .attributes(self.source)?
                .into_iter()
                .filter(|(name, _)| INHERITED_ROW_ATTRIBUTES.contains(&name.as_str())),
        );
        let row_number = Row::new(line).map_err(|error| invalid_xml(self.source, error))?;
        let mut cells = Vec::new();
        for cell in template.elements().filter(|element| element.is("c")) {
            let Some(style) = styled(cell, self.source)? else {
                continue;
            };
            let address = cell_address(cell, self.source)?;
            cells.push(blank_cell(
                cell,
                CellAddress::new(row_number, address.column()),
                &style,
            ));
        }
        if attributes.len() == 1 && cells.is_empty() {
            return Ok(None);
        }
        let mut row = template.clone();
        row.set_attributes(&attributes);
        row.children = cells.into_iter().map(XmlNode::Element).collect();
        Ok(Some(row))
    }

    fn moved_columns(
        &self,
        rows: BTreeMap<u32, XmlElement>,
    ) -> Result<BTreeMap<u32, XmlElement>, XlsxWriteError> {
        let mut placed = BTreeMap::new();
        for (number, mut row) in rows {
            let mut cells = BTreeMap::<u32, XmlElement>::new();
            let mut others = Vec::new();
            let mut template = None;
            for node in std::mem::take(&mut row.children) {
                let XmlNode::Element(mut cell) = node else {
                    continue;
                };
                if !cell.is("c") {
                    others.push(XmlNode::Element(cell));
                    continue;
                }
                let address = cell_address(&cell, self.source)?;
                if let GridEdit::Insert { at, .. } = self.edit
                    && address.column().get() + 1 == at
                    && let Some(style) = styled(&cell, self.source)?
                {
                    template = Some((cell.clone(), style));
                }
                let Some(target) = self.edit.map_line(address.column().get()) else {
                    continue;
                };
                let moved = CellAddress::new(
                    address.row(),
                    Column::new(target).map_err(|error| invalid_xml(self.source, error))?,
                );
                cell.set_attribute("r", Some(&moved.to_string()), self.source)?;
                cells.insert(target, cell);
            }
            if let (GridEdit::Insert { at, count, .. }, Some((template, style))) =
                (self.edit, template)
            {
                let row_number =
                    Row::new(number).map_err(|error| invalid_xml(self.source, error))?;
                for line in at..at + count {
                    let column =
                        Column::new(line).map_err(|error| invalid_xml(self.source, error))?;
                    cells.insert(
                        line,
                        blank_cell(&template, CellAddress::new(row_number, column), &style),
                    );
                }
            }
            // `spans` only hints at the columns a row uses, so it goes rather than mislead.
            row.set_attribute("spans", None, self.source)?;
            row.children = cells.into_values().map(XmlNode::Element).collect();
            row.children.extend(others);
            placed.insert(number, row);
        }
        Ok(placed)
    }

    /// Moves the cells of a range move: cells of the source land in the destination, replacing
    /// the cells there, and each row keeps its own attributes.
    fn moved_cells(
        &self,
        rows: BTreeMap<u32, XmlElement>,
        sheet_data: &XmlElement,
    ) -> Result<BTreeMap<u32, XmlElement>, XlsxWriteError> {
        let mut placed = BTreeMap::<u32, XmlElement>::new();
        let mut cells = BTreeMap::<CellAddress, XmlElement>::new();
        let mut row_others = BTreeMap::<u32, Vec<XmlNode>>::new();
        for (number, mut row) in rows {
            for node in std::mem::take(&mut row.children) {
                let XmlNode::Element(mut cell) = node else {
                    continue;
                };
                if !cell.is("c") {
                    row_others
                        .entry(number)
                        .or_default()
                        .push(XmlNode::Element(cell));
                    continue;
                }
                let Some(moved) = self.edit.map_address(cell_address(&cell, self.source)?) else {
                    continue;
                };
                cell.set_attribute("r", Some(&moved.to_string()), self.source)?;
                cells.insert(moved, cell);
            }
            row.set_attribute("spans", None, self.source)?;
            placed.insert(number, row);
        }
        let template =
            XmlElement::sibling(sheet_data.elements().next().unwrap_or(sheet_data), "row");
        for (address, cell) in cells {
            let number = address.row().get();
            let row = placed.entry(number).or_insert_with(|| {
                let mut row = template.clone();
                row.set_attributes(&[("r".to_owned(), number.to_string())]);
                row
            });
            row.children.push(XmlNode::Element(cell));
        }
        for (number, others) in row_others {
            if let Some(row) = placed.get_mut(&number) {
                row.children.extend(others);
            }
        }
        Ok(placed)
    }

    fn merged_ranges(&self, merges: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let mut kept = Vec::new();
        for merge in merges.elements().filter(|element| element.is("mergeCell")) {
            let Some(reference) = merge.attribute("ref", self.source)? else {
                continue;
            };
            let range = self.range(&reference)?;
            if self.edit.overwrites(range) {
                continue;
            }
            let Some(moved) = self.edit.map_range(range, RangeRule::Reference) else {
                continue;
            };
            if moved.height() == 1 && moved.width() == 1 {
                continue;
            }
            let mut merge = merge.clone();
            merge.set_attribute("ref", Some(&range_text(moved)), self.source)?;
            kept.push(XmlNode::Element(merge));
        }
        if kept.is_empty() {
            return Ok(false);
        }
        merges.set_attribute("count", Some(&kept.len().to_string()), self.source)?;
        merges.children = kept;
        Ok(true)
    }

    /// Moves a conditional format: its ranges, then every formula of its rules relative to its
    /// first cell. Returns `false` when every cell it formats is deleted.
    fn conditional_format(
        &self,
        format: &mut XmlElement,
        extension: bool,
    ) -> Result<bool, XlsxWriteError> {
        let ranges = self.anchored_ranges(format, extension)?;
        let mut formulas = Vec::new();
        let mut values = Vec::new();
        if extension {
            for formula in format.descendants_mut("f") {
                formulas.push(self.formula_text(formula)?);
            }
        } else {
            for formula in format.descendants_mut("formula") {
                formulas.push(self.formula_text(formula)?);
            }
            for threshold in format.descendants_mut("cfvo") {
                if let Some(value) = threshold.attribute("val", self.source)? {
                    values.push(value);
                }
            }
        }
        let texts = formulas
            .iter()
            .chain(&values)
            .map(String::as_str)
            .collect::<Vec<_>>();
        let Some(moved) = self.move_anchored(&ranges, &texts)? else {
            return Ok(false);
        };
        self.set_anchored_ranges(format, extension, &moved.ranges)?;
        let mut rewritten = moved.formulas.into_iter();
        let name = if extension { "f" } else { "formula" };
        for formula in format.descendants_mut(name) {
            if let Some(Some(text)) = rewritten.next() {
                formula.set_text(&text)?;
            }
        }
        if !extension {
            for threshold in format.descendants_mut("cfvo") {
                if threshold.attribute("val", self.source)?.is_some()
                    && let Some(Some(text)) = rewritten.next()
                {
                    threshold.set_attribute("val", Some(&text), self.source)?;
                }
            }
        }
        Ok(true)
    }

    /// Moves every data validation of a `dataValidations` element and drops the ones whose cells
    /// are all deleted. Returns `false` when none remains.
    fn data_validations(
        &self,
        validations: &mut XmlElement,
        extension: bool,
    ) -> Result<bool, XlsxWriteError> {
        let mut kept = Vec::new();
        for node in std::mem::take(&mut validations.children) {
            let XmlNode::Element(mut validation) = node else {
                kept.push(node);
                continue;
            };
            if !validation.is("dataValidation") {
                kept.push(XmlNode::Element(validation));
                continue;
            }
            let ranges = self.anchored_ranges(&validation, extension)?;
            let mut formulas = Vec::new();
            for slot in ["formula1", "formula2"] {
                if let Some(holder) = validation.child_mut(slot) {
                    let formula = if extension {
                        holder
                            .child_mut("f")
                            .ok_or_else(|| self.unrewritable_formula())?
                    } else {
                        holder
                    };
                    formulas.push(self.formula_text(formula)?);
                }
            }
            let texts = formulas.iter().map(String::as_str).collect::<Vec<_>>();
            let Some(moved) = self.move_anchored(&ranges, &texts)? else {
                continue;
            };
            self.set_anchored_ranges(&mut validation, extension, &moved.ranges)?;
            let mut rewritten = moved.formulas.into_iter();
            for slot in ["formula1", "formula2"] {
                if let Some(holder) = validation.child_mut(slot) {
                    let formula = if extension {
                        holder
                            .child_mut("f")
                            .ok_or_else(|| self.unrewritable_formula())?
                    } else {
                        holder
                    };
                    if let Some(Some(text)) = rewritten.next() {
                        formula.set_text(&text)?;
                    }
                }
            }
            kept.push(XmlNode::Element(validation));
        }
        let count = kept
            .iter()
            .filter(
                |node| matches!(node, XmlNode::Element(element) if element.is("dataValidation")),
            )
            .count();
        validations.children = kept;
        if count == 0 {
            return Ok(false);
        }
        if validations.attribute("count", self.source)?.is_some() {
            validations.set_attribute("count", Some(&count.to_string()), self.source)?;
        }
        Ok(true)
    }

    fn hyperlinks(&self, hyperlinks: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let mut kept = Vec::new();
        for hyperlink in hyperlinks
            .elements()
            .filter(|element| element.is("hyperlink"))
        {
            let Some(reference) = hyperlink.attribute("ref", self.source)? else {
                continue;
            };
            let range = self.range(&reference)?;
            if self.edit.overwrites(range) {
                continue;
            }
            let Some(moved) = self.edit.map_range(range, RangeRule::Reference) else {
                continue;
            };
            let mut hyperlink = hyperlink.clone();
            hyperlink.set_attribute("ref", Some(&range_text(moved)), self.source)?;
            kept.push(XmlNode::Element(hyperlink));
        }
        if kept.is_empty() {
            return Ok(false);
        }
        hyperlinks.children = kept;
        Ok(true)
    }

    /// Moves the sheet auto-filter as the workbook model moves it; `false` once its range is
    /// deleted.
    fn auto_filter(&self, filter: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let Some(reference) = filter.attribute("ref", self.source)? else {
            return Ok(true);
        };
        let range = self.range(&reference)?;
        let Some(moved) = self.edit.map_range(range, RangeRule::Reference) else {
            return Ok(false);
        };
        filter.set_attribute("ref", Some(&range_text(moved)), self.source)?;
        if self.edit.line_axis() == Some(GridAxis::Columns) {
            let first = range.start().column().get();
            let new_first = moved.start().column().get();
            let mut kept = Vec::new();
            for node in std::mem::take(&mut filter.children) {
                let XmlNode::Element(mut column) = node else {
                    kept.push(node);
                    continue;
                };
                if column.is("filterColumn") {
                    let Some(identifier) = column
                        .attribute("colId", self.source)?
                        .and_then(|value| value.parse::<u32>().ok())
                    else {
                        return Err(invalid_reference(self.source));
                    };
                    let Some(offset) = self
                        .edit
                        .map_line(first + identifier)
                        .and_then(|line| line.checked_sub(new_first))
                        .filter(|offset| *offset < moved.width())
                    else {
                        continue;
                    };
                    column.set_attribute("colId", Some(&offset.to_string()), self.source)?;
                }
                kept.push(XmlNode::Element(column));
            }
            filter.children = kept;
        }
        if let Some(sort) = filter.child_mut("sortState")
            && !self.sort_state(sort)?
        {
            filter.retain_elements(|element| !element.is("sortState"));
        }
        Ok(true)
    }

    fn sort_state(&self, sort: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let Some(reference) = sort.attribute("ref", self.source)? else {
            return Ok(true);
        };
        let Some(moved) = self
            .edit
            .map_range(self.range(&reference)?, RangeRule::Reference)
        else {
            return Ok(false);
        };
        sort.set_attribute("ref", Some(&range_text(moved)), self.source)?;
        let mut kept = Vec::new();
        for node in std::mem::take(&mut sort.children) {
            let XmlNode::Element(mut condition) = node else {
                kept.push(node);
                continue;
            };
            if condition.is("sortCondition")
                && let Some(reference) = condition.attribute("ref", self.source)?
            {
                let Some(moved) = self
                    .edit
                    .map_range(self.range(&reference)?, RangeRule::Reference)
                else {
                    continue;
                };
                condition.set_attribute("ref", Some(&range_text(moved)), self.source)?;
            }
            kept.push(XmlNode::Element(condition));
        }
        sort.children = kept;
        Ok(true)
    }

    /// Moves the `sqref` of each child of a protected-range or ignored-error list, dropping the
    /// children whose cells are all deleted. Returns `false` when none remains.
    fn sqref_children(
        &self,
        list: &mut XmlElement,
        extension: bool,
    ) -> Result<bool, XlsxWriteError> {
        let mut kept = Vec::new();
        for node in std::mem::take(&mut list.children) {
            let XmlNode::Element(mut child) = node else {
                kept.push(node);
                continue;
            };
            let ranges = self.anchored_ranges(&child, extension)?;
            let moved = self.edit.map_ranges(&ranges, RangeRule::Reference);
            if moved.is_empty() {
                continue;
            }
            self.set_anchored_ranges(&mut child, extension, &moved)?;
            kept.push(XmlNode::Element(child));
        }
        let any = kept.iter().any(|node| matches!(node, XmlNode::Element(_)));
        list.children = kept;
        Ok(any)
    }

    fn cell_watches(&self, watches: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let mut kept = Vec::new();
        for watch in watches.elements() {
            let Some(cell) = watch.attribute("r", self.source)? else {
                continue;
            };
            let Some(moved) = self.moved_cell(&cell)? else {
                continue;
            };
            let mut watch = watch.clone();
            watch.set_attribute("r", Some(&moved), self.source)?;
            kept.push(XmlNode::Element(watch));
        }
        if kept.is_empty() {
            return Ok(false);
        }
        watches.children = kept;
        Ok(true)
    }

    /// Moves the page breaks along `axis`. A break sits before the zero-based line its `id`
    /// names, so it follows that line; a break on a deleted line goes.
    fn breaks(&self, breaks: &mut XmlElement, axis: GridAxis) -> Result<bool, XlsxWriteError> {
        if self.edit.line_axis() != Some(axis) {
            return Ok(true);
        }
        let mut kept = Vec::new();
        let mut manual = 0_usize;
        for brk in breaks.elements() {
            let Some(identifier) = brk
                .attribute("id", self.source)?
                .and_then(|value| value.parse::<u32>().ok())
            else {
                continue;
            };
            let Some(line) = self.edit.map_line(identifier + 1) else {
                continue;
            };
            let mut brk = brk.clone();
            brk.set_attribute("id", Some(&(line - 1).to_string()), self.source)?;
            if brk.attribute("man", self.source)?.as_deref() == Some("1") {
                manual += 1;
            }
            kept.push(XmlNode::Element(brk));
        }
        if kept.is_empty() {
            return Ok(false);
        }
        breaks.set_attribute("count", Some(&kept.len().to_string()), self.source)?;
        if breaks.attribute("manualBreakCount", self.source)?.is_some() {
            breaks.set_attribute("manualBreakCount", Some(&manual.to_string()), self.source)?;
        }
        breaks.children = kept;
        Ok(true)
    }

    /// Moves the extensions Excel writes with coordinates and refuses unknown ones. Returns
    /// `false` when no extension remains.
    fn extensions(&self, list: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let mut kept = Vec::new();
        for node in std::mem::take(&mut list.children) {
            let XmlNode::Element(mut extension) = node else {
                kept.push(node);
                continue;
            };
            let uri = extension.attribute("uri", self.source)?.unwrap_or_default();
            let keep = match extension.elements_mut().next() {
                None => true,
                Some(content) => {
                    if content.is("conditionalFormattings") {
                        let mut formats = Vec::new();
                        for node in std::mem::take(&mut content.children) {
                            match node {
                                XmlNode::Element(mut format) => {
                                    if self.conditional_format(&mut format, true)? {
                                        formats.push(XmlNode::Element(format));
                                    }
                                }
                                other => formats.push(other),
                            }
                        }
                        let any = formats
                            .iter()
                            .any(|node| matches!(node, XmlNode::Element(_)));
                        content.children = formats;
                        any
                    } else if content.is("dataValidations") {
                        self.data_validations(content, true)?
                    } else if content.is("sparklineGroups") {
                        self.sparkline_groups(content)?
                    } else if content.is("protectedRanges") || content.is("ignoredErrors") {
                        self.sqref_children(content, true)?
                    } else if KEPT_EXTENSIONS.iter().any(|name| content.is(name)) {
                        true
                    } else {
                        return Err(unrewritable(
                            self.source,
                            DETAIL_UNREWRITABLE_EXTENSION,
                            &uri,
                        ));
                    }
                }
            };
            if keep {
                kept.push(XmlNode::Element(extension));
            }
        }
        let any = kept.iter().any(|node| matches!(node, XmlNode::Element(_)));
        list.children = kept;
        Ok(any)
    }

    /// Moves each sparkline's location and rewrites its data range; a sparkline whose cell is
    /// deleted goes, and so does a group left without sparklines.
    fn sparkline_groups(&self, groups: &mut XmlElement) -> Result<bool, XlsxWriteError> {
        let request = FormulaRewriteRequest::Grid(self.change);
        let mut kept_groups = Vec::new();
        for node in std::mem::take(&mut groups.children) {
            let XmlNode::Element(mut group) = node else {
                kept_groups.push(node);
                continue;
            };
            if let Some(axis) = group.child_mut("f") {
                self.rewrite_element(axis, &request)?;
            }
            let mut any = false;
            if let Some(sparklines) = group.child_mut("sparklines") {
                let mut kept = Vec::new();
                for node in std::mem::take(&mut sparklines.children) {
                    let XmlNode::Element(mut sparkline) = node else {
                        kept.push(node);
                        continue;
                    };
                    let ranges = self.anchored_ranges(&sparkline, true)?;
                    let moved = self.edit.map_ranges(&ranges, RangeRule::Reference);
                    if moved.is_empty() {
                        continue;
                    }
                    self.set_anchored_ranges(&mut sparkline, true, &moved)?;
                    if let Some(data) = sparkline.child_mut("f") {
                        self.rewrite_element(data, &request)?;
                    }
                    kept.push(XmlNode::Element(sparkline));
                }
                any = kept.iter().any(|node| matches!(node, XmlNode::Element(_)));
                sparklines.children = kept;
            }
            if any {
                kept_groups.push(XmlNode::Element(group));
            }
        }
        let any = kept_groups
            .iter()
            .any(|node| matches!(node, XmlNode::Element(_)));
        groups.children = kept_groups;
        Ok(any)
    }

    fn rewrite_element(
        &self,
        element: &mut XmlElement,
        request: &FormulaRewriteRequest<'_>,
    ) -> Result<(), XlsxWriteError> {
        let text = self.formula_text(element)?;
        let never_cancelled = || false;
        let mut budget =
            FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &never_cancelled);
        if let Some(rewritten) =
            rewrite_formula(&text, request, &mut budget).map_err(|_| self.unrewritable_formula())?
        {
            element.set_text(&rewritten)?;
        }
        Ok(())
    }

    /// Reads the ranges of a conditional format, data validation, or protected range: the
    /// `sqref` attribute, or in an extension the `xm:sqref` child.
    fn anchored_ranges(
        &self,
        element: &XmlElement,
        extension: bool,
    ) -> Result<Vec<CellRange>, XlsxWriteError> {
        let text = if extension {
            element
                .child("sqref")
                .map(|sqref| sqref.text(self.source))
                .transpose()?
                .flatten()
        } else {
            element.attribute("sqref", self.source)?
        };
        text.map_or_else(|| Ok(Vec::new()), |text| self.sqref(&text))
    }

    fn set_anchored_ranges(
        &self,
        element: &mut XmlElement,
        extension: bool,
        ranges: &[CellRange],
    ) -> Result<(), XlsxWriteError> {
        let text = sqref_text(ranges);
        if extension {
            if let Some(sqref) = element.child_mut("sqref") {
                sqref.set_text(&text)?;
            }
            Ok(())
        } else {
            element.set_attribute("sqref", Some(&text), self.source)
        }
    }

    fn move_anchored(
        &self,
        ranges: &[CellRange],
        formulas: &[&str],
    ) -> Result<Option<AnchoredFormulas>, XlsxWriteError> {
        let never_cancelled = || false;
        let mut budget =
            FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &never_cancelled);
        if ranges.is_empty() {
            return Ok(Some(AnchoredFormulas {
                ranges: Vec::new(),
                formulas: vec![None; formulas.len()],
            }));
        }
        move_anchored_formulas(ranges, formulas, self.change, &mut budget)
            .map_err(|_| self.unrewritable_formula())
    }

    fn formula_text(&self, element: &XmlElement) -> Result<String, XlsxWriteError> {
        element
            .text(self.source)?
            .ok_or_else(|| self.unrewritable_formula())
    }

    fn moved_cell(&self, text: &str) -> Result<Option<String>, XlsxWriteError> {
        let address =
            CellAddress::from_a1(text).map_err(|error| invalid_xml(self.source, error))?;
        Ok(self
            .edit
            .map_address(address)
            .map(|moved| moved.to_string()))
    }

    fn range(&self, text: &str) -> Result<CellRange, XlsxWriteError> {
        parse_range(text).ok_or_else(|| invalid_reference(self.source))
    }

    fn sqref(&self, text: &str) -> Result<Vec<CellRange>, XlsxWriteError> {
        text.split_whitespace()
            .map(|reference| self.range(reference))
            .collect()
    }

    fn row_number(&self, row: &XmlElement) -> Result<u32, XlsxWriteError> {
        row.attribute("r", self.source)?
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or_else(|| invalid_reference(self.source))
    }

    fn unrewritable_formula(&self) -> XlsxWriteError {
        XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
            .with_detail(format!(
                "{DETAIL_UNREWRITABLE_FORMULA}: {}",
                self.source.as_str()
            ))
            .at_source(self.source.source_id())
    }
}

/// Returns the style of a cell that carries one other than the default.
fn styled(cell: &XmlElement, source: &PartPath) -> Result<Option<String>, XlsxWriteError> {
    Ok(cell.attribute("s", source)?.filter(|style| style != "0"))
}

fn blank_cell(template: &XmlElement, address: CellAddress, style: &str) -> XmlElement {
    let mut cell = XmlElement::sibling(template, "c");
    cell.set_attributes(&[
        ("r".to_owned(), address.to_string()),
        ("s".to_owned(), style.to_owned()),
    ]);
    cell
}

fn cell_address(cell: &XmlElement, source: &PartPath) -> Result<CellAddress, XlsxWriteError> {
    let text = cell
        .attribute("r", source)?
        .ok_or_else(|| invalid_reference(source))?;
    CellAddress::from_a1(&text).map_err(|error| invalid_xml(source, error))
}

/// Parses `A1`, `A1:B2`, or a span of whole columns or rows, ignoring `$` markers.
pub(super) fn parse_range(text: &str) -> Option<CellRange> {
    CellRange::from_sqref_item(text)
}

pub(super) fn sqref_text(ranges: &[CellRange]) -> String {
    ranges
        .iter()
        .map(|range| range.sqref_item_text())
        .collect::<Vec<_>>()
        .join(" ")
}

fn invalid_reference(source: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(DETAIL_REFERENCE)
        .at_source(source.source_id())
}

pub(super) fn unrewritable(source: &PartPath, detail: &str, what: &str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(format!("{detail}: {what} in {}", source.as_str()))
        .at_source(source.source_id())
}

#[cfg(test)]
#[path = "grid_worksheet_tests.rs"]
mod tests;
