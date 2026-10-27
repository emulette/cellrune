//! Precedent and dependent queries over the dependency graph of the installed calculation.

use std::collections::BTreeSet;

use super::super::eval::{CompiledWorkbook, DependencyTarget};
use super::super::runtime::{CellId, Rect, RectSpan};
use super::WorkbookCalculationSession;
use crate::{CalculationCellId, CellAddress, CellRange, SheetId, WorkbookSnapshot};

/// How a formula reads one of its precedents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PrecedentKind {
    /// The formula reads the values of the cells.
    Value,
    /// The formula reads the spill range of the dynamic-array formula anchored at the cell, as
    /// `A1#` does.
    Spill,
    /// The formula reads whether the cell holds a formula and its text, as `FORMULATEXT` and
    /// `ISFORMULA` do.
    FormulaText,
    /// A reference end that moves with values, such as the `INDEX` end of a range, can reach
    /// these cells; the formula reads the part its current values select.
    ValueDependent,
}

/// One rectangle a formula reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Precedent {
    sheet_id: SheetId,
    range: CellRange,
    kind: PrecedentKind,
}

impl Precedent {
    /// Returns the sheet the rectangle is on.
    pub const fn sheet_id(&self) -> SheetId {
        self.sheet_id
    }

    /// Returns the rectangle; one cell is a one-cell range.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns how the formula reads the rectangle.
    pub const fn kind(&self) -> PrecedentKind {
        self.kind
    }
}

/// What one cell's formula reads, from the dependency graph of the installed calculation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellPrecedents {
    precedents: Vec<Precedent>,
    dynamic: bool,
}

impl CellPrecedents {
    /// Returns the rectangles the formula reads, in sheet, row, column, and kind order. A cell
    /// without a formula, or whose formula did not parse, reads none.
    pub fn precedents(&self) -> &[Precedent] {
        &self.precedents
    }

    /// Returns whether the formula's references depend on values, as `INDIRECT` and an `OFFSET`
    /// whose position or size is not constant do, so the precedents are those the installed
    /// calculation's values resolved and can change with the values. An `OFFSET` of a cell or
    /// range by constant positions and size reads fixed cells and is not dynamic.
    pub const fn is_dynamic(&self) -> bool {
        self.dynamic
    }
}

/// One formula that reads a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dependent {
    cell: CalculationCellId,
    dynamic: bool,
}

impl Dependent {
    /// Returns the formula cell.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns whether the formula's references depend on values, so it reads the cell under the
    /// installed calculation's values and may not after they change.
    pub const fn is_dynamic(&self) -> bool {
        self.dynamic
    }
}

/// The formulas that read one cell, from the dependency graph of the installed calculation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellDependents {
    dependents: Vec<Dependent>,
    unlisted_dynamic_formulas: usize,
}

impl CellDependents {
    /// Returns the formulas that read the cell directly, in sheet and address order. A
    /// dynamic-array formula whose spill range covers the cell is among them, since a value there
    /// blocks its spill.
    pub fn dependents(&self) -> &[Dependent] {
        &self.dependents
    }

    /// Returns how many formulas with references that depend on values are not listed: they do not
    /// read the cell under the installed calculation's values, but they can after a value
    /// changes, which the graph cannot resolve without calculating.
    pub const fn unlisted_dynamic_formulas(&self) -> usize {
        self.unlisted_dynamic_formulas
    }
}

impl WorkbookCalculationSession {
    /// Returns what the formula at `cell` reads, or `None` when no calculation of the current
    /// revision is installed, since the dependency graph is the one that calculation built.
    pub fn precedents(&self, cell: CalculationCellId) -> Option<CellPrecedents> {
        let compiled = self.current_compiled()?;
        let workbook = self.workbook();
        let Some(internal) = internal_cell(workbook, cell) else {
            return Some(CellPrecedents::default());
        };
        // Keyed by sheet position, rectangle, and kind, so the list follows tab order.
        let mut precedents = BTreeSet::new();
        for target in compiled.dependency_targets(internal).unwrap_or_default() {
            let (spans, kind) = match target {
                DependencyTarget::Cell(cell) => (vec![cell_rect(*cell)], PrecedentKind::Value),
                DependencyTarget::SpillAnchor(cell) => {
                    (vec![cell_rect(*cell)], PrecedentKind::Spill)
                }
                DependencyTarget::FormulaContent(cell) => {
                    (vec![cell_rect(*cell)], PrecedentKind::FormulaText)
                }
                DependencyTarget::Area(span) => (span_rects(span), PrecedentKind::Value),
                DependencyTarget::ImpactArea(span) => {
                    (span_rects(span), PrecedentKind::ValueDependent)
                }
                DependencyTarget::TableIdentity(_) => continue,
            };
            precedents.extend(spans.into_iter().map(|rect| {
                (
                    rect.sheet,
                    rect.row_start,
                    rect.col_start,
                    rect.row_end,
                    rect.col_end,
                    kind,
                )
            }));
        }
        Some(CellPrecedents {
            precedents: precedents
                .into_iter()
                .filter_map(|(sheet, row_start, col_start, row_end, col_end, kind)| {
                    precedent(
                        workbook,
                        Rect {
                            sheet,
                            row_start,
                            col_start,
                            row_end,
                            col_end,
                            whole_rows: false,
                        },
                        kind,
                    )
                })
                .collect(),
            dynamic: compiled.value_dependent_formulas().contains(&internal),
        })
    }

