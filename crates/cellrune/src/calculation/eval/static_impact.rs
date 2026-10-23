//! Formulas a draft edit can affect, found without evaluating the workbook.
//!
//! The analysis parses every formula of the edited workbook, collects its static dependency
//! targets, and indexes them in reverse. It never evaluates a cell, so it cannot rely on results
//! the way session impact analysis does: anything whose references depend on values is treated as
//! affected. The answer may include formulas an edit cannot change; it must never miss one.

use std::collections::{BTreeMap, BTreeSet};

use super::dependency::DependencyTarget;
use super::{DependencyImpactIndex, Engine};
use crate::calculation::ast::{Expr, SheetPrefix};
use crate::calculation::functions::{
    function_call_shape_is_valid, function_evaluator, normalize_name,
};
use crate::calculation::parser::parse_formula_with_limits;
use crate::calculation::runtime::CellId;
use crate::{
    CalculationCellId, CalculationOptions, CellContent, CellValue, ExcelError, FormulaMetadata,
    SavedResult, TableId, WorkbookSnapshot,
};

/// The edits whose consequences the analysis traces.
pub(crate) struct DraftEdits<'a> {
    /// The workbook the draft started from.
    pub(crate) source: &'a WorkbookSnapshot,
    /// The edited workbook.
    pub(crate) draft: &'a WorkbookSnapshot,
    /// Cells whose calculation-relevant content was written or removed.
    pub(crate) changed_cells: &'a BTreeSet<CalculationCellId>,
    /// Tables whose range, name, or columns changed.
    pub(crate) changed_tables: &'a BTreeSet<TableId>,
}

/// Returns every formula of the edited workbook whose result the edits can change.
///
/// Formulas whose references cannot be determined without evaluation are always included once
/// anything changed: dynamic references, formulas that do not parse or exceed an analysis limit,
/// formulas with calls the call contract rejects, and formula containers without text. When the
/// dependency analysis itself exceeds its limit, every formula is included.
pub(crate) fn affected_formulas(
    edits: &DraftEdits<'_>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let source = edits.source;
    let draft = edits.draft;
    if source.date_system() != draft.date_system()
        || source.calculation_hints() != draft.calculation_hints()
    {
        return all_formulas(draft, cancelled);
    }
    let changed_names = changed_defined_names(source, draft);
    // Old and new names of added, renamed, and deleted sheets: a reference to one of them may
    // resolve differently now, and the sheet list itself changed.
    let changed_sheet_names = draft
        .sheets()
        .iter()
        .flat_map(|sheet| match source.sheet_by_id(sheet.id()) {
            Some(original) if original.name() == sheet.name() => Vec::new(),
            Some(original) => vec![original.name().as_str(), sheet.name().as_str()],
            None => vec![sheet.name().as_str()],
        })
        .chain(
            source
                .sheets()
                .iter()
                .filter(|sheet| draft.sheet_by_id(sheet.id()).is_none())
                .map(|sheet| sheet.name().as_str()),
        )
        .map(case_insensitive_key)
        .collect::<BTreeSet<_>>();
    // A moved tab changes the sheet list, which 3-D references and sheet functions read, but not
    // what a reference to one sheet reads.
    let tab_ids = |workbook: &WorkbookSnapshot| {
        workbook
            .tab_order()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>()
    };
    let sheets_changed = !changed_sheet_names.is_empty() || tab_ids(source) != tab_ids(draft);
    let sheet_changes = SheetListChange {
        names: &changed_sheet_names,
        list_changed: sheets_changed,
    };
    if edits.changed_cells.is_empty()
        && changed_names.is_empty()
        && edits.changed_tables.is_empty()
        && !sheets_changed
    {
        return Ok(BTreeSet::new());
    }

    let engine = Engine::parsed_cancellable(draft, CalculationOptions::default(), cancelled)?;
    let targets = engine.dependency_targets_cancellable(cancelled)?;
    let collected = engine.dependencies_cancellable(cancelled)?;
    if !collected.limited.is_empty() {
        return all_formulas(draft, cancelled);
    }
    let index =
        DependencyImpactIndex::build(&targets, &collected.graph, &engine.array_regions, cancelled)?;

    let changed_table_names = edits
        .changed_tables
        .iter()
        .flat_map(|id| [source.table_by_id(*id), draft.table_by_id(*id)])
        .flatten()
        .map(|table| case_insensitive_key(table.display_name().as_str()))
        .collect::<BTreeSet<_>>();
    let affected_names = affected_defined_names(
        source,
        draft,
        changed_names,
        &changed_table_names,
        sheet_changes,
        cancelled,
    )?;

    let mut direct = BTreeSet::new();
    let mut charge = || if cancelled() { Err(()) } else { Ok(()) };
    let mut removed_cell = false;
    for changed in edits.changed_cells {
        charge()?;
        removed_cell |= draft
            .sheet_by_id(changed.sheet_id())
            .is_none_or(|sheet| sheet.cell(changed.address()).is_none());
        let Some(internal) = internal_cell(draft, *changed) else {
            continue;
        };
        direct.extend(index.formulas_for_cell(internal, &mut charge)?);
        if engine.asts.contains_key(&internal) || engine.parse_failures.contains_key(&internal) {
            direct.insert(internal);
        }
    }

    for (sheet_index, sheet) in draft.sheets().iter().enumerate() {
        for cell in sheet.cells() {
            charge()?;
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let internal = (
                sheet_index,
                cell.address().row().get(),
                cell.address().column().get(),
            );
            let always = formula.text().is_none()
                || engine.parse_failures.contains_key(&internal)
                || engine.name_cycle_cells.contains(&internal)
                || engine.name_limit_cells.contains(&internal)
                || engine.formula_contains_dynamic_reference(internal, cancelled)
                || engine
                    .asts
                    .get(&internal)
                    .is_some_and(|parsed| contains_rejected_call(parsed.root()));
            // Clearing a cell can free the area a failed spill was blocked from.
            let unblocked_spill = removed_cell
                && matches!(formula.metadata(), FormulaMetadata::DynamicArray { .. })
                && *formula.saved_result()
                    == SavedResult::Present(CellValue::Error(ExcelError::Spill));
            let references_changed = engine.asts.get(&internal).is_some_and(|parsed| {
                let tokens = FormulaTokens::of(parsed.root());
                tokens.references(&affected_names)
                    || tokens.references(&changed_table_names)
                    || tokens.reads_sheets(sheet_changes)
            }) || targets.get(&internal).is_some_and(|formula_targets| {
                formula_targets.iter().any(|target| {
                    matches!(
                        target,
                        DependencyTarget::TableIdentity(table)
                            if edits.changed_tables.contains(&table.table_id())
                    )
                })
            });
            if always || unblocked_spill || references_changed {
                direct.insert(internal);
            }
        }
    }

    let mut affected = direct.clone();
    index.close_over_dependents(&mut affected, direct, &mut charge)?;
    Ok(affected
        .into_iter()
        .filter_map(|cell| public_cell(draft, cell))
        .collect())
}

