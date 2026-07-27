use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationHints, CalculationMode,
    CalculationOptions, CellAddress, CellContent, CellRange, CellValue, DefinedName,
    DefinedNameScope, ExcelError, FiniteNumber, FormulaMetadata, FormulaText, NumberFormat,
    NumberFormatKind, RecalculationWriteOptions, SavedResult, SheetName, SheetVisibility,
    WorkbookDraft, WriteOptions, XlsxWriteErrorCode, calculate_workbook, open_xlsx_document_bytes,
    write_recalculated_xlsx_bytes, write_xlsx_draft_bytes, write_xlsx_draft_path,
};

use super::recalculation_write_support as support;

#[test]
fn canonical_draft_is_deterministic_and_reopens_with_typed_results() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("A1").expect("A1"),
            CellValue::Number(FiniteNumber::new(2.0).expect("finite")),
        )
        .expect("set A1");
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("A2").expect("A2"),
            CellValue::Text("  Rune & 셀  ".to_owned()),
        )
        .expect("set A2");
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("B1").expect("B1"),
            FormulaText::from_xlsx("A1+3").expect("formula"),
        )
        .expect("set B1");
    draft
        .set_cell_number_format(
            sheet_id,
            CellAddress::from_a1("A1").expect("A1"),
            NumberFormat::custom(164, "0.00", NumberFormatKind::Number).expect("format"),
        )
        .expect("format A1");
    let hidden = draft
        .add_sheet(SheetName::new("Data Sheet").expect("sheet name"))
        .expect("add sheet");
    draft
        .set_cell_value(
            hidden,
            CellAddress::from_a1("A1").expect("A1"),
            CellValue::Logical(true),
        )
        .expect("set hidden sheet");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let first = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("canonical output");
    let second = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("deterministic output");
    assert_eq!(first.bytes(), second.bytes());
    assert!(first.report().is_complete());
    assert_eq!(first.report().provenance().input_hash(), None);
    assert!(
        !support::archive_parts(first.bytes()).contains_key("xl/metadata.xml"),
        "ordinary canonical workbooks must not carry dynamic-array metadata"
    );

    let reopened =
        open_xlsx_document_bytes(first.bytes(), cellrune::OpenOptions::default()).expect("reopen");
    let sheet = reopened.workbook().sheet_by_name("Sheet1").expect("Sheet1");
    let formula = sheet.cell_by_a1("B1").expect("valid B1").expect("B1 cell");
    let CellContent::Formula(formula) = formula.content() else {
        panic!("B1 must remain a formula");
    };
    assert_eq!(
        formula.saved_result(),
        &SavedResult::Present(CellValue::Number(FiniteNumber::new(5.0).expect("finite")))
    );
    assert_eq!(
        sheet
            .cell_by_a1("A2")
            .expect("valid A2")
            .expect("A2")
            .content(),
        &CellContent::Literal(CellValue::Text("  Rune & 셀  ".to_owned()))
    );

    let mut document_draft = WorkbookDraft::from_document(&reopened);
    document_draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("D1").expect("D1"),
            FormulaText::from_xlsx("SEQUENCE(1,2)").expect("dynamic formula"),
            None,
        )
        .expect("set document-backed dynamic formula");
    let document_calculation =
        calculate_workbook(document_draft.workbook(), CalculationOptions::default());
    let error = write_xlsx_draft_bytes(
        &document_draft,
        &document_calculation,
        RecalculationWriteOptions::default(),
    )
    .expect_err("document-backed dynamic metadata merge must fail closed");
    assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
}

