use std::cell::Cell;

use super::*;
use crate::{
    CalculationHints, CalculationLimits, CellAddress, DateSystem, DefinedName, DefinedNameScope,
    FormulaText, Provenance, ProviderIdentity, Sheet, SheetId, SheetName, SheetVisibility, Table,
    TableColumn, TableId, TableName, WorkbookSource,
};

fn sheet_id(value: u32) -> SheetId {
    SheetId::new(value).expect("test sheet ID is non-zero")
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("test formula is non-empty XLSX syntax")
}

fn defined_name(name: &str, scope: DefinedNameScope, value: &str) -> DefinedName {
    DefinedName::new(name, scope, formula(value), false).expect("test defined name is valid")
}

fn sheet(id: u32, name: &str) -> Sheet {
    Sheet::new(
        sheet_id(id),
        SheetName::new(name).expect("test sheet name is valid"),
        SheetVisibility::Visible,
    )
}

fn workbook_with_sheets(sheets: Vec<Sheet>, names: Vec<DefinedName>) -> WorkbookSnapshot {
    WorkbookSnapshot::new_with_metadata(
        sheets,
        names,
        Vec::new(),
        DateSystem::Excel1900,
        CalculationHints::default(),
        WorkbookSource::default(),
        Provenance::new(
            ProviderIdentity::new("defined-name-analysis-test", "1")
                .expect("test provider identity is valid"),
            None,
        ),
    )
    .expect("test workbook is valid")
}

fn workbook(names: Vec<DefinedName>) -> WorkbookSnapshot {
    workbook_with_sheets(
        vec![sheet(1, "Sheet 1"), sheet(2, "Middle"), sheet(3, "Sheet3")],
        names,
    )
}

fn range(value: &str) -> CellRange {
    let (start, end) = value
        .split_once(':')
        .map_or((value, value), |(start, end)| (start, end));
    CellRange::new(
        CellAddress::from_a1(start).expect("test range start is valid"),
        CellAddress::from_a1(end).expect("test range end is valid"),
    )
    .expect("test range is ordered")
}

fn assert_rectangular(
    analysis: DefinedNameAnalysis,
    expected_sheet: SheetId,
    expected_range: &str,
) {
    assert_eq!(
        analysis,
        DefinedNameAnalysis::Rectangular {
            sheet_id: expected_sheet,
            range: range(expected_range),
        }
    );
}

#[test]
fn lookup_is_case_insensitive_and_preserves_definition_scope() {
    let first = sheet_id(1);
    let workbook = workbook(vec![
        defined_name("Target", DefinedNameScope::Workbook, "'Sheet 1'!A1"),
        defined_name("Target", DefinedNameScope::Sheet(first), "'Sheet 1'!B2"),
        defined_name("WorkbookAlias", DefinedNameScope::Workbook, "Target"),
        defined_name("LocalAlias", DefinedNameScope::Sheet(first), "Target"),
        defined_name("WorkbookOnly", DefinedNameScope::Workbook, "Sheet3!D4"),
        defined_name(
            "QualifiedLocal",
            DefinedNameScope::Workbook,
            "'Sheet 1'!Target",
        ),
        defined_name(
            "QualifiedFallback",
            DefinedNameScope::Workbook,
            "Middle!WorkbookOnly",
        ),
    ]);

    assert_rectangular(
        analyze_defined_name(&workbook, "tArGeT", Some(first)).expect("analysis succeeds"),
        first,
        "B2",
    );
    assert_rectangular(
        analyze_defined_name(&workbook, "TARGET", None).expect("analysis succeeds"),
        first,
        "A1",
    );
    assert_rectangular(
        analyze_defined_name(&workbook, "WorkbookAlias", Some(first)).expect("analysis succeeds"),
        first,
        "A1",
    );
    assert_rectangular(
        analyze_defined_name(&workbook, "LocalAlias", Some(first)).expect("analysis succeeds"),
        first,
        "B2",
    );
    assert_rectangular(
        analyze_defined_name(&workbook, "QualifiedLocal", None).expect("analysis succeeds"),
        first,
        "B2",
    );
    assert_rectangular(
        analyze_defined_name(&workbook, "QualifiedFallback", None).expect("analysis succeeds"),
        sheet_id(3),
        "D4",
    );
    assert_eq!(
        analyze_defined_name(&workbook, "LocalAlias", None).expect("analysis succeeds"),
        DefinedNameAnalysis::NotFound,
    );
    assert_eq!(
        analyze_defined_name(&workbook, "Missing", Some(first)).expect("analysis succeeds"),
        DefinedNameAnalysis::NotFound,
    );
}