fn all_formulas(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let mut formulas = BTreeSet::new();
    for sheet in workbook.sheets() {
        for address in sheet.formula_addresses() {
            if cancelled() {
                return Err(());
            }
            formulas.insert(CalculationCellId::new(sheet.id(), *address));
        }
    }
    Ok(formulas)
}

/// Names added, removed, or redefined by the draft, compared case-insensitively and across scopes.
fn changed_defined_names(source: &WorkbookSnapshot, draft: &WorkbookSnapshot) -> BTreeSet<String> {
    let definitions = |workbook: &WorkbookSnapshot| {
        workbook
            .defined_names()
            .iter()
            .map(|name| {
                (
                    (name.scope(), case_insensitive_key(name.name())),
                    name.formula().as_str().to_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let before = definitions(source);
    let after = definitions(draft);
    before
        .keys()
        .chain(after.keys())
        .filter(|key| before.get(key) != after.get(key))
        .map(|(_, name)| name.clone())
        .collect()
}

/// Closes the changed names over every name whose definition, before or after the edits, refers
/// to an affected name, a changed table, or a changed sheet or the sheet list. A definition whose
/// references cannot be known statically (it does not parse, or it holds a call the call contract
/// rejects) is affected by any edit.
fn affected_defined_names(
    source: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
    mut affected: BTreeSet<String>,
    changed_table_names: &BTreeSet<String>,
    sheet_changes: SheetListChange<'_>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<String>, ()> {
    let mut definitions = Vec::new();
    for name in source.defined_names().iter().chain(draft.defined_names()) {
        if cancelled() {
            return Err(());
        }
        let key = case_insensitive_key(name.name());
        match parse_formula_with_limits(
            name.formula().as_str(),
            CalculationOptions::default().limits(),
        ) {
            Ok(parsed) if !contains_rejected_call(parsed.root()) => {
                definitions.push((key, FormulaTokens::of(parsed.root())));
            }
            Ok(_) | Err(_) => {
                affected.insert(key);
            }
        }
    }
    for (key, tokens) in &definitions {
        if tokens.references(changed_table_names) || tokens.reads_sheets(sheet_changes) {
            affected.insert(key.clone());
        }
    }
    loop {
        if cancelled() {
            return Err(());
        }
        let before = affected.len();
        for (key, tokens) in &definitions {
            if !affected.contains(key) && tokens.references(&affected) {
                affected.insert(key.clone());
            }
        }
        if affected.len() == before {
            return Ok(affected);
        }
    }
}

/// The sheets an edit renamed, added, or deleted, and whether the sheet list or its order changed.
#[derive(Clone, Copy)]
struct SheetListChange<'a> {
    names: &'a BTreeSet<String>,
    list_changed: bool,
}

/// The names a formula mentions and whether it reads the sheet list, gathered from its syntax.
///
/// Names include defined names, LAMBDA names called like functions, table names, and names that
/// resolve to nothing yet, so a formula that reads `#NAME?` today is found when the name appears.
#[derive(Default)]
struct FormulaTokens {
    names: BTreeSet<String>,
    sheet_names: BTreeSet<String>,
    depends_on_sheet_list: bool,
}

impl FormulaTokens {
    fn of(root: &Expr) -> Self {
        let mut tokens = Self::default();
        let mut pending = vec![root];
        while let Some(expr) = pending.pop() {
            match expr {
                Expr::Name(name) => {
                    tokens.names.insert(case_insensitive_key(name));
                }
                Expr::QualifiedName { name, sheet } => {
                    tokens.names.insert(case_insensitive_key(name));
                    tokens.add_sheet(sheet);
                }
                Expr::BuiltinCallable(callable) => {
                    tokens
                        .names
                        .insert(case_insensitive_key(callable.canonical_name()));
                }
                Expr::StructuredRef(reference) => {
                    if let Some(table) = &reference.table {
                        tokens.names.insert(case_insensitive_key(table));
                    }
                }
                Expr::Ref(reference) => {
                    if let Some(sheet) = &reference.sheet {
                        tokens.add_sheet(sheet);
                    }
                }
                Expr::Call { name, args } => {
                    if function_evaluator(name).is_none() {
                        tokens.names.insert(case_insensitive_key(name));
                    }
                    tokens.depends_on_sheet_list |=
                        matches!(normalize_name(name).as_str(), "SHEET" | "SHEETS" | "CELL");
                    pending.extend(args);
                }
                Expr::Invoke { callee, args } => {
                    pending.push(callee);
                    pending.extend(args);
                }
                Expr::ImplicitIntersection(child)
                | Expr::SpillRef(child)
                | Expr::Unary { operand: child, .. }
                | Expr::Paren(child) => pending.push(child),
                Expr::Binary { left, right, .. }
                | Expr::ReferenceUnion { left, right }
                | Expr::ReferenceIntersection { left, right }
                | Expr::Range {
                    start: left,
                    end: right,
                } => {
                    pending.push(left);
                    pending.push(right);
                }
                Expr::Array(rows) => pending.extend(rows.iter().flatten()),
                Expr::Number(_)
                | Expr::Text(_)
                | Expr::Logical(_)
                | Expr::ErrorLit(_)
                | Expr::ExternalReference(_)
                | Expr::Missing => {}
            }
        }
        tokens
    }

    fn add_sheet(&mut self, sheet: &SheetPrefix) {
        self.sheet_names.insert(case_insensitive_key(&sheet.name));
        if let Some(end) = &sheet.end_name {
            self.sheet_names.insert(case_insensitive_key(end));
            self.depends_on_sheet_list = true;
        }
    }

    fn references(&self, names: &BTreeSet<String>) -> bool {
        !self.names.is_disjoint(names)
    }

    /// Reports whether the formula may read differently after sheets are added or renamed: it
    /// names one of those sheets, or it reads the sheet list through a 3-D reference or a
    /// sheet function.
    fn reads_sheets(&self, changes: SheetListChange<'_>) -> bool {
        (changes.list_changed && self.depends_on_sheet_list)
            || !self.sheet_names.is_disjoint(changes.names)
    }
}

/// Reports whether a formula holds a call the call contract rejects. Dependency collection skips
/// the arguments of such a call, so its references are unknown.
fn contains_rejected_call(root: &Expr) -> bool {
    let mut pending = vec![root];
    while let Some(expr) = pending.pop() {
        match expr {
            Expr::Call { name, args } => {
                if function_evaluator(name).is_some() && !function_call_shape_is_valid(name, args) {
                    return true;
                }
                pending.extend(args);
            }
            Expr::Invoke { callee, args } => {
                pending.push(callee);
                pending.extend(args);
            }
            Expr::ImplicitIntersection(child)
            | Expr::SpillRef(child)
            | Expr::Unary { operand: child, .. }
            | Expr::Paren(child) => pending.push(child),
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Range {
                start: left,
                end: right,
            } => {
                pending.push(left);
                pending.push(right);
            }
            Expr::Array(rows) => pending.extend(rows.iter().flatten()),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Name(_)
            | Expr::BuiltinCallable(_)
            | Expr::Missing => {}
        }
    }
    false
}

fn internal_cell(workbook: &WorkbookSnapshot, cell: CalculationCellId) -> Option<CellId> {
    let sheet = workbook.sheet_position(cell.sheet_id())?;
    Some((
        sheet,
        cell.address().row().get(),
        cell.address().column().get(),
    ))
}

fn public_cell(workbook: &WorkbookSnapshot, cell: CellId) -> Option<CalculationCellId> {
    let sheet = workbook.sheets().get(cell.0)?;
    let address = crate::CellAddress::from_indices(cell.1, cell.2).ok()?;
    Some(CalculationCellId::new(sheet.id(), address))
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
#[path = "static_impact_tests.rs"]
mod tests;