#[test]
fn dynamic_spills_are_calculated_written_and_reopened_with_metadata() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let anchor = CellAddress::from_a1("A1").expect("A1");
    let range = CellRange::new(anchor, CellAddress::from_a1("B2").expect("B2")).expect("range");
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            anchor,
            FormulaText::from_xlsx("SEQUENCE(2,2)").expect("formula"),
            Some(range),
        )
        .expect("dynamic formula");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [("A1", 1.0), ("B1", 2.0), ("A2", 3.0), ("B2", 4.0)] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("spill address"),
        );
        assert_eq!(
            calculation.materialized_cell(id).map(|cell| cell.result()),
            Some(&CalculationCellResult::Value(
                CellValue::number(expected).expect("finite spill value")
            ))
        );
    }

    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write dynamic workbook");
    assert!(
        support::archive_parts(output.bytes()).contains_key("xl/metadata.xml"),
        "canonical dynamic workbooks carry cell metadata"
    );
    let reopened =
        open_xlsx_document_bytes(output.bytes(), cellrune::OpenOptions::default()).expect("reopen");
    let sheet = reopened.workbook().sheet_by_name("Sheet1").expect("sheet");
    let CellContent::Formula(formula) = sheet.cell(anchor).expect("anchor cell").content() else {
        panic!("dynamic anchor must remain a formula");
    };
    assert_eq!(
        formula.metadata(),
        &FormulaMetadata::DynamicArray {
            range: Some(range),
            always_calculate: false,
        }
    );
    assert_eq!(
        formula.saved_result(),
        &SavedResult::Present(CellValue::number(1.0).expect("finite"))
    );
    assert_eq!(
        sheet
            .cell(CellAddress::from_a1("B2").expect("B2"))
            .expect("spill follower")
            .content(),
        &CellContent::Literal(CellValue::number(4.0).expect("finite"))
    );

    let recalculation = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    let recalculated = write_recalculated_xlsx_bytes(
        &reopened,
        &recalculation,
        RecalculationWriteOptions::default(),
    )
    .expect("existing dynamic document recalculation");
    assert!(recalculated.report().is_complete());
    assert!(
        support::archive_parts(recalculated.bytes()).contains_key("xl/metadata.xml"),
        "existing dynamic metadata must be preserved during cache rewriting"
    );

    let mut blocked = WorkbookDraft::new();
    let blocked_sheet = blocked.workbook().sheets()[0].id();
    blocked
        .set_cell_dynamic_formula(
            blocked_sheet,
            anchor,
            FormulaText::from_xlsx("SEQUENCE(2)").expect("formula"),
            None,
        )
        .expect("dynamic formula");
    blocked
        .set_cell_value(
            blocked_sheet,
            CellAddress::from_a1("A2").expect("A2"),
            CellValue::Text("occupied".to_owned()),
        )
        .expect("obstruction");
    let blocked_calculation = calculate_workbook(blocked.workbook(), CalculationOptions::default());
    assert_eq!(
        blocked_calculation.cell(CalculationCellId::new(blocked_sheet, anchor)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Spill
        )))
    );
}

#[test]
fn draft_path_save_is_opt_in_and_failed_preparation_preserves_destination() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("A1").expect("A1"),
            FormulaText::from_xlsx("1+1").expect("formula"),
        )
        .expect("formula");
    let stale = calculate_workbook(draft.workbook(), CalculationOptions::default());
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("B1").expect("B1"),
            CellValue::Text("new revision".to_owned()),
        )
        .expect("edit after calculation");

    let output = support::TemporaryOutput::new("xlsx");
    std::fs::write(output.path(), b"existing-destination").expect("sentinel");
    let stale_error = write_xlsx_draft_path(
        &draft,
        &stale,
        output.path(),
        RecalculationWriteOptions::new(WriteOptions::default().with_replace_existing(true)),
    )
    .expect_err("stale calculation");
    assert_eq!(
        stale_error.code(),
        XlsxWriteErrorCode::StaleSemanticRevision
    );
    assert_eq!(
        std::fs::read(output.path()).expect("sentinel remains"),
        b"existing-destination"
    );

    let current = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let exists_error = write_xlsx_draft_path(
        &draft,
        &current,
        output.path(),
        RecalculationWriteOptions::default(),
    )
    .expect_err("replacement is opt in");
    assert_eq!(exists_error.code(), XlsxWriteErrorCode::DestinationExists);
    assert_eq!(
        std::fs::read(output.path()).expect("sentinel remains"),
        b"existing-destination"
    );

    write_xlsx_draft_path(
        &draft,
        &current,
        output.path(),
        RecalculationWriteOptions::new(WriteOptions::default().with_replace_existing(true)),
    )
    .expect("explicit replacement");
    let reopened =
        cellrune::open_xlsx_document_path(output.path(), cellrune::OpenOptions::default())
            .expect("reopen path output");
    assert!(reopened.workbook().sheet_by_name("Sheet1").is_some());
}