#[test]
fn unknown_current_sheet_is_an_execution_error() {
    let workbook = workbook(Vec::new());
    let error = analyze_defined_name(&workbook, "Missing", Some(sheet_id(99)))
        .expect_err("unknown caller input is not a semantic result");
    assert_eq!(
        error.kind(),
        DefinedNameAnalysisErrorKind::UnknownCurrentSheet
    );
    assert_eq!(error.detail(), Some("99"));
    assert_eq!(error.limit(), None);
}

#[test]
fn static_geometry_preserves_three_d_and_ordered_multi_area_identity() {
    let workbook = workbook(vec![
        defined_name("Span", DefinedNameScope::Workbook, "'Sheet 1':Sheet3!A1:B2"),
        defined_name(
            "ReversedSpan",
            DefinedNameScope::Workbook,
            "Sheet3:'Sheet 1'!C3",
        ),
        defined_name(
            "ExplicitSingleSpan",
            DefinedNameScope::Workbook,
            "Middle:Middle!D4",
        ),
        defined_name(
            "Areas",
            DefinedNameScope::Workbook,
            "('Sheet 1'!A1,Sheet3!B2,'Sheet 1':Sheet3!C3,'Sheet 1'!A1)",
        ),
    ]);

    assert_eq!(
        analyze_defined_name(&workbook, "Span", None).expect("analysis succeeds"),
        DefinedNameAnalysis::ThreeDimensional {
            sheet_span: DefinedNameSheetSpan::new(sheet_id(1), sheet_id(3)),
            range: range("A1:B2"),
        }
    );
    assert_eq!(
        analyze_defined_name(&workbook, "ReversedSpan", None).expect("analysis succeeds"),
        DefinedNameAnalysis::ThreeDimensional {
            sheet_span: DefinedNameSheetSpan::new(sheet_id(1), sheet_id(3)),
            range: range("C3"),
        }
    );
    assert_eq!(
        analyze_defined_name(&workbook, "ExplicitSingleSpan", None).expect("analysis succeeds"),
        DefinedNameAnalysis::ThreeDimensional {
            sheet_span: DefinedNameSheetSpan::new(sheet_id(2), sheet_id(2)),
            range: range("D4"),
        }
    );
    assert_eq!(
        analyze_defined_name(&workbook, "Areas", None).expect("analysis succeeds"),
        DefinedNameAnalysis::NonRectangular {
            areas: vec![
                DefinedNameReferenceArea::Rectangular {
                    sheet_id: sheet_id(1),
                    range: range("A1"),
                },
                DefinedNameReferenceArea::Rectangular {
                    sheet_id: sheet_id(3),
                    range: range("B2"),
                },
                DefinedNameReferenceArea::ThreeDimensional {
                    sheet_span: DefinedNameSheetSpan::new(sheet_id(1), sheet_id(3)),
                    range: range("C3"),
                },
                DefinedNameReferenceArea::Rectangular {
                    sheet_id: sheet_id(1),
                    range: range("A1"),
                },
            ],
        }
    );
}

#[test]
fn structured_reference_can_resolve_to_a_valid_empty_area() {
    let mut first = sheet(1, "Sheet 1");
    first.set_tables(vec![
        Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("EmptyData").expect("table name"),
            TableName::new("EmptyData").expect("display name"),
            range("A1"),
            1,
            0,
            vec![TableColumn::new(1, "Value", None).expect("table column")],
        )
        .expect("empty-data table"),
    ]);
    let workbook = workbook_with_sheets(
        vec![first, sheet(2, "Middle"), sheet(3, "Sheet3")],
        vec![defined_name(
            "EmptyBand",
            DefinedNameScope::Workbook,
            "EmptyData[#Data]",
        )],
    );

    assert_eq!(
        analyze_defined_name(&workbook, "EmptyBand", None).expect("analysis succeeds"),
        DefinedNameAnalysis::EmptyReference,
    );
}

