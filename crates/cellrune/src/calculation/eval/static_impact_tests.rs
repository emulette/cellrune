use std::collections::BTreeSet;

use crate::calculation::ast::{Expr, RefBody, Reference};
use crate::calculation::parser::parse_formula_with_limits;
use crate::xlsx::RecalculationWritePolicy;
use crate::{
    CalculationCellId, CalculationOptions, CellAddress, CellContent, CellValue, DefinedName,
    DefinedNameScope, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, EditBatch, FormulaText, OpenOptions,
    RecalculationWriteOptions, SheetId, SheetName, WorkbookChange, WorkbookDraft, WorkbookSnapshot,
    WriteOptions, calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
    write_xlsx_draft_edits_bytes,
};

/// The syntactic superset check: a formula whose syntax reaches an edit must be in the affected
/// set, whether or not its result can be calculated.
///
/// The reach is gathered here independently of the dependency analysis. It follows every call
/// argument, expands every defined name with the formula's name text in any scope, and covers the
/// whole box around a range whose end is computed. The fixture therefore holds no functions that
/// read only a reference's shape, such as ROWS, whose values an edit cannot change.
#[test]
fn affected_formulas_cover_every_formula_whose_syntax_reaches_an_edit() {
    let document = superset_document();
    let sheet = |name: &str| {
        document
            .workbook()
            .sheet_by_name(name)
            .expect("fixture sheet")
            .id()
    };
    let (data, other, third) = (sheet("Data"), sheet("Other"), sheet("Third"));
    let targets = [
        (data, "A1"),
        (data, "A2"),
        (data, "A3"),
        (data, "A4"),
        (data, "A5"),
        (data, "A6"),
        (data, "A7"),
        (data, "A9"),
        (data, "C5"),
        (data, "C7"),
        (data, "B1"),
        (other, "A1"),
        (third, "A1"),
    ];
    let mut state = 0x2545_f491_u32;
    let mut next = |bound: usize| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as usize % bound
    };
    for sequence in 0..160 {
        let mut draft = WorkbookDraft::from_document(&document);
        for _ in 0..=next(3) {
            let (sheet_id, target) = targets[next(targets.len())];
            let target = address(target);
            let change = match next(10) {
                9 => WorkbookChange::delete_sheet([other, third][next(2)]),
                0 => WorkbookChange::clear_cell(sheet_id, target),
                1 => WorkbookChange::set_cell_formula(sheet_id, target, formula("A8+1")),
                2 => WorkbookChange::set_defined_name(name(
                    ["Rate", "Base", "Fresh"][next(3)],
                    DefinedNameScope::Workbook,
                    ["Data!$A$6", "Data!$A$4", "7"][next(3)],
                )),
                3 => WorkbookChange::set_defined_name(name(
                    "Local",
                    DefinedNameScope::Sheet(other),
                    "Data!$A$3",
                )),
                4 => WorkbookChange::remove_defined_name(DefinedNameScope::Workbook, "Base"),
                5 => WorkbookChange::add_sheet(SheetName::new("New").expect("sheet name")),
                6 => WorkbookChange::rename_sheet(
                    third,
                    SheetName::new(["Fourth", "Third"][next(2)]).expect("sheet name"),
                ),
                7 => WorkbookChange::set_cell_value(
                    sheet_id,
                    target,
                    CellValue::Text(format!("t{}", next(4))),
                ),
                _ => WorkbookChange::set_cell_value(sheet_id, target, number(next(9) as f64)),
            };
            // An edit the draft rejects simply does not happen.
            let _ = draft.apply_changes(EditBatch::new([change]));
        }
        let output = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
            .unwrap_or_else(|error| panic!("sequence {sequence} must save: {error}"));
        let affected = output
            .report()
            .affected_formulas()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        for formula in reached_formulas(document.workbook(), draft.workbook()) {
            assert!(
                affected.contains(&formula),
                "sequence {sequence}: {formula:?} reaches an edit but is not affected"
            );
        }
    }
}