#[test]
fn document_draft_preserves_unknown_parts_and_patches_only_declared_semantics() {
    let mut seed = WorkbookDraft::new();
    let sheet_id = seed.workbook().sheets()[0].id();
    seed.set_cell_value(
        sheet_id,
        CellAddress::from_a1("A1").expect("A1"),
        CellValue::Number(FiniteNumber::new(2.0).expect("finite")),
    )
    .expect("seed A1");
    seed.set_cell_formula(
        sheet_id,
        CellAddress::from_a1("B1").expect("B1"),
        FormulaText::from_xlsx("A1+1").expect("formula"),
    )
    .expect("seed B1");
    let untouched = seed
        .add_sheet(SheetName::new("Untouched").expect("name"))
        .expect("add untouched sheet");
    seed.set_cell_value(
        untouched,
        CellAddress::from_a1("D4").expect("D4"),
        CellValue::Text("preserve me".to_owned()),
    )
    .expect("seed untouched sheet");
    let seed_calculation = calculate_workbook(seed.workbook(), CalculationOptions::default());
    let seed_output = write_xlsx_draft_bytes(
        &seed,
        &seed_calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("seed output");
    let source_bytes = support::rewrite_archive(
        seed_output.bytes(),
        &std::collections::BTreeMap::new(),
        &[("custom/opaque.bin", b"\x00CellRune-opaque\xff")],
    );
    let document =
        open_xlsx_document_bytes(&source_bytes, cellrune::OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("A1").expect("A1"),
            CellValue::Number(FiniteNumber::new(7.0).expect("finite")),
        )
        .expect("edit A1");
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("B1").expect("B1"),
            FormulaText::from_xlsx("A1*2").expect("formula"),
        )
        .expect("edit B1");
    draft
        .set_cell_number_format(
            sheet_id,
            CellAddress::from_a1("A1").expect("A1"),
            NumberFormat::custom(165, "0.000", NumberFormatKind::Number).expect("format"),
        )
        .expect("format A1");
    draft
        .rename_sheet(sheet_id, SheetName::new("Renamed").expect("name"))
        .expect("rename");
    let added = draft
        .add_sheet(SheetName::new("Added").expect("name"))
        .expect("add sheet");
    draft
        .set_cell_value(
            added,
            CellAddress::from_a1("C3").expect("C3"),
            CellValue::Text("new sheet".to_owned()),
        )
        .expect("set added cell");
    draft
        .set_date_system(cellrune::DateSystem::Excel1904)
        .expect("date system");
    draft
        .set_sheet_visibility(added, SheetVisibility::Hidden)
        .expect("hide added sheet");
    draft
        .set_defined_name(
            DefinedName::new(
                "InputValue",
                DefinedNameScope::Workbook,
                FormulaText::from_xlsx("Renamed!$A$1").expect("name formula"),
                false,
            )
            .expect("defined name"),
        )
        .expect("set defined name");
    draft
        .set_calculation_hints(CalculationHints::new(
            Some(CalculationMode::Manual),
            None,
            None,
            None,
        ))
        .expect("calculation hints");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("document draft output");
    assert_eq!(
        support::archive_parts(&source_bytes)
            .remove("custom/opaque.bin")
            .expect("source opaque part"),
        support::archive_parts(output.bytes())
            .remove("custom/opaque.bin")
            .expect("output opaque part")
    );
    assert_eq!(
        support::archive_parts(&source_bytes)
            .remove("xl/worksheets/sheet2.xml")
            .expect("source untouched worksheet"),
        support::archive_parts(output.bytes())
            .remove("xl/worksheets/sheet2.xml")
            .expect("output untouched worksheet")
    );
    let second = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("deterministic document output");
    assert_eq!(output.bytes(), second.bytes());
    let reopened =
        open_xlsx_document_bytes(output.bytes(), cellrune::OpenOptions::default()).expect("reopen");
    let renamed = reopened
        .workbook()
        .sheet_by_name("Renamed")
        .expect("renamed sheet");
    assert_eq!(
        renamed
            .cell_by_a1("A1")
            .expect("valid A1")
            .expect("A1")
            .number_format()
            .code(),
        Some("0.000")
    );
    let CellContent::Formula(formula) = renamed
        .cell_by_a1("B1")
        .expect("valid B1")
        .expect("B1")
        .content()
    else {
        panic!("B1 formula");
    };
    assert_eq!(
        formula.saved_result(),
        &SavedResult::Present(CellValue::Number(FiniteNumber::new(14.0).expect("finite")))
    );
    assert_eq!(
        reopened.workbook().date_system(),
        cellrune::DateSystem::Excel1904
    );
    assert_eq!(
        reopened
            .workbook()
            .sheet_by_name("Added")
            .expect("added sheet")
            .visibility(),
        SheetVisibility::Hidden
    );
    assert_eq!(reopened.workbook().defined_names().len(), 1);
    assert_eq!(
        reopened.workbook().calculation_hints().mode(),
        Some(CalculationMode::Manual)
    );
}