#[test]
fn dynamic_classification_applies_only_to_terminal_reference_shapes() {
    let workbook = workbook(vec![
        defined_name(
            "OffsetRef",
            DefinedNameScope::Workbook,
            "OFFSET('Sheet 1'!A1,1,0)",
        ),
        defined_name("OffsetAlias", DefinedNameScope::Workbook, "OffsetRef"),
        defined_name(
            "IndirectRef",
            DefinedNameScope::Workbook,
            r#"INDIRECT("'Sheet 1'!A1")"#,
        ),
        defined_name("SpillRef", DefinedNameScope::Workbook, "'Sheet 1'!A1#"),
        defined_name(
            "MixedDynamic",
            DefinedNameScope::Workbook,
            "(OFFSET('Sheet 1'!A1,1,0),INDIRECT(\"'Sheet 1'!B1\"))",
        ),
        defined_name(
            "MixedIntersection",
            DefinedNameScope::Workbook,
            "OFFSET('Sheet 1'!A1,1,0) OFFSET('Sheet 1'!B1,1,0)",
        ),
        defined_name(
            "MixedRange",
            DefinedNameScope::Workbook,
            "OFFSET('Sheet 1'!A1,1,0):INDIRECT(\"'Sheet 1'!B1\")",
        ),
        defined_name(
            "OffsetValue",
            DefinedNameScope::Workbook,
            "OFFSET('Sheet 1'!A1,1,0)+1",
        ),
    ]);

    for (name, kind, terminal_formula) in [
        (
            "OffsetRef",
            DefinedNameDynamicKind::Offset,
            "OFFSET('Sheet 1'!A1,1,0)",
        ),
        (
            "OffsetAlias",
            DefinedNameDynamicKind::Offset,
            "OFFSET('Sheet 1'!A1,1,0)",
        ),
        (
            "IndirectRef",
            DefinedNameDynamicKind::Indirect,
            r#"INDIRECT("'Sheet 1'!A1")"#,
        ),
        ("SpillRef", DefinedNameDynamicKind::Spill, "'Sheet 1'!A1#"),
        (
            "MixedDynamic",
            DefinedNameDynamicKind::Mixed,
            "(OFFSET('Sheet 1'!A1,1,0),INDIRECT(\"'Sheet 1'!B1\"))",
        ),
        (
            "MixedIntersection",
            DefinedNameDynamicKind::Mixed,
            "OFFSET('Sheet 1'!A1,1,0) OFFSET('Sheet 1'!B1,1,0)",
        ),
        (
            "MixedRange",
            DefinedNameDynamicKind::Mixed,
            "OFFSET('Sheet 1'!A1,1,0):INDIRECT(\"'Sheet 1'!B1\")",
        ),
    ] {
        assert_eq!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::DynamicFormula {
                kind,
                formula: formula(terminal_formula),
            },
            "{name}",
        );
    }
    assert!(matches!(
        analyze_defined_name(&workbook, "OffsetValue", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::NonReferenceExpression,
            ..
        }
    ));
}

#[test]
fn tableless_structured_references_require_a_current_cell_context() {
    let workbook = workbook(vec![
        defined_name("ImplicitHeaders", DefinedNameScope::Workbook, "[#Headers]"),
        defined_name("ImplicitData", DefinedNameScope::Workbook, "[#Data]"),
        defined_name("ImplicitColumn", DefinedNameScope::Workbook, "[Value]"),
    ]);

    for name in ["ImplicitHeaders", "ImplicitData", "ImplicitColumn"] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, Some(sheet_id(1))).expect("analysis succeeds"),
            DefinedNameAnalysis::Unsupported {
                reason: DefinedNameUnsupportedReason::ContextDependent,
                ..
            }
        ));
    }
}

#[test]
fn reference_operators_share_evaluator_geometry_rules() {
    let workbook = workbook(vec![
        defined_name(
            "SpanIntersection",
            DefinedNameScope::Workbook,
            "'Sheet 1':Sheet3!A1 Middle!A1",
        ),
        defined_name(
            "UnionRange",
            DefinedNameScope::Workbook,
            "('Sheet 1'!A1,'Sheet 1'!B2):'Sheet 1'!C3",
        ),
    ]);

    assert_eq!(
        analyze_defined_name(&workbook, "SpanIntersection", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Invalid {
            reason: DefinedNameInvalidReason::InvalidReference,
            detail: Some("#VALUE!".into()),
        }
    );
    assert_rectangular(
        analyze_defined_name(&workbook, "UnionRange", None).expect("analysis succeeds"),
        sheet_id(1),
        "A1:C3",
    );
}

#[test]
fn constants_and_valid_non_reference_formulas_remain_distinct() {
    let workbook = workbook(vec![
        defined_name("Number", DefinedNameScope::Workbook, "42"),
        defined_name("Expression", DefinedNameScope::Workbook, "1+2*3"),
        defined_name("Array", DefinedNameScope::Workbook, "{1,2;3,4}"),
        defined_name("Function", DefinedNameScope::Workbook, "SUM(1,2)"),
        defined_name(
            "Callable",
            DefinedNameScope::Workbook,
            "LAMBDA(value,value+1)",
        ),
        defined_name(
            "Context",
            DefinedNameScope::Workbook,
            "INDEX('Sheet 1'!A1:B2,1,1)",
        ),
    ]);

    for name in ["Number", "Expression", "Array"] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::Constant { .. }
        ));
    }
    for name in ["Function", "Callable"] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::Unsupported {
                reason: DefinedNameUnsupportedReason::NonReferenceExpression,
                ..
            }
        ));
    }
    assert!(matches!(
        analyze_defined_name(&workbook, "Context", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::ContextDependent,
            ..
        }
    ));
}

