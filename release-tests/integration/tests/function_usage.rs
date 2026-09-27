use cellrune::{
    CalculationCellId, CalculationLimits, CalculationOptions, CellAddress, DefinedName,
    DefinedNameScope, FormulaText, SheetId, SheetName, WorkbookDraft, scan_function_usage,
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

#[test]
fn compressed_counts_are_exact_above_the_javascript_integer_limit_and_saturate_at_u64() {
    for (expression, expected) in [
        ("Name_53+SUM(2)".to_owned(), (1_u64 << 53) + 1),
        ("Name_63".to_owned(), 1_u64 << 63),
        (
            (0..64)
                .map(|i| format!("Name_{i}"))
                .collect::<Vec<_>>()
                .join("+"),
            u64::MAX,
        ),
        ("Name_64".to_owned(), u64::MAX),
    ] {
        let mut draft = repeated_names(64);
        let sheet = draft.workbook().sheets()[0].id();
        formula(&mut draft, sheet, "A1", &expression);
        let report = scan_function_usage(draft.workbook());
        assert_eq!(report.entries()[0].call_count(), expected, "{expression}");
        assert_eq!(report.entries()[0].formula_count(), 1);
    }
    let mut draft = repeated_names(63);
    let sheet = draft.workbook().sheets()[0].id();
    formula(&mut draft, sheet, "A1", "Name_63");
    formula(&mut draft, sheet, "A2", "Name_63");
    let report = scan_function_usage(draft.workbook());
    assert_eq!(report.entries()[0].call_count(), u64::MAX);
    assert_eq!(report.entries()[0].formula_count(), 2);
}

#[test]
fn deep_name_chains_do_not_consume_the_host_call_stack() {
    let mut draft = WorkbookDraft::new();
    for i in 0..2048 {
        let text = if i == 0 {
            "SUM(1)".to_owned()
        } else {
            format!("Chain_{}", i - 1)
        };
        name(
            &mut draft,
            &format!("Chain_{i}"),
            &text,
            DefinedNameScope::Workbook,
        );
    }
    let sheet = draft.workbook().sheets()[0].id();
    formula(&mut draft, sheet, "A1", "Chain_2047");
    let report = scan_function_usage(draft.workbook());
    assert_eq!(report.entries()[0].call_count(), 1);
}

#[test]
fn deep_callable_aliases_preserve_argument_reachability_without_host_recursion() {
    for with_let in [false, true] {
        let mut draft = WorkbookDraft::new();
        let sheet = draft.workbook().sheets()[0].id();
        for level in 0..2048 {
            name(
                &mut draft,
                &format!("Callable_{level}"),
                &if level == 0 {
                    "LAMBDA(x,SUM(x))".to_owned()
                } else if with_let {
                    format!("LET(f,Callable_{},f)", level - 1)
                } else {
                    format!("Callable_{}", level - 1)
                },
                DefinedNameScope::Workbook,
            );
        }
        formula(&mut draft, sheet, "A1", "Callable_2047(MIN(1))");
        let report = scan_function_usage(draft.workbook());
        let counts: Vec<_> = report
            .entries()
            .iter()
            .map(|entry| (entry.name(), entry.call_count()))
            .collect();
        let expected = if with_let {
            vec![("LAMBDA", 1), ("LET", 2047), ("MIN", 1), ("SUM", 1)]
        } else {
            vec![("LAMBDA", 1), ("MIN", 1), ("SUM", 1)]
        };
        assert_eq!(counts, expected);
    }
}

#[test]
fn local_names_on_different_sheets_are_separate_from_workbook_definition_scope() {
    let mut draft = WorkbookDraft::new();
    let first = draft.workbook().sheets()[0].id();
    let second = draft.add_sheet(SheetName::new("Second").unwrap()).unwrap();
    name(&mut draft, "Base", "SUM(1)", DefinedNameScope::Workbook);
    name(&mut draft, "Outer", "Base+Base", DefinedNameScope::Workbook);
    name(&mut draft, "Base", "MIN(2)", DefinedNameScope::Sheet(first));
    name(
        &mut draft,
        "Base",
        "MAX(3)",
        DefinedNameScope::Sheet(second),
    );
    formula(&mut draft, first, "A1", "Outer+Base");
    formula(&mut draft, second, "A1", "Outer+Base");
    let report = scan_function_usage(draft.workbook());
    let counts: Vec<_> = report
        .entries()
        .iter()
        .map(|e| (e.name(), e.call_count(), e.formula_count()))
        .collect();
    assert_eq!(counts, [("MAX", 1, 1), ("MIN", 1, 1), ("SUM", 4, 2)]);
}

#[test]
fn cyclic_partial_summaries_do_not_poison_later_roots_or_shared_acyclic_children() {
    let mut draft = repeated_names(8);
    let sheet = draft.workbook().sheets()[0].id();
    name(
        &mut draft,
        "Alpha",
        "Name_8+Beta+Name_8",
        DefinedNameScope::Workbook,
    );
    name(
        &mut draft,
        "Beta",
        "MIN(1)+Alpha",
        DefinedNameScope::Workbook,
    );
    for (address, text) in [("A1", "Alpha"), ("A2", "Beta"), ("A3", "Name_8")] {
        formula(&mut draft, sheet, address, text);
    }
    let report = scan_function_usage(draft.workbook());
    let counts: Vec<_> = report
        .entries()
        .iter()
        .map(|e| (e.name(), e.call_count(), e.formula_count()))
        .collect();
    assert_eq!(counts, [("MIN", 2, 2), ("SUM", 1280, 3)]);
}