/// An OFFSET whose position the draft resolves without calculating anything is affected only by
/// the cells it reads; INDIRECT and an OFFSET whose position comes from a formula are affected by
/// any edit.
#[test]
fn resolvable_offsets_are_affected_only_by_the_cells_they_read() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (cell, value) in [
        ("A1", 1.0),
        ("A2", 2.0),
        ("A3", 3.0),
        ("A4", 4.0),
        ("C1", 2.0),
    ] {
        draft
            .set_cell_value(sheet, address(cell), number(value))
            .expect("value");
    }
    for (cell, text) in [
        ("B1", "OFFSET(A1,1,0)"),
        ("B2", "OFFSET(A1,C1,0)"),
        ("B3", "INDIRECT(\"A4\")"),
        ("B4", "OFFSET(A1,D1,0)"),
        ("D1", "1+0"),
    ] {
        draft
            .set_cell_formula(sheet, address(cell), formula(text))
            .expect("formula");
    }
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("fixture output");
    let document =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("fixture");
    let affected_by = |cell: &str| {
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .set_cell_value(sheet, address(cell), number(10.0))
            .expect("edit");
        write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
            .expect("edited output")
            .report()
            .affected_formulas()
            .iter()
            .map(|formula| formula.address())
            .collect::<BTreeSet<_>>()
    };
    let addresses = |cells: &[&str]| {
        cells
            .iter()
            .map(|cell| address(cell))
            .collect::<BTreeSet<_>>()
    };

    assert_eq!(affected_by("Z9"), addresses(&["B3", "B4"]));
    assert_eq!(affected_by("A2"), addresses(&["B1", "B3", "B4"]));
    assert_eq!(affected_by("A3"), addresses(&["B2", "B3", "B4"]));
    assert_eq!(affected_by("C1"), addresses(&["B2", "B3", "B4"]));
}

/// A workbook whose formulas reach their inputs in every way the analysis traces: plain, whole
/// column, `INDEX`-ended, and 3-D ranges, workbook and sheet-scoped names, a sheet-qualified
/// name, nested names, a name holding a rejected call, a sheet that does not exist yet, LET and
/// LAMBDA, and chains through other formulas.
fn superset_document() -> crate::XlsxDocument {
    let mut draft = WorkbookDraft::new();
    let data = draft.workbook().sheets()[0].id();
    draft
        .rename_sheet(data, SheetName::new("Data").expect("sheet name"))
        .expect("rename");
    let other = draft
        .add_sheet(SheetName::new("Other").expect("sheet name"))
        .expect("Other");
    let third = draft
        .add_sheet(SheetName::new("Third").expect("sheet name"))
        .expect("Third");
    for row in 1..=9 {
        draft
            .set_cell_value(
                data,
                CellAddress::from_indices(row, 1).expect("address"),
                number(f64::from(row)),
            )
            .expect("value");
    }
    for (sheet_id, cell) in [(other, "A1"), (third, "A1"), (data, "C5"), (data, "C7")] {
        draft
            .set_cell_value(sheet_id, address(cell), number(2.0))
            .expect("value");
    }
    for defined in [
        name("Rate", DefinedNameScope::Workbook, "Data!$A$5"),
        name("Base", DefinedNameScope::Workbook, "Data!$A$1"),
        name("Chain", DefinedNameScope::Workbook, "Rate+Base"),
        name("BadCall", DefinedNameScope::Workbook, "ROUND(Data!$A$3)"),
        name(
            "Pick",
            DefinedNameScope::Workbook,
            "INDEX(Data!$A$1:$A$9,Data!$A$1)",
        ),
        name("Local", DefinedNameScope::Sheet(other), "Data!$A$2"),
    ] {
        draft.set_defined_name(defined).expect("defined name");
    }
    for (sheet_id, cell, text) in [
        (data, "B1", "SUM(A1:A3)"),
        (data, "B2", "A4*0"),
        (data, "B3", "SUM(A:A)"),
        (data, "B4", "SUM(C5:INDEX(A1:A9,A1))"),
        (data, "B5", "Rate*2"),
        (data, "B6", "Other!Local+1"),
        (data, "B7", "Chain"),
        (data, "B8", "IFERROR(New!A1,-1)"),
        (data, "B9", "SUM(Other:Third!A1)"),
        (data, "B10", "BadCall"),
        (data, "B11", "B1*2"),
        (data, "B12", "LET(x,A6,x*2)"),
        (data, "B13", "SUM(MAP(A7:A8,LAMBDA(v,v+1)))"),
        (data, "B14", "ROUND(A9)"),
        (data, "B15", "Pick+0"),
        (data, "B16", "IF(FALSE,A2,0)"),
        (other, "B1", "Data!A1*2"),
        (third, "B1", "Local+Data!B11"),
    ] {
        draft
            .set_cell_formula(sheet_id, address(cell), formula(text))
            .expect("formula");
    }
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(
        &draft,
        &calculation,
        RecalculationWriteOptions::default()
            .with_policy(RecalculationWritePolicy::InvalidateUnavailable),
    )
    .expect("fixture output");
    open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("fixture document")
}