#[test]
fn builtin_callable_analysis_uses_the_same_shadow_resolution_as_evaluation() {
    let workbook = workbook(vec![
        defined_name("SUM", DefinedNameScope::Workbook, "'Sheet 1'!A1"),
        defined_name("TypedAlias", DefinedNameScope::Workbook, "_xleta.SUM"),
        defined_name("PlainAlias", DefinedNameScope::Workbook, "SUM"),
        defined_name("Unshadowed", DefinedNameScope::Workbook, "_xleta.AVERAGE"),
        defined_name("COUNT", DefinedNameScope::Workbook, "COUNT"),
        defined_name("CyclicTyped", DefinedNameScope::Workbook, "_xleta.COUNT"),
        defined_name(
            "LocalShadow",
            DefinedNameScope::Workbook,
            "LAMBDA(SUM,_xleta.SUM)",
        ),
    ]);

    for name in ["TypedAlias", "PlainAlias"] {
        assert_rectangular(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            sheet_id(1),
            "A1",
        );
    }
    assert!(matches!(
        analyze_defined_name(&workbook, "Unshadowed", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::NonReferenceExpression,
            ..
        }
    ));
    assert!(matches!(
        analyze_defined_name(&workbook, "CyclicTyped", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Invalid {
            reason: DefinedNameInvalidReason::CircularReference,
            ..
        }
    ));
    assert!(matches!(
        analyze_defined_name(&workbook, "LocalShadow", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::NonReferenceExpression,
            ..
        }
    ));
}

#[test]
fn invalid_typed_builtin_invocations_do_not_validate_unreachable_arguments() {
    let arguments = std::iter::once("NoSuchName")
        .chain(std::iter::repeat_n("1", 255))
        .collect::<Vec<_>>()
        .join(",");
    let workbook = workbook(vec![
        defined_name(
            "Direct",
            DefinedNameScope::Workbook,
            &format!("_xleta.SUM({arguments})"),
        ),
        defined_name(
            "Parenthesized",
            DefinedNameScope::Workbook,
            &format!("(_xleta.SUM)({arguments})"),
        ),
        defined_name("X", DefinedNameScope::Workbook, "42"),
        defined_name("SUM", DefinedNameScope::Workbook, "X"),
        defined_name(
            "DefinedNonCallable",
            DefinedNameScope::Workbook,
            "_xleta.SUM(NoSuchName)",
        ),
        defined_name(
            "LocalNonCallable",
            DefinedNameScope::Workbook,
            "LET(SUM,2,_xleta.SUM(NoSuchName))",
        ),
        defined_name(
            "PlainNonCallable",
            DefinedNameScope::Workbook,
            "SUM(NoSuchName)",
        ),
        defined_name("F", DefinedNameScope::Workbook, "LAMBDA(value,value)"),
        defined_name("Alias", DefinedNameScope::Workbook, "F"),
        defined_name(
            "AliasInvalidArity",
            DefinedNameScope::Workbook,
            "Alias(NoSuchName,1)",
        ),
    ]);

    for name in [
        "Direct",
        "Parenthesized",
        "DefinedNonCallable",
        "LocalNonCallable",
        "PlainNonCallable",
        "AliasInvalidArity",
    ] {
        let analysis = analyze_defined_name(&workbook, name, None).expect("analysis succeeds");
        assert!(
            matches!(analysis, DefinedNameAnalysis::Unsupported { .. }),
            "{name}: {analysis:?}"
        );
    }
}

#[test]
fn invoked_callable_cycles_are_dead_targets_but_standalone_aliases_remain_cycles() {
    let workbook = workbook(vec![
        defined_name("Loop", DefinedNameScope::Workbook, "Loop"),
        defined_name(
            "OrdinaryInvokedCycle",
            DefinedNameScope::Workbook,
            "Loop(1)",
        ),
        defined_name("SUM", DefinedNameScope::Workbook, "SUM"),
        defined_name(
            "TypedInvokedCycle",
            DefinedNameScope::Workbook,
            "_xleta.SUM(1)",
        ),
    ]);

    for name in ["OrdinaryInvokedCycle", "TypedInvokedCycle"] {
        let analysis = analyze_defined_name(&workbook, name, None).expect("analysis succeeds");
        assert!(
            matches!(
                analysis,
                DefinedNameAnalysis::Unsupported {
                    reason: DefinedNameUnsupportedReason::NonReferenceExpression,
                    ..
                }
            ),
            "{name}: {analysis:?}"
        );
    }
    for name in ["Loop", "SUM"] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::Invalid {
                reason: DefinedNameInvalidReason::CircularReference,
                ..
            }
        ));
    }
}

