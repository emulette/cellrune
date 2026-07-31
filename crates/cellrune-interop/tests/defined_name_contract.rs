use cellrune_interop::{
    CalculationOptionsDto, DefinedNameDynamicKindDto, DefinedNameInspectionRequestDto,
    DefinedNameInspectionResultDto, DefinedNameInvalidReasonDto, DefinedNameReferenceAreaDto,
    DefinedNameUnsupportedReasonDto, EditBatchDto, WorkbookChangeDto, WorkbookSession,
    WriteOptionsDto,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u32,
    sheets: Vec<String>,
    defined_names: Vec<DefinedNameEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinedNameEntry {
    name: String,
    scope_sheet: Option<String>,
    formula: String,
    hidden: bool,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../binding-contract/defined-name-v1.json"
    ))
    .expect("defined-name binding corpus must remain valid")
}

fn build_session(corpus: &Corpus) -> WorkbookSession {
    let mut session = WorkbookSession::create();
    for sheet in &corpus.sheets {
        session
            .add_sheet(sheet)
            .expect("corpus sheet must be valid");
    }
    let revision = session.summary().semantic_revision;
    session
        .apply_changes(
            revision,
            EditBatchDto {
                changes: corpus
                    .defined_names
                    .iter()
                    .map(|entry| WorkbookChangeDto::SetDefinedName {
                        name: entry.name.clone(),
                        scope_sheet: entry.scope_sheet.clone(),
                        formula: entry.formula.clone(),
                        hidden: entry.hidden,
                    })
                    .collect(),
            },
        )
        .expect("corpus names must install atomically");
    session
}

fn inspect(
    session: &WorkbookSession,
    name: &str,
    current_sheet: Option<&str>,
) -> DefinedNameInspectionResultDto {
    session
        .inspect_defined_name(&DefinedNameInspectionRequestDto {
            name: name.to_owned(),
            current_sheet: current_sheet.map(str::to_owned),
        })
        .expect("corpus inspection must succeed")
        .result
}

#[test]
fn shared_binding_corpus_preserves_scope_geometry_and_classification() {
    let corpus = corpus();
    assert_eq!(corpus.schema_version, 1);
    let session = build_session(&corpus);

    assert!(matches!(
        inspect(&session, "WorkbookAlias", Some("Sheet1")),
        DefinedNameInspectionResultDto::Rectangular {
            sheet_id: 1,
            ref sheet_name,
            ref range,
        } if sheet_name == "Sheet1" && range == "A1:A1"
    ));
    assert!(matches!(
        inspect(&session, "LocalAlias", Some("Sheet1")),
        DefinedNameInspectionResultDto::Rectangular {
            sheet_id: 1,
            ref sheet_name,
            ref range,
        } if sheet_name == "Sheet1" && range == "B2:B2"
    ));
    assert!(matches!(
        inspect(&session, "QualifiedLocal", None),
        DefinedNameInspectionResultDto::Rectangular {
            sheet_id: 1,
            ref sheet_name,
            ref range,
        } if sheet_name == "Sheet1" && range == "B2:B2"
    ));
    assert!(matches!(
        inspect(&session, "ExplicitSingleSpan", None),
        DefinedNameInspectionResultDto::ThreeDimensional {
            ref sheet_span,
            ref range,
        } if sheet_span.start_sheet_id == 2
            && sheet_span.start_sheet_name == "Middle"
            && sheet_span.end_sheet_id == 2
            && sheet_span.end_sheet_name == "Middle"
            && range == "D4:D4"
    ));

    let DefinedNameInspectionResultDto::NonRectangular { areas } = inspect(&session, "Areas", None)
    else {
        panic!("Areas must preserve its ordered union");
    };
    assert_eq!(areas.len(), 4);
    assert!(matches!(
        &areas[2],
        DefinedNameReferenceAreaDto::ThreeDimensional { sheet_span, range }
            if sheet_span.start_sheet_id == 1
                && sheet_span.end_sheet_id == 3
                && range == "C3:C3"
    ));
    assert_eq!(areas[0], areas[3], "union duplicates must survive");
    assert!(matches!(
        inspect(&session, "Dynamic", None),
        DefinedNameInspectionResultDto::DynamicFormula {
            ref dynamic_kind,
            ref formula,
        } if *dynamic_kind == DefinedNameDynamicKindDto::Offset
            && formula == "=OFFSET(Sheet1!A1,1,0)"
    ));
    for (name, expected) in [
        ("IndirectDynamic", DefinedNameDynamicKindDto::Indirect),
        ("SpillDynamic", DefinedNameDynamicKindDto::Spill),
        ("MixedDynamic", DefinedNameDynamicKindDto::Mixed),
    ] {
        assert!(matches!(
            inspect(&session, name, None),
            DefinedNameInspectionResultDto::DynamicFormula { dynamic_kind, .. }
                if dynamic_kind == expected
        ));
    }
    assert!(matches!(
        inspect(&session, "ConstantValue", None),
        DefinedNameInspectionResultDto::Constant { .. }
    ));
    assert_eq!(
        inspect(&session, "ExternalValue", None),
        DefinedNameInspectionResultDto::ExternalReference {
            locator: None,
            workbook: "Book.xlsx".to_owned(),
            sheet: Some("Data".to_owned()),
            sheet_end: None,
            target_kind: cellrune_interop::DefinedNameExternalTargetKindDto::Reference,
            target_text: "A1".to_owned(),
        }
    );
    assert!(matches!(
        inspect(&session, "InvalidValue", None),
        DefinedNameInspectionResultDto::Invalid { ref reason, .. }
            if *reason == DefinedNameInvalidReasonDto::ParseError
    ));
    assert!(matches!(
        inspect(&session, "CallableValue", None),
        DefinedNameInspectionResultDto::Unsupported { ref reason, .. }
            if *reason == DefinedNameUnsupportedReasonDto::NonReferenceExpression
    ));
    assert_eq!(
        inspect(&session, "Missing", None),
        DefinedNameInspectionResultDto::NotFound
    );
}

#[test]
fn generated_xlsx_reopen_preserves_defined_name_inspection() {
    let corpus = corpus();
    let mut session = build_session(&corpus);
    session
        .calculate(CalculationOptionsDto::default())
        .expect("current revision must calculate before save");
    let (bytes, report) = session
        .save_bytes(WriteOptionsDto::default())
        .expect("generated workbook must save");
    assert!(report.complete);

    let reopened = WorkbookSession::open_bytes(&bytes).expect("generated workbook must reopen");
    for entry in &corpus.defined_names {
        assert_eq!(
            inspect(&reopened, &entry.name, entry.scope_sheet.as_deref()),
            inspect(&session, &entry.name, entry.scope_sheet.as_deref()),
            "{} must retain its inspection result after reopen",
            entry.name,
        );
    }
}

#[test]
fn request_dto_rejects_unknown_fields() {
    let error = serde_json::from_str::<DefinedNameInspectionRequestDto>(
        r#"{"name":"Areas","current_sheet":null,"future":true}"#,
    )
    .expect_err("unknown request fields must fail closed");
    assert!(error.to_string().contains("unknown field"));
}
