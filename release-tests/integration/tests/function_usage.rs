use cellrune::{
    CalculationCellId, CalculationLimits, CalculationOptions, CellAddress, DefinedName,
    DefinedNameScope, FormulaText, SheetId, WorkbookDraft, scan_function_usage,
    scan_function_usage_with_options,
};

fn name(draft: &mut WorkbookDraft, name: &str, formula: &str, scope: DefinedNameScope) {
    draft
        .set_defined_name(
            DefinedName::new(name, scope, FormulaText::from_xlsx(formula).unwrap(), false).unwrap(),
        )
        .unwrap();
}

fn formula(draft: &mut WorkbookDraft, sheet: SheetId, address: &str, text: &str) {
    draft
        .set_cell_formula(
            sheet,
            CellAddress::from_a1(address).unwrap(),
            FormulaText::from_xlsx(text).unwrap(),
        )
        .unwrap();
}

fn repeated_names(depth: usize) -> WorkbookDraft {
    let mut draft = WorkbookDraft::new();
    for level in 0..=depth {
        let expression = if level == 0 {
            "SUM(1)".to_owned()
        } else {
            format!("Name_{}+Name_{}", level - 1, level - 1)
        };
        name(
            &mut draft,
            &format!("Name_{level}"),
            &expression,
            DefinedNameScope::Workbook,
        );
    }
    draft
}

#[test]
fn repeated_named_calls_keep_multiplicity_distinct_from_formula_counts_and_samples() {
    let mut draft = repeated_names(8);
    let sheet = draft.workbook().sheets()[0].id();
    for row in 1..=10 {
        formula(&mut draft, sheet, &format!("A{row}"), "Name_8");
    }
    let report = scan_function_usage(draft.workbook());
    let entry = &report.entries()[0];
    assert_eq!(entry.name(), "SUM");
    assert_eq!(entry.call_count(), 2560);
    assert_eq!(entry.formula_count(), 10);
    let expected: Vec<_> = (1..=8)
        .map(|row| CalculationCellId::new(sheet, CellAddress::from_a1(&format!("A{row}")).unwrap()))
        .collect();
    assert_eq!(entry.sample_cells(), expected);
    assert_eq!(scan_function_usage(draft.workbook()), report);

    let old = draft.workbook().clone();
    name(&mut draft, "Name_0", "MIN(1)", DefinedNameScope::Workbook);
    assert_eq!(scan_function_usage(&old), report);
    assert_eq!(
        scan_function_usage(draft.workbook()).entries()[0].name(),
        "MIN"
    );
    let restricted = CalculationOptions::default().with_limits(
        CalculationLimits::default()
            .with_max_formula_ast_nodes(1)
            .unwrap(),
    );
    let _restricted = scan_function_usage_with_options(&old, restricted);
    assert_eq!(scan_function_usage(&old), report);
}

#[test]
fn cycles_are_cut_per_path_and_keep_both_entry_points() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    name(
        &mut draft,
        "Alpha",
        "SUM(1)+Beta",
        DefinedNameScope::Workbook,
    );
    name(
        &mut draft,
        "Beta",
        "MIN(2)+Alpha",
        DefinedNameScope::Workbook,
    );
    formula(&mut draft, sheet, "A1", "Alpha+Alpha");
    formula(&mut draft, sheet, "A2", "Beta");
    let report = scan_function_usage(draft.workbook());
    assert_eq!(report.entries().len(), 2);
    for entry in report.entries() {
        assert_eq!(entry.call_count(), 3);
        assert_eq!(entry.formula_count(), 2);
    }
}

#[test]
fn let_scope_and_workbook_definitions_do_not_share_local_bindings() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    name(&mut draft, "Base", "SUM(1)", DefinedNameScope::Workbook);
    name(&mut draft, "Outer", "Base+Base", DefinedNameScope::Workbook);
    formula(&mut draft, sheet, "A1", "LET(Base,MIN(2),Outer+Base)");
    formula(&mut draft, sheet, "A2", "Outer");
    let report = scan_function_usage(draft.workbook());
    let actual: Vec<_> = report
        .entries()
        .iter()
        .map(|entry| (entry.name(), entry.call_count(), entry.formula_count()))
        .collect();
    assert_eq!(actual, [("LET", 1, 1), ("MIN", 1, 1), ("SUM", 4, 2)]);
}