/// Returns the formulas of `draft` whose syntax reaches a change from `source`, and every formula
/// whose syntax reaches one of those.
fn reached_formulas(
    source: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
) -> BTreeSet<CalculationCellId> {
    let changed_cells = changed_cells(source, draft);
    let changed_names = changed_names(source, draft);
    let changed_sheets = draft
        .sheets()
        .iter()
        .flat_map(|sheet| match source.sheet_by_id(sheet.id()) {
            Some(original) if original.name() == sheet.name() => Vec::new(),
            Some(original) => vec![key(original.name().as_str()), key(sheet.name().as_str())],
            None => vec![key(sheet.name().as_str())],
        })
        .chain(
            source
                .sheets()
                .iter()
                .filter(|sheet| draft.sheet_by_id(sheet.id()).is_none())
                .map(|sheet| key(sheet.name().as_str())),
        )
        .collect::<BTreeSet<_>>();
    let anything_changed =
        !changed_cells.is_empty() || !changed_names.is_empty() || !changed_sheets.is_empty();

    let mut formulas = Vec::new();
    for sheet in draft.sheets() {
        for cell in sheet.cells() {
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let reach = formula
                .text()
                .map_or_else(Reach::unknown, |text| Reach::of(draft, text.as_str()));
            formulas.push((CalculationCellId::new(sheet.id(), cell.address()), reach));
        }
    }
    let mut reached = BTreeSet::new();
    for (id, reach) in &formulas {
        if changed_cells.contains(id)
            || (anything_changed && reach.unknown)
            || !reach.names.is_disjoint(&changed_names)
            || !reach.sheets.is_disjoint(&changed_sheets)
            || (reach.sheet_list && !changed_sheets.is_empty())
            || reach.covers_any(draft, id.sheet_id(), &changed_cells)
        {
            reached.insert(*id);
        }
    }
    loop {
        let before = reached.len();
        for (id, reach) in &formulas {
            if !reached.contains(id) && reach.covers_any(draft, id.sheet_id(), &reached) {
                reached.insert(*id);
            }
        }
        if reached.len() == before {
            return reached;
        }
    }
}

fn changed_cells(
    source: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
) -> BTreeSet<CalculationCellId> {
    let mut changed = BTreeSet::new();
    for sheet in draft.sheets() {
        let original = source.sheet_by_id(sheet.id());
        let addresses = sheet
            .cells()
            .map(crate::Cell::address)
            .chain(
                original
                    .into_iter()
                    .flat_map(|sheet| sheet.cells().map(crate::Cell::address)),
            )
            .collect::<BTreeSet<_>>();
        for address in addresses {
            let before = original.and_then(|sheet| sheet.cell(address));
            let after = sheet.cell(address);
            let same = match (before, after) {
                (Some(before), Some(after)) => {
                    before.content() == after.content()
                        && before.number_format() == after.number_format()
                }
                (before, after) => before.is_none() && after.is_none(),
            };
            if !same {
                changed.insert(CalculationCellId::new(sheet.id(), address));
            }
        }
    }
    changed
}