    /// Returns the formulas that read `cell` directly, or `None` when no calculation of the
    /// current revision is installed, since the dependency graph is the one that calculation built.
    pub fn dependents(&self, cell: CalculationCellId) -> Option<CellDependents> {
        let compiled = self.current_compiled()?;
        let workbook = self.workbook();
        let dynamic = compiled.value_dependent_formulas();
        let mut formulas = internal_cell(workbook, cell)
            .map(|internal| {
                compiled
                    .direct_affected_formulas(internal, &mut || Ok(()))
                    .expect("uncharged dependent lookup cannot be cancelled")
            })
            .unwrap_or_default();
        // An array formula's region holds its own anchor, which reads that cell only when one of
        // its references does.
        if let Some(internal) = internal_cell(workbook, cell)
            && formulas.contains(&internal)
            && !self.precedents(cell).is_some_and(|precedents| {
                precedents.precedents().iter().any(|precedent| {
                    precedent.sheet_id() == cell.sheet_id()
                        && precedent.range().contains(cell.address())
                })
            })
        {
            formulas.retain(|formula| *formula != internal);
        }
        let listed_dynamic = formulas
            .iter()
            .filter(|formula| dynamic.contains(formula))
            .count();
        let mut dependents = formulas
            .into_iter()
            .filter_map(|formula| {
                Some(Dependent {
                    cell: public_cell(workbook, formula)?,
                    dynamic: dynamic.contains(&formula),
                })
            })
            .collect::<Vec<_>>();
        dependents.sort_unstable();
        Some(CellDependents {
            dependents,
            unlisted_dynamic_formulas: dynamic.len() - listed_dynamic,
        })
    }

    /// Returns the compiled graph of the installed calculation when it calculated the current
    /// revision.
    fn current_compiled(&self) -> Option<&CompiledWorkbook> {
        let calculation = self.calculation.as_deref()?;
        if calculation.source_revision() != self.workbook().semantic_revision() {
            return None;
        }
        self.compiled.as_deref()
    }
}

fn internal_cell(workbook: &WorkbookSnapshot, cell: CalculationCellId) -> Option<CellId> {
    Some((
        workbook.sheet_position(cell.sheet_id())?,
        cell.address().row().get(),
        cell.address().column().get(),
    ))
}

fn public_cell(workbook: &WorkbookSnapshot, cell: CellId) -> Option<CalculationCellId> {
    let sheet = workbook.sheets().get(cell.0)?;
    Some(CalculationCellId::new(
        sheet.id(),
        CellAddress::from_indices(cell.1, cell.2).ok()?,
    ))
}

const fn cell_rect((sheet, row, column): CellId) -> Rect {
    Rect {
        sheet,
        row_start: row,
        col_start: column,
        row_end: row,
        col_end: column,
        whole_rows: false,
    }
}

fn span_rects(span: &RectSpan) -> Vec<Rect> {
    span.rects().collect()
}