#[test]
fn document_draft_round_trips_an_iterative_calculation_declaration_and_replaces_it_on_request() {
    let mut seed = WorkbookDraft::new();
    let sheet_id = seed.workbook().sheets()[0].id();
    seed.set_cell_value(
        sheet_id,
        CellAddress::from_a1("A1").expect("A1"),
        CellValue::Number(FiniteNumber::new(2.0).expect("finite")),
    )
    .expect("seed A1");
    let seed_calculation = calculate_workbook(seed.workbook(), CalculationOptions::default());
    let seed_output = write_xlsx_draft_bytes(
        &seed,
        &seed_calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("seed output");

    // A workbook authored elsewhere with iterative calculation switched on.
    let workbook_part = String::from_utf8(
        support::archive_parts(seed_output.bytes())
            .remove("xl/workbook.xml")
            .expect("seed workbook part"),
    )
    .expect("UTF-8 workbook part");
    assert!(
        !workbook_part.contains("<calcPr"),
        "the canonical writer omits calcPr here, so the declaration is injected below: \
         {workbook_part}"
    );
    let iterative_part = workbook_part.replace(
        "</workbook>",
        r#"<calcPr calcId="191029" iterate="1" iterateCount="7"/></workbook>"#,
    );
    let source_bytes = support::rewrite_archive(
        seed_output.bytes(),
        &std::collections::BTreeMap::from([("xl/workbook.xml".to_owned(), iterative_part)]),
        &[],
    );

    let document =
        open_xlsx_document_bytes(&source_bytes, cellrune::OpenOptions::default()).expect("open");
    assert_eq!(
        document
            .workbook()
            .calculation_hints()
            .iterative_calculation(),
        Some(true),
        "the source declaration has to survive reading before the write can be judged"
    );

    // Editing a cell without touching the hints preserves the whole declaration, siblings
    // included. This is the ordinary round trip.
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("A1").expect("A1"),
            CellValue::Number(FiniteNumber::new(9.0).expect("finite")),
        )
        .expect("edit A1");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("document draft output");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), cellrune::OpenOptions::default()).expect("reopen");
    assert_eq!(
        reopened
            .workbook()
            .calculation_hints()
            .iterative_calculation(),
        Some(true)
    );
    let reopened_part = String::from_utf8(
        support::archive_parts(output.bytes())
            .remove("xl/workbook.xml")
            .expect("output workbook part"),
    )
    .expect("UTF-8 workbook part");
    assert!(
        reopened_part.contains(r#"iterateCount="7""#),
        "the sibling attribute travels with the declaration: {reopened_part}"
    );

    // `CalculationHints` is the complete calcPr declaration, so setting it replaces rather than
    // merges: a value built without the flag clears it, and the builder carries it through. The
    // write verification compares the reopened file against the draft's semantic model, so a
    // writer that preserved the source declaration here would fail closed rather than disagree.
    for (hints, expected) in [
        (
            CalculationHints::new(Some(CalculationMode::Manual), None, None, None),
            None,
        ),
        (
            CalculationHints::new(Some(CalculationMode::Manual), None, None, None)
                .with_iterative_calculation(Some(true)),
            Some(true),
        ),
    ] {
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .set_calculation_hints(hints)
            .expect("calculation hints");
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .expect("document draft output");
        let reopened = open_xlsx_document_bytes(output.bytes(), cellrune::OpenOptions::default())
            .expect("reopen");
        let written = reopened.workbook().calculation_hints();
        assert_eq!(written.iterative_calculation(), expected);
        assert_eq!(written.mode(), Some(CalculationMode::Manual));
    }
}

#[test]
fn document_draft_allocates_relationship_ids_without_clobbering_source_ids() {
    let seed = WorkbookDraft::new();
    let seed_calculation = calculate_workbook(seed.workbook(), CalculationOptions::default());
    let seed_output = write_xlsx_draft_bytes(
        &seed,
        &seed_calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("seed output");
    let relationship_path = "xl/_rels/workbook.xml.rels";
    let relationship_xml = String::from_utf8(
        support::archive_parts(seed_output.bytes())
            .remove(relationship_path)
            .expect("workbook relationships"),
    )
    .expect("relationship XML");
    let colliding_relationship = r#"<Relationship Id="rIdCellRuneSheet2" Type="urn:cellrune:test:preserved" Target="../custom/opaque.bin"/>"#;
    let relationship_xml = relationship_xml.replace(
        "</Relationships>",
        &format!("{colliding_relationship}</Relationships>"),
    );
    let source_bytes = support::rewrite_archive(
        seed_output.bytes(),
        &std::collections::BTreeMap::from([(relationship_path.to_owned(), relationship_xml)]),
        &[("custom/opaque.bin", b"preserved")],
    );
    let document =
        open_xlsx_document_bytes(&source_bytes, cellrune::OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .add_sheet(SheetName::new("Added").expect("sheet name"))
        .expect("add sheet");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("write with colliding source relationship ID");

    let output_parts = support::archive_parts(output.bytes());
    let output_relationships =
        String::from_utf8(output_parts[relationship_path].clone()).expect("relationship XML");
    assert!(output_relationships.contains(colliding_relationship));
    assert!(output_relationships.contains(r#"Id="rIdCellRuneSheet2_1""#));
    let reopened =
        open_xlsx_document_bytes(output.bytes(), cellrune::OpenOptions::default()).expect("reopen");
    assert!(reopened.workbook().sheet_by_name("Added").is_some());
}