fn changed_names(source: &WorkbookSnapshot, draft: &WorkbookSnapshot) -> BTreeSet<String> {
    let definitions = |workbook: &WorkbookSnapshot| {
        workbook
            .defined_names()
            .iter()
            .map(|name| {
                (
                    (name.scope(), key(name.name())),
                    name.formula().as_str().to_owned(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let (before, after) = (definitions(source), definitions(draft));
    before
        .keys()
        .chain(after.keys())
        .filter(|name| before.get(name) != after.get(name))
        .map(|(_, name)| name.clone())
        .collect()
}

/// What a formula's syntax can reach.
#[derive(Default)]
struct Reach {
    areas: Vec<Area>,
    names: BTreeSet<String>,
    sheets: BTreeSet<String>,
    sheet_list: bool,
    /// The formula's references cannot be read from its syntax.
    unknown: bool,
}

/// A rectangle on the formula's own sheet (`sheet` is `None`) or on the named sheet.
#[derive(Clone)]
struct Area {
    sheet: Option<String>,
    rows: (u32, u32),
    columns: (u32, u32),
}

impl Reach {
    fn unknown() -> Self {
        Self {
            unknown: true,
            ..Self::default()
        }
    }

    fn of(workbook: &WorkbookSnapshot, text: &str) -> Self {
        let mut reach = Self::default();
        reach.walk_formula(workbook, text, &mut Vec::new());
        reach
    }

    fn walk_formula(
        &mut self,
        workbook: &WorkbookSnapshot,
        text: &str,
        expanding: &mut Vec<String>,
    ) {
        match parse_formula_with_limits(text, CalculationOptions::default().limits()) {
            Ok(parsed) => {
                let areas = self.walk(workbook, parsed.root(), expanding);
                self.areas.extend(areas);
            }
            Err(_) => self.unknown = true,
        }
    }

    /// Returns the areas `expr` can reach, recording names and sheets on the way.
    fn walk(
        &mut self,
        workbook: &WorkbookSnapshot,
        expr: &Expr,
        expanding: &mut Vec<String>,
    ) -> Vec<Area> {
        match expr {
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::BuiltinCallable(_)
            | Expr::Missing => Vec::new(),
            Expr::Ref(reference) => self.reference(workbook, reference),
            // The fixture holds none of these.
            Expr::StructuredRef(_) | Expr::SpillRef(_) | Expr::ExternalReference(_) => {
                self.unknown = true;
                Vec::new()
            }
            Expr::Range { start, end } => {
                let mut areas = self.walk(workbook, start, expanding);
                areas.extend(self.walk(workbook, end, expanding));
                // A computed end can land anywhere its arguments reach.
                if let Some(first) = areas.first().cloned() {
                    if areas.iter().all(|area| area.sheet == first.sheet) {
                        areas.push(Area {
                            sheet: first.sheet,
                            rows: (
                                areas.iter().map(|area| area.rows.0).min().unwrap_or(1),
                                areas.iter().map(|area| area.rows.1).max().unwrap_or(1),
                            ),
                            columns: (
                                areas.iter().map(|area| area.columns.0).min().unwrap_or(1),
                                areas.iter().map(|area| area.columns.1).max().unwrap_or(1),
                            ),
                        });
                    } else {
                        self.unknown = true;
                    }
                }
                areas
            }
            Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Binary { left, right, .. } => {
                let mut areas = self.walk(workbook, left, expanding);
                areas.extend(self.walk(workbook, right, expanding));
                areas
            }
            Expr::QualifiedName { sheet, name } => {
                self.sheets.insert(key(&sheet.name));
                self.name(workbook, name, expanding)
            }
            Expr::Name(name) => self.name(workbook, name, expanding),
            Expr::Call { name, args } => {
                let bare = name.trim_start_matches("_xlfn.").to_ascii_uppercase();
                if matches!(bare.as_str(), "INDIRECT" | "OFFSET") {
                    self.unknown = true;
                }
                let mut areas = self.name(workbook, name, expanding);
                for arg in args {
                    areas.extend(self.walk(workbook, arg, expanding));
                }
                areas
            }
            Expr::Invoke { callee, args } => {
                let mut areas = self.walk(workbook, callee, expanding);
                for arg in args {
                    areas.extend(self.walk(workbook, arg, expanding));
                }
                areas
            }
            Expr::ImplicitIntersection(inner)
            | Expr::Unary { operand: inner, .. }
            | Expr::Paren(inner) => self.walk(workbook, inner, expanding),
            Expr::Array(rows) => rows
                .iter()
                .flatten()
                .flat_map(|item| self.walk(workbook, item, expanding))
                .collect(),
        }
    }

    /// Records a name and returns what every definition with that name text, in any scope, reaches.
    fn name(
        &mut self,
        workbook: &WorkbookSnapshot,
        name: &str,
        expanding: &mut Vec<String>,
    ) -> Vec<Area> {
        let name = key(name);
        self.names.insert(name.clone());
        if expanding.contains(&name) {
            return Vec::new();
        }
        expanding.push(name.clone());
        let before = self.areas.len();
        for defined in workbook.defined_names() {
            if key(defined.name()) == name {
                self.walk_formula(workbook, defined.formula().as_str(), expanding);
            }
        }
        expanding.pop();
        self.areas.split_off(before)
    }

    fn reference(&mut self, workbook: &WorkbookSnapshot, reference: &Reference) -> Vec<Area> {
        let (rows, columns) = match reference.body {
            RefBody::Cell(cell) => ((cell.row, cell.row), (cell.column, cell.column)),
            RefBody::Area(start, end) => (
                (start.row.min(end.row), start.row.max(end.row)),
                (start.column.min(end.column), start.column.max(end.column)),
            ),
            RefBody::Columns(start, end) => (
                (1, EXCEL_MAX_ROWS),
                (start.column.min(end.column), start.column.max(end.column)),
            ),
            RefBody::Rows(start, end) => (
                (start.row.min(end.row), start.row.max(end.row)),
                (1, EXCEL_MAX_COLUMNS),
            ),
        };
        let area = |sheet| Area {
            sheet,
            rows,
            columns,
        };
        match &reference.sheet {
            None => vec![area(None)],
            Some(prefix) => {
                self.sheets.insert(key(&prefix.name));
                match &prefix.end_name {
                    Some(end) => {
                        // A 3-D range reads the sheet list and every sheet between its ends.
                        self.sheets.insert(key(end));
                        self.sheet_list = true;
                        let sheets = workbook.sheets();
                        let position = |name: &str| {
                            sheets
                                .iter()
                                .position(|sheet| key(sheet.name().as_str()) == key(name))
                        };
                        match (position(&prefix.name), position(end)) {
                            (Some(start), Some(end)) => sheets[start.min(end)..=start.max(end)]
                                .iter()
                                .map(|sheet| area(Some(key(sheet.name().as_str()))))
                                .collect(),
                            _ => Vec::new(),
                        }
                    }
                    None => vec![area(Some(key(&prefix.name)))],
                }
            }
        }
    }

    fn covers_any(
        &self,
        workbook: &WorkbookSnapshot,
        own_sheet: SheetId,
        cells: &BTreeSet<CalculationCellId>,
    ) -> bool {
        self.areas.iter().any(|area| {
            let sheet_id = match &area.sheet {
                None => Some(own_sheet),
                Some(name) => workbook
                    .sheets()
                    .iter()
                    .find(|sheet| key(sheet.name().as_str()) == *name)
                    .map(crate::Sheet::id),
            };
            sheet_id.is_some_and(|sheet_id| {
                cells.iter().any(|cell| {
                    cell.sheet_id() == sheet_id
                        && (area.rows.0..=area.rows.1).contains(&cell.address().row().get())
                        && (area.columns.0..=area.columns.1)
                            .contains(&cell.address().column().get())
                })
            })
        })
    }
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

fn name(name: &str, scope: DefinedNameScope, text: &str) -> DefinedName {
    DefinedName::new(name, scope, formula(text), false).expect("defined name")
}

fn formula(text: &str) -> FormulaText {
    FormulaText::from_user_input(format!("={text}")).expect("formula")
}

fn address(text: &str) -> CellAddress {
    CellAddress::from_a1(text).expect("address")
}

fn number(value: f64) -> CellValue {
    CellValue::number(value).expect("finite value")
}