#[test]
fn invalid_static_references_inside_general_expressions_are_not_unsupported() {
    let workbook = workbook(vec![
        defined_name(
            "MissingSheetInCall",
            DefinedNameScope::Workbook,
            "SUM(NoSuchSheet!A1)",
        ),
        defined_name(
            "MissingTableInCall",
            DefinedNameScope::Workbook,
            "SUM(NoSuchTable[Value])",
        ),
        defined_name("DeletedRoot", DefinedNameScope::Workbook, "#REF!"),
        defined_name("DeletedAlias", DefinedNameScope::Workbook, "DeletedRoot"),
        defined_name("DeletedRangeStart", DefinedNameScope::Workbook, "#REF!:A1"),
        defined_name("DeletedRangeEnd", DefinedNameScope::Workbook, "A1:#REF!"),
        defined_name("DeletedInCall", DefinedNameScope::Workbook, "SUM(#REF!)"),
    ]);

    for name in [
        "MissingSheetInCall",
        "MissingTableInCall",
        "DeletedRoot",
        "DeletedAlias",
        "DeletedRangeStart",
        "DeletedRangeEnd",
        "DeletedInCall",
    ] {
        let analysis = analyze_defined_name(&workbook, name, None).expect("analysis succeeds");
        assert!(
            matches!(
                analysis,
                DefinedNameAnalysis::Invalid {
                    reason: DefinedNameInvalidReason::InvalidReference,
                    ..
                }
            ),
            "{name}: {analysis:?}"
        );
    }
}

#[test]
fn defined_callable_names_shadow_builtin_dynamic_functions() {
    let workbook = workbook(vec![
        defined_name("OFFSET", DefinedNameScope::Workbook, "LAMBDA(value,value)"),
        defined_name(
            "UsesOffset",
            DefinedNameScope::Workbook,
            "OFFSET('Sheet 1'!A1)",
        ),
    ]);

    assert!(matches!(
        analyze_defined_name(&workbook, "UsesOffset", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::NonReferenceExpression,
            ..
        }
    ));
}

#[test]
fn external_references_have_typed_target_details() {
    let workbook = workbook(vec![
        defined_name(
            "ExternalCell",
            DefinedNameScope::Workbook,
            "[Book.xlsx]Sheet1!A1",
        ),
        defined_name("ExternalName", DefinedNameScope::Workbook, "[1]!RemoteName"),
        defined_name(
            "ExternalTable",
            DefinedNameScope::Workbook,
            "[2]!RemoteTable[Amount]",
        ),
        defined_name(
            "ExternalPath",
            DefinedNameScope::Workbook,
            "'C:\\Dir]\\[Book.xlsx]Sheet 1'!B2",
        ),
    ]);

    for (name, locator, expected_workbook, sheet, target, target_text) in [
        (
            "ExternalCell",
            None,
            "Book.xlsx",
            Some("Sheet1"),
            DefinedNameExternalTargetKind::Reference,
            "A1",
        ),
        (
            "ExternalName",
            None,
            "1",
            None,
            DefinedNameExternalTargetKind::DefinedName,
            "RemoteName",
        ),
        (
            "ExternalTable",
            None,
            "2",
            None,
            DefinedNameExternalTargetKind::StructuredReference,
            "RemoteTable[Amount]",
        ),
        (
            "ExternalPath",
            Some("C:\\Dir]\\"),
            "Book.xlsx",
            Some("Sheet 1"),
            DefinedNameExternalTargetKind::Reference,
            "B2",
        ),
    ] {
        let DefinedNameAnalysis::ExternalReference { detail } =
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds")
        else {
            panic!("{name} should resolve as an external reference");
        };
        assert_eq!(detail.locator(), locator);
        assert_eq!(detail.workbook(), expected_workbook);
        assert_eq!(detail.sheet(), sheet);
        assert_eq!(detail.target(), target);
        assert_eq!(detail.target_text(), target_text);
    }
}

#[test]
fn semantic_invalidity_distinguishes_parse_name_reference_and_cycles() {
    let workbook = workbook(vec![
        defined_name("BrokenSyntax", DefinedNameScope::Workbook, "SUM("),
        defined_name("MissingAlias", DefinedNameScope::Workbook, "DoesNotExist"),
        defined_name("MissingSheet", DefinedNameScope::Workbook, "NoSuchSheet!A1"),
        defined_name("CycleA", DefinedNameScope::Workbook, "CycleB"),
        defined_name("CycleB", DefinedNameScope::Workbook, "CycleA"),
        defined_name(
            "RecursiveLambda",
            DefinedNameScope::Workbook,
            "LAMBDA(n,IF(n=0,0,RecursiveLambda(n-1)))",
        ),
    ]);

    for (name, reason) in [
        ("BrokenSyntax", DefinedNameInvalidReason::ParseError),
        ("MissingAlias", DefinedNameInvalidReason::UnresolvedName),
        ("MissingSheet", DefinedNameInvalidReason::InvalidReference),
        ("CycleA", DefinedNameInvalidReason::CircularReference),
    ] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::Invalid {
                reason: actual,
                ..
            } if actual == reason
        ));
    }
    assert!(matches!(
        analyze_defined_name(&workbook, "RecursiveLambda", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::NonReferenceExpression,
            ..
        }
    ));
}