fn precedent(workbook: &WorkbookSnapshot, rect: Rect, kind: PrecedentKind) -> Option<Precedent> {
    let sheet = workbook.sheets().get(rect.sheet)?;
    let range = CellRange::new(
        CellAddress::from_indices(rect.row_start, rect.col_start).ok()?,
        CellAddress::from_indices(rect.row_end, rect.col_end).ok()?,
    )
    .ok()?;
    Some(Precedent {
        sheet_id: sheet.id(),
        range,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::PrecedentKind;
    use crate::{
        CalculationCellId, CalculationOptions, CellAddress, CellValue, EditBatch, FiniteNumber,
        FormulaText, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
    };

    fn cell(sheet: SheetId, address: &str) -> CalculationCellId {
        CalculationCellId::new(sheet, CellAddress::from_a1(address).expect("address"))
    }

    fn formula(sheet: SheetId, address: &str, text: &str) -> WorkbookChange {
        WorkbookChange::set_cell_formula(
            sheet,
            CellAddress::from_a1(address).expect("address"),
            FormulaText::from_user_input(text).expect("formula"),
        )
    }

    fn number(sheet: SheetId, address: &str, value: f64) -> WorkbookChange {
        WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_a1(address).expect("address"),
            CellValue::Number(FiniteNumber::new(value).expect("finite")),
        )
    }

    fn described(
        session: &WorkbookCalculationSession,
        address: &str,
    ) -> (Vec<(String, PrecedentKind)>, bool) {
        let sheet = session.workbook().sheets()[0].id();
        let precedents = session
            .precedents(cell(sheet, address))
            .expect("current calculation");
        (
            precedents
                .precedents()
                .iter()
                .map(|precedent| {
                    let range = precedent.range();
                    let text = if range.start() == range.end() {
                        range.start().to_string()
                    } else {
                        format!("{}:{}", range.start(), range.end())
                    };
                    (text, precedent.kind())
                })
                .collect(),
            precedents.is_dynamic(),
        )
    }

    #[test]
    fn precedents_and_dependents_follow_the_installed_graph() {
        let mut session = WorkbookCalculationSession::create();
        let sheet = session.workbook().sheets()[0].id();
        session
            .apply_changes(
                0,
                EditBatch::new([
                    number(sheet, "A1", 1.0),
                    number(sheet, "A2", 2.0),
                    formula(sheet, "B1", "=A1+SUM(A1:A2)"),
                    formula(sheet, "C1", "=B1*2"),
                    formula(sheet, "D1", "=INDIRECT(\"A\"&A2)"),
                    formula(sheet, "E1", "=FORMULATEXT(B1)"),
                    WorkbookChange::set_cell_dynamic_formula(
                        sheet,
                        CellAddress::from_a1("G1").expect("address"),
                        crate::FormulaText::from_user_input("=SEQUENCE(2)").expect("formula"),
                        Some(
                            crate::CellRange::new(
                                CellAddress::from_a1("G1").expect("address"),
                                CellAddress::from_a1("G2").expect("address"),
                            )
                            .expect("range"),
                        ),
                    )
                    .expect("dynamic formula"),
                    formula(sheet, "H1", "=SUM(G1#)"),
                ]),
            )
            .expect("edits");
        assert!(session.precedents(cell(sheet, "B1")).is_none());
        assert!(session.dependents(cell(sheet, "A1")).is_none());
        session
            .recalculate(
                RecalculationMode::Full,
                CalculationOptions::default(),
                crate::CancellationToken::new(),
            )
            .expect("calculate");

        use PrecedentKind::{Spill, Value};
        let formula_text = PrecedentKind::FormulaText;
        assert_eq!(
            described(&session, "B1"),
            (
                vec![("A1".to_owned(), Value), ("A1:A2".to_owned(), Value)],
                false
            )
        );
        assert_eq!(
            described(&session, "C1"),
            (vec![("B1".to_owned(), Value)], false)
        );
        assert_eq!(
            described(&session, "E1"),
            (vec![("B1".to_owned(), formula_text)], false)
        );
        assert_eq!(
            described(&session, "H1"),
            (vec![("G1".to_owned(), Spill)], false)
        );
        let (indirect, dynamic) = described(&session, "D1");
        assert!(dynamic, "INDIRECT resolves its reference from values");
        assert!(indirect.contains(&("A2".to_owned(), Value)), "{indirect:?}");
        assert_eq!(described(&session, "A1"), (Vec::new(), false));

        let dependents = session
            .dependents(cell(sheet, "B1"))
            .expect("current calculation");
        let listed = dependents
            .dependents()
            .iter()
            .map(|dependent| {
                (
                    dependent.cell().address().to_string(),
                    dependent.is_dynamic(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(listed, [("C1".to_owned(), false), ("E1".to_owned(), false)]);
        assert_eq!(dependents.unlisted_dynamic_formulas(), 1);
        let dependents = session
            .dependents(cell(sheet, "A2"))
            .expect("current calculation");
        let listed = dependents
            .dependents()
            .iter()
            .map(|dependent| {
                (
                    dependent.cell().address().to_string(),
                    dependent.is_dynamic(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(listed, [("B1".to_owned(), false), ("D1".to_owned(), true)]);
        assert_eq!(dependents.unlisted_dynamic_formulas(), 0);
        let spill = session
            .dependents(cell(sheet, "G2"))
            .expect("current calculation");
        assert!(
            spill
                .dependents()
                .iter()
                .any(|dependent| dependent.cell() == cell(sheet, "G1")),
            "a value in a spill range blocks its anchor"
        );

        session
            .apply_changes(1, EditBatch::new([number(sheet, "A1", 5.0)]))
            .expect("edit");
        assert!(
            session.precedents(cell(sheet, "B1")).is_none(),
            "an edit leaves the graph behind until the next calculation"
        );
    }
}