#[test]
fn callable_bodies_are_bounded_and_statically_validated() {
    let workbook = workbook(vec![
        defined_name(
            "BadCallable",
            DefinedNameScope::Workbook,
            "LAMBDA(value,#REF!)",
        ),
        defined_name(
            "BadCallableUse",
            DefinedNameScope::Workbook,
            "BadCallable(1)",
        ),
        defined_name(
            "BadImmediateCall",
            DefinedNameScope::Workbook,
            "LAMBDA(value,#REF!)(1)",
        ),
        defined_name(
            "BadMap",
            DefinedNameScope::Workbook,
            "MAP('Sheet 1'!A1,LAMBDA(value,NoSuchSheet!A1))",
        ),
        defined_name(
            "RecursiveCallable",
            DefinedNameScope::Workbook,
            "LAMBDA(value,IF(value=0,0,RecursiveCallable(value-1)))",
        ),
        defined_name(
            "SelfCallableValue",
            DefinedNameScope::Workbook,
            "LAMBDA(value,SelfCallableValue)",
        ),
        defined_name(
            "MutualCallableA",
            DefinedNameScope::Workbook,
            "LAMBDA(value,MutualCallableB)",
        ),
        defined_name(
            "MutualCallableB",
            DefinedNameScope::Workbook,
            "LAMBDA(value,MutualCallableA)",
        ),
        defined_name(
            "RecursiveLocal",
            DefinedNameScope::Sheet(sheet_id(1)),
            "LAMBDA(value,IF(value=0,0,'Sheet 1'!RecursiveLocal(value-1)))",
        ),
        defined_name(
            "UseQualifiedCallable",
            DefinedNameScope::Workbook,
            "'Sheet 1'!RecursiveLocal(1)",
        ),
    ]);

    for name in [
        "BadCallable",
        "BadCallableUse",
        "BadImmediateCall",
        "BadMap",
    ] {
        let analysis = analyze_defined_name(&workbook, name, None).expect("analysis succeeds");
        assert!(
            matches!(
                analysis,
                DefinedNameAnalysis::Invalid {
                    reason: DefinedNameInvalidReason::InvalidReference,
                    ..
                }
            ),
            "{name}: {analysis:?}"
        );
    }
    for name in [
        "RecursiveCallable",
        "SelfCallableValue",
        "MutualCallableA",
        "MutualCallableB",
        "UseQualifiedCallable",
    ] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::Unsupported {
                reason: DefinedNameUnsupportedReason::NonReferenceExpression,
                ..
            }
        ));
    }

    let bounded = DefinedNameAnalysisOptions::default()
        .with_max_scan_nodes(2)
        .expect("non-zero limit");
    let error = analyze_defined_name_with_options(&workbook, "RecursiveCallable", None, bounded)
        .expect_err("lambda parameters and body must consume the scan budget");
    assert_eq!(error.limit(), Some(DefinedNameAnalysisLimitKind::ScanNodes));
}

#[test]
fn unqualified_and_current_row_references_report_context_dependency() {
    let mut first = sheet(1, "Sheet 1");
    first.set_tables(vec![
        Table::new(
            TableId::new(1).expect("table ID"),
            TableName::new("KnownTable").expect("table name"),
            TableName::new("KnownTable").expect("display name"),
            range("A1:A2"),
            1,
            0,
            vec![TableColumn::new(1, "Value", None).expect("table column")],
        )
        .expect("known table"),
    ]);
    let workbook = workbook_with_sheets(
        vec![first, sheet(2, "Middle"), sheet(3, "Sheet3")],
        vec![
            defined_name("Unqualified", DefinedNameScope::Workbook, "A1"),
            defined_name(
                "ValidCurrentRow",
                DefinedNameScope::Workbook,
                "KnownTable[@Value]",
            ),
            defined_name(
                "MissingCurrentRowTable",
                DefinedNameScope::Workbook,
                "MissingTable[@Value]",
            ),
            defined_name(
                "MissingCurrentRowColumn",
                DefinedNameScope::Workbook,
                "KnownTable[@Missing]",
            ),
            defined_name(
                "MissingCombinedColumn",
                DefinedNameScope::Workbook,
                "KnownTable[[#This Row],[Missing]]",
            ),
        ],
    );

    assert!(matches!(
        analyze_defined_name(&workbook, "Unqualified", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::ContextDependent,
            ..
        }
    ));
    assert_rectangular(
        analyze_defined_name(&workbook, "Unqualified", Some(sheet_id(2)))
            .expect("analysis succeeds"),
        sheet_id(2),
        "A1",
    );
    assert!(matches!(
        analyze_defined_name(&workbook, "ValidCurrentRow", None).expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::ContextDependent,
            ..
        }
    ));
    for name in [
        "MissingCurrentRowTable",
        "MissingCurrentRowColumn",
        "MissingCombinedColumn",
    ] {
        assert!(matches!(
            analyze_defined_name(&workbook, name, None).expect("analysis succeeds"),
            DefinedNameAnalysis::Invalid {
                reason: DefinedNameInvalidReason::InvalidReference,
                ..
            }
        ));
    }
}

#[test]
fn caller_limits_are_enforced_at_exact_boundaries() {
    let workbook = workbook(vec![
        defined_name("ChainA", DefinedNameScope::Workbook, "ChainB"),
        defined_name("ChainB", DefinedNameScope::Workbook, "ChainC"),
        defined_name("ChainC", DefinedNameScope::Workbook, "'Sheet 1'!A1"),
        defined_name(
            "TwoAreas",
            DefinedNameScope::Workbook,
            "('Sheet 1'!A1,'Sheet 1'!B1)",
        ),
        defined_name("Short", DefinedNameScope::Workbook, "A1"),
    ]);

    let depth_three = DefinedNameAnalysisOptions::default()
        .with_max_name_chain_depth(3)
        .expect("non-zero limit");
    assert!(analyze_defined_name_with_options(&workbook, "ChainA", None, depth_three).is_ok());
    let depth_two = DefinedNameAnalysisOptions::default()
        .with_max_name_chain_depth(2)
        .expect("non-zero limit");
    let error = analyze_defined_name_with_options(&workbook, "ChainA", None, depth_two)
        .expect_err("N+1 chain must exceed the limit");
    assert_eq!(
        error.limit(),
        Some(DefinedNameAnalysisLimitKind::NameChainDepth)
    );

    let scan_eight = DefinedNameAnalysisOptions::default()
        .with_max_scan_nodes(8)
        .expect("non-zero limit");
    assert!(analyze_defined_name_with_options(&workbook, "TwoAreas", None, scan_eight).is_ok());
    let scan_seven = DefinedNameAnalysisOptions::default()
        .with_max_scan_nodes(7)
        .expect("non-zero limit");
    let error = analyze_defined_name_with_options(&workbook, "TwoAreas", None, scan_seven)
        .expect_err("N+1 AST nodes must exceed the scan limit");
    assert_eq!(error.limit(), Some(DefinedNameAnalysisLimitKind::ScanNodes));

    let areas_two = CalculationLimits::default()
        .with_max_reference_areas(2)
        .expect("non-zero limit");
    let options =
        DefinedNameAnalysisOptions::new(CalculationOptions::default().with_limits(areas_two));
    assert!(analyze_defined_name_with_options(&workbook, "TwoAreas", None, options).is_ok());
    let areas_one = CalculationLimits::default()
        .with_max_reference_areas(1)
        .expect("non-zero limit");
    let options =
        DefinedNameAnalysisOptions::new(CalculationOptions::default().with_limits(areas_one));
    let error = analyze_defined_name_with_options(&workbook, "TwoAreas", None, options)
        .expect_err("N+1 areas must exceed the limit");
    assert_eq!(
        error.limit(),
        Some(DefinedNameAnalysisLimitKind::ReferenceAreas)
    );

    let source_two = CalculationLimits::default()
        .with_max_formula_source_bytes(2)
        .expect("non-zero limit");
    let options =
        DefinedNameAnalysisOptions::new(CalculationOptions::default().with_limits(source_two));
    assert!(
        analyze_defined_name_with_options(&workbook, "Short", Some(sheet_id(1)), options).is_ok()
    );
    let source_one = CalculationLimits::default()
        .with_max_formula_source_bytes(1)
        .expect("non-zero limit");
    let options =
        DefinedNameAnalysisOptions::new(CalculationOptions::default().with_limits(source_one));
    let error = analyze_defined_name_with_options(&workbook, "Short", Some(sheet_id(1)), options)
        .expect_err("N+1 bytes must exceed the parser limit");
    assert_eq!(
        error.limit(),
        Some(DefinedNameAnalysisLimitKind::FormulaSourceBytes)
    );
}

#[test]
fn let_binding_limit_precedes_defined_name_body_validation() {
    let workbook = workbook(vec![defined_name(
        "OverLimitLet",
        DefinedNameScope::Workbook,
        "LET(first,1,second,2,NoSuchName)",
    )]);
    let limits = CalculationLimits::default()
        .with_max_let_bindings(1)
        .expect("positive LET binding limit");
    let options =
        DefinedNameAnalysisOptions::new(CalculationOptions::default().with_limits(limits));

    assert!(matches!(
        analyze_defined_name_with_options(&workbook, "OverLimitLet", None, options)
            .expect("analysis succeeds"),
        DefinedNameAnalysis::Unsupported {
            reason: DefinedNameUnsupportedReason::ContextDependent,
            ..
        }
    ));
}

#[test]
fn cancellation_is_checked_before_and_during_reachable_scans() {
    let workbook = workbook(vec![defined_name(
        "Areas",
        DefinedNameScope::Workbook,
        "('Sheet 1'!A1,'Sheet 1'!B1,'Sheet 1'!C1,'Sheet 1'!D1)",
    )]);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = analyze_defined_name_cancellable(
        &workbook,
        "Areas",
        None,
        DefinedNameAnalysisOptions::default(),
        &cancellation,
    )
    .expect_err("pre-cancelled analysis must stop");
    assert_eq!(error.kind(), DefinedNameAnalysisErrorKind::Cancelled);

    let polls = Cell::new(0_u32);
    let error = analyzer::analyze(
        &workbook,
        "Areas",
        None,
        DefinedNameAnalysisOptions::default(),
        &|| {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 5
        },
    )
    .expect_err("cooperative cancellation must interrupt a scan");
    assert_eq!(error.kind(), DefinedNameAnalysisErrorKind::Cancelled);
    assert!(polls.get() >= 5);
}

#[test]
fn repeated_alias_branches_reuse_context_specific_classification() {
    let mut names = vec![defined_name(
        "Level0",
        DefinedNameScope::Workbook,
        "OFFSET('Sheet 1'!A1,1,0)",
    )];
    for level in 1..=20 {
        names.push(defined_name(
            &format!("Level{level}"),
            DefinedNameScope::Workbook,
            &format!("(Level{},Level{})", level - 1, level - 1),
        ));
    }
    let workbook = workbook(names);
    let polls = Cell::new(0_u32);
    let result = analyzer::analyze(
        &workbook,
        "Level20",
        None,
        DefinedNameAnalysisOptions::default(),
        &|| {
            polls.set(polls.get() + 1);
            false
        },
    )
    .expect("bounded repeated aliases succeed");

    assert_eq!(
        result,
        DefinedNameAnalysis::DynamicFormula {
            kind: DefinedNameDynamicKind::Mixed,
            formula: formula("(Level19,Level19)"),
        }
    );
    assert!(
        polls.get() < 800,
        "classification should stay linear, observed {} cancellation polls",
        polls.get(),
    );
}

#[test]
fn deep_name_and_formula_composition_uses_bounded_explicit_stacks() {
    let depth = 128;
    let nesting = 64;
    let mut names = Vec::new();
    for index in 0..depth {
        let target = if index + 1 == depth {
            "'Sheet 1'!A1".to_owned()
        } else {
            format!("Chain{}", index + 1)
        };
        names.push(defined_name(
            &format!("Chain{index}"),
            DefinedNameScope::Workbook,
            &format!("{}{}{}", "(".repeat(nesting), target, ")".repeat(nesting)),
        ));
    }
    let workbook = workbook(names);

    assert_rectangular(
        analyze_defined_name(&workbook, "Chain0", None).expect("bounded analysis succeeds"),
        sheet_id(1),
        "A1",
    );
}

#[test]
fn context_specific_validation_consumes_the_cumulative_scan_budget() {
    let sheets = (1..=24)
        .map(|index| sheet(index, &format!("S{index}")))
        .collect::<Vec<_>>();
    let leaf = (0..40).fold("1".to_owned(), |expr, _| format!("({expr}+1)"));
    let root = (1..=24)
        .map(|index| format!("S{index}!Leaf"))
        .collect::<Vec<_>>()
        .join(",");
    let workbook = workbook_with_sheets(
        sheets,
        vec![
            defined_name("Leaf", DefinedNameScope::Workbook, &leaf),
            defined_name("Root", DefinedNameScope::Workbook, &format!("({root})")),
        ],
    );
    let options = DefinedNameAnalysisOptions::default()
        .with_max_scan_nodes(500)
        .expect("non-zero limit");

    let error = analyze_defined_name_with_options(&workbook, "Root", None, options)
        .expect_err("every context-specific AST traversal must consume budget");
    assert_eq!(error.limit(), Some(DefinedNameAnalysisLimitKind::ScanNodes));
}
