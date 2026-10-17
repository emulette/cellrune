use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use cellrune::{
    ApplyChangesError, CalculationCellId, CalculationCellResult, CalculationDecisionReason,
    CalculationExecutionMode, CalculationOptions, CancellationToken, CellAddress, CellContent,
    CellValue, EditBatch, OpenOptions, ReadLimits, ReadOptions, RecalculationMode,
    RecalculationWriteOptions, Row, SessionErrorCode, SessionLimits, SheetId, SheetName,
    SheetVisibility, TableColumnId, TableColumnName, TableId, TableName,
    WorkbookCalculationSession, WorkbookChange, WorkbookDraft, WorkbookSourceKind, WriteLimits,
    WriteOptions, XlsxDocumentKind, XlsxErrorCode, XlsxWriteErrorCode, calculate_workbook,
    open_xlsx_document, open_xlsx_document_bytes, open_xlsx_document_path, read_xlsx_bytes,
    scan_formula_capabilities, write_preserved_xlsx_bytes, write_xlsx_draft_bytes,
};
use sha2::{Digest, Sha256};
use zip::read::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::support::generated_xlsx::{
    ProducerProfile, TemporaryWorkbook, generated_empty_sheet_data_workbook,
    generated_formula_fixture, generated_legacy_array_workbook, generated_shared_formula_workbook,
    generated_table_reference_fixture, generated_table_topology_fixture,
    generated_totals_table_fixture, generated_workbook, generated_workbook_with_comment,
    generated_workbook_with_sheet_count,
};

#[test]
fn typed_reference_formula_fixture_survives_preserved_write_and_reopen() {
    let formulas = [
        "Table1[ @Amount ]",
        "_xlfn.ANCHORARRAY((A1,B1))",
        "[Book.xlsx]Sheet1:Sheet3!A1",
        "[1]!DataTable[Amount]",
    ];
    let bytes = generated_formula_fixture(&formulas);
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("grammar document");
    let output = write_preserved_xlsx_bytes(&document, WriteOptions::default())
        .expect("preserved grammar output");
    let reopened =
        read_xlsx_bytes(&output, ReadOptions::default()).expect("reopened grammar output");
    let sheet = reopened
        .sheet_by_name("Calculations")
        .expect("Calculations sheet");
    for (index, expected) in formulas.iter().enumerate() {
        let address = format!("B{}", index + 2);
        let cell = sheet
            .cell_by_a1(&address)
            .expect("valid address")
            .expect("formula cell");
        let CellContent::Formula(formula) = cell.content() else {
            panic!("expected formula at {address}");
        };
        assert_eq!(formula.text().expect("formula text").as_str(), *expected);
    }
}

#[test]
fn generated_distribution_formulas_calculate_before_and_after_preserved_reopen() {
    let formulas = [
        "GAMMA.DIST(2,3,1.5,TRUE)",
        "BETA.DIST(0.6,8,10,TRUE,0,1)",
        "BINOM.DIST.RANGE(10,0.4,3,6)",
        "CRITBINOM(10,0.4,0.6)",
        "HYPGEOM.DIST(1,4,8,20,TRUE)",
        "F.DIST(1,5,7,TRUE)",
        "F.DIST.RT(1,5,7)",
        "F.INV(0.6,5,7)",
        "F.INV.RT(0.4,5,7)",
        "F.TEST({1,2,3,4},{2,4,8,16})",
        "FDIST(1,5,7)",
        "FINV(0.4,5,7)",
        "FTEST({1,2,3,4},{2,4,8,16})",
        "T.DIST(1.5,10,TRUE)",
        "T.DIST.2T(1.5,10)",
        "T.DIST.RT(1.5,10)",
        "T.INV(0.9,10)",
        "T.INV.2T(0.1,10)",
        "T.TEST({1,2,3,4},{1.1,2.1,3.2,4.2},2,1)",
        "TDIST(1.5,10,2)",
        "TINV(0.1,10)",
        "TTEST({1,2,3,4},{1.1,2.1,3.2,4.2},2,1)",
        "Z.TEST({1,2,3,4,5},2.5)",
        "ZTEST({1,2,3,4,5},2.5)",
        "COVARIANCE.S({1,2,3,4},{2,4,5,8})",
    ];
    let bytes = generated_formula_fixture(&formulas);
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("distribution document");
    assert!(scan_formula_capabilities(document.workbook()).is_supported());
    let before = calculate_workbook(document.workbook(), CalculationOptions::default());

    let output = write_preserved_xlsx_bytes(&document, WriteOptions::default())
        .expect("preserved distribution output");
    let reopened =
        read_xlsx_bytes(&output, ReadOptions::default()).expect("reopened distribution output");
    assert!(scan_formula_capabilities(&reopened).is_supported());
    let after = calculate_workbook(&reopened, CalculationOptions::default());

    let source_sheet = document
        .workbook()
        .sheet_by_name("Calculations")
        .expect("source Calculations sheet")
        .id();
    let reopened_sheet = reopened
        .sheet_by_name("Calculations")
        .expect("reopened Calculations sheet")
        .id();
    for index in 0..formulas.len() {
        let address =
            CellAddress::from_a1(&format!("B{}", index + 2)).expect("valid formula address");
        let before_cell = before.cell(CalculationCellId::new(source_sheet, address));
        let after_cell = after.cell(CalculationCellId::new(reopened_sheet, address));
        assert!(
            matches!(
                before_cell,
                Some(CalculationCellResult::Value(CellValue::Number(_)))
            ),
            "expected a numeric distribution result at B{}",
            index + 2
        );
        assert_eq!(before_cell, after_cell, "mismatch at B{}", index + 2);
    }
}

#[test]
fn generated_table_references_calculate_before_and_after_preserved_reopen() {
    let bytes = generated_table_reference_fixture();
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("table document");
    assert!(scan_formula_capabilities(document.workbook()).is_supported());
    assert_table_reference_results(document.workbook());

    let output = write_preserved_xlsx_bytes(&document, WriteOptions::default())
        .expect("preserved table-reference output");
    let reopened =
        read_xlsx_bytes(&output, ReadOptions::default()).expect("reopened table-reference output");
    assert_eq!(
        reopened.table("sales").expect("reopened table").range(),
        document
            .workbook()
            .table("Sales")
            .expect("source table")
            .range()
    );
    assert!(scan_formula_capabilities(&reopened).is_supported());
    assert_table_reference_results(&reopened);
}

#[test]
fn generated_table_topology_growth_and_shrink_variants_reopen() {
    for data_rows in [1_u32, 4] {
        let bytes = generated_table_topology_fixture(data_rows);
        let document =
            open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("table topology input");
        assert_table_topology_results(document.workbook(), data_rows);

        let output = write_preserved_xlsx_bytes(&document, WriteOptions::default())
            .expect("preserved table topology output");
        let reopened = read_xlsx_bytes(&output, ReadOptions::default())
            .expect("reopened table topology output");
        assert_table_topology_results(&reopened, data_rows);
    }
}

#[test]
fn generated_table_value_edits_match_incremental_and_full_calculation() {
    let bytes = generated_table_topology_fixture(3);
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("table value input");
    let mut session = WorkbookCalculationSession::new(WorkbookDraft::from_document(&document));
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial table calculation");
    let sheet_id = session.workbook().sheets()[0].id();
    session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet_id,
                CellAddress::from_a1("B3").expect("table data cell"),
                CellValue::number(200.0).expect("finite table edit"),
            )]),
        )
        .expect("table data edit");
    let delta = session
        .recalculate(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("incremental table calculation");
    assert_eq!(delta.mode(), CalculationExecutionMode::Incremental);
    let incremental = session.calculation().expect("installed calculation");
    let full = calculate_workbook(session.workbook(), CalculationOptions::default());
    assert_eq!(
        incremental.cells().collect::<Vec<_>>(),
        full.cells().collect::<Vec<_>>(),
    );
    for (address, expected) in [("C3", 200.0), ("E1", 240.0), ("F1", 3.0)] {
        let id = CalculationCellId::new(
            sheet_id,
            CellAddress::from_a1(address).expect("table result address"),
        );
        assert_eq!(
            incremental.cell(id),
            Some(&CalculationCellResult::Value(
                CellValue::number(expected).expect("finite table result")
            )),
            "{address}",
        );
    }
}

#[test]
fn table_authoring_forces_a_full_rebuild_that_matches_fresh_calculation() {
    let bytes = generated_table_topology_fixture(3);
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("table authoring input");
    let mut session = WorkbookCalculationSession::new(WorkbookDraft::from_document(&document));
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial table calculation");
    let table_id = TableId::new(1).expect("stable table ID");
    let receipt = session
        .apply_changes(
            session.workbook().semantic_revision(),
            EditBatch::new([
                WorkbookChange::rename_table(
                    table_id,
                    TableName::new("Orders").expect("valid table name"),
                ),
                WorkbookChange::rename_table_column(
                    table_id,
                    TableColumnId::new(2).expect("stable column ID"),
                    TableColumnName::new("Gross Amount").expect("valid column name"),
                ),
                WorkbookChange::resize_table_rows(
                    table_id,
                    Row::new(2).expect("first data row"),
                    Row::new(5).expect("last data row"),
                )
                .expect("valid resize"),
            ]),
        )
        .expect("table authoring");
    assert_eq!(receipt.changed_table_ids(), [table_id]);
    assert!(receipt.topology_changed());

    let error = session
        .prepare_recalculation(
            RecalculationMode::Incremental,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect_err("forced incremental cannot reuse stale table topology");
    assert_eq!(error.code(), SessionErrorCode::IncrementalUnsafe);
    let delta = session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("automatic full rebuild");
    assert_eq!(delta.mode(), CalculationExecutionMode::Full);
    assert_eq!(delta.reason(), CalculationDecisionReason::TopologyChanged);
    assert_eq!(
        session
            .calculation()
            .expect("installed authored calculation")
            .cells()
            .collect::<Vec<_>>(),
        calculate_workbook(session.workbook(), CalculationOptions::default())
            .cells()
            .collect::<Vec<_>>()
    );
}

#[test]
fn table_rewrite_budget_and_cancellation_leave_the_session_unchanged() {
    let bytes = generated_table_reference_fixture();
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("table document");
    let limits = SessionLimits::default()
        .with_formula_rewrite_limits(1, usize::MAX, usize::MAX, usize::MAX)
        .expect("positive rewrite limits");
    let session =
        WorkbookCalculationSession::with_limits(WorkbookDraft::from_document(&document), limits);
    let revision = session.workbook().semantic_revision();
    let table_id = TableId::new(1).expect("stable table ID");
    let rename = EditBatch::new([WorkbookChange::rename_table(
        table_id,
        TableName::new("Orders").expect("valid table name"),
    )]);

    let error = session
        .prepare_changes(revision, rename.clone())
        .expect_err("whole-workbook formula count exceeds the configured limit");
    assert!(matches!(
        error,
        ApplyChangesError::Session(error)
            if error.code() == SessionErrorCode::RewriteLimitExceeded
    ));
    assert_eq!(session.workbook().semantic_revision(), revision);
    assert_eq!(
        session
            .workbook()
            .table_by_id(table_id)
            .expect("source table")
            .display_name()
            .as_str(),
        "Sales"
    );

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = session
        .prepare_changes_cancellable(revision, rename, &cancellation)
        .expect_err("pre-cancelled staging fails");
    assert!(matches!(
        error,
        ApplyChangesError::Session(error) if error.code() == SessionErrorCode::Cancelled
    ));
    assert_eq!(session.workbook().semantic_revision(), revision);
}

fn assert_table_topology_results(workbook: &cellrune::WorkbookSnapshot, data_rows: u32) {
    assert!(scan_formula_capabilities(workbook).is_supported());
    let table = workbook.table("Sales").expect("Sales table");
    assert_eq!(
        table.range().start(),
        CellAddress::from_a1("A1").expect("start")
    );
    assert_eq!(
        table.range().end(),
        CellAddress::from_indices(data_rows + 1, 3).expect("topology end"),
    );
    let sheet = workbook.sheet_by_name("Data").expect("Data sheet");
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    for (address, expected) in [
        (
            "E1".to_owned(),
            f64::from(data_rows * (data_rows + 1) / 2 * 10),
        ),
        ("F1".to_owned(), f64::from(data_rows)),
        (format!("C{}", data_rows + 1), f64::from(data_rows * 10)),
    ] {
        let id = CalculationCellId::new(
            sheet.id(),
            CellAddress::from_a1(&address).expect("result address"),
        );
        assert_eq!(
            calculation.cell(id),
            Some(&CalculationCellResult::Value(
                CellValue::number(expected).expect("finite topology result")
            )),
            "data_rows={data_rows}, address={address}",
        );
    }
}

fn assert_table_reference_results(workbook: &cellrune::WorkbookSnapshot) {
    let sheet = workbook.sheet_by_name("Data").expect("Data sheet");
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    for (address, expected) in [
        ("C2", 10.0),
        ("C3", 20.0),
        ("C4", 30.0),
        ("H1", 60.0),
        ("H2", 3.0),
        ("H3", 1.0),
        ("H4", 120.0),
        ("H5", 1.0),
    ] {
        let id = CalculationCellId::new(
            sheet.id(),
            CellAddress::from_a1(address).expect("result address"),
        );
        let Some(CalculationCellResult::Value(CellValue::Number(actual))) = calculation.cell(id)
        else {
            panic!("numeric table-reference result expected at {address}");
        };
        assert_eq!(actual.get(), expected, "{address}");
    }
}

#[test]
fn document_adapters_retain_exact_identity_without_changing_read_only_behavior() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let temporary = TemporaryWorkbook::new(&bytes);
    let expected_hash: [u8; 32] = Sha256::digest(&bytes).into();

    let from_bytes =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("byte document");
    let from_reader = open_xlsx_document(Cursor::new(bytes.clone()), OpenOptions::default())
        .expect("reader document");
    let from_path =
        open_xlsx_document_path(temporary.path(), OpenOptions::default()).expect("path document");

    for document in [&from_bytes, &from_reader, &from_path] {
        assert_eq!(document.input_hash().as_bytes(), &expected_hash);
        assert_eq!(document.input_bytes(), bytes.as_slice());
        assert_eq!(
            <[u8; 32]>::from(Sha256::digest(document.input_bytes())),
            *document.input_hash().as_bytes()
        );
        assert_eq!(
            document.workbook().provenance().input_hash(),
            Some(document.input_hash())
        );
        assert_eq!(document.kind(), XlsxDocumentKind::Xlsx);
        assert_eq!(document.semantic_revision(), 0);
        assert_eq!(document.workbook_part().as_str(), "xl/workbook.xml");
        assert_eq!(
            document
                .worksheet_part(SheetId::new(1).expect("sheet ID"))
                .expect("Inputs worksheet")
                .as_str(),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            document
                .worksheet_part(SheetId::new(2).expect("sheet ID"))
                .expect("Calculations worksheet")
                .as_str(),
            "xl/worksheets/sheet2.xml"
        );
    }
    assert_eq!(
        from_bytes.workbook().source().kind(),
        WorkbookSourceKind::Bytes
    );
    assert_eq!(
        from_reader.workbook().source().kind(),
        WorkbookSourceKind::Reader
    );
    assert_eq!(
        from_path.workbook().source().kind(),
        WorkbookSourceKind::Path
    );
    assert!(
        !format!("{from_path:?}")
            .contains(temporary.path().to_str().expect("temporary path is UTF-8"))
    );

    let read_only =
        read_xlsx_bytes(&bytes, ReadOptions::default()).expect("read-only adapter remains valid");
    assert_eq!(read_only.provenance().input_hash(), None);

    let calculation = calculate_workbook(from_bytes.workbook(), CalculationOptions::default());
    assert_eq!(
        calculation.provenance().input_hash(),
        Some(from_bytes.input_hash())
    );
}

#[test]
fn package_preservation_copy_reopens_and_retains_every_uncompressed_part() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("source document");
    let output = write_preserved_xlsx_bytes(&document, WriteOptions::default())
        .expect("preserved package output");

    let reopened =
        open_xlsx_document_bytes(&output, OpenOptions::default()).expect("reopened output");
    assert_eq!(
        reopened.workbook().sheets().len(),
        document.workbook().sheets().len()
    );
    assert_eq!(archive_manifest(&output), archive_manifest(&bytes));
}

#[test]
fn mutated_archive_bytes_change_identity_even_when_workbook_semantics_match() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let mutated = generated_workbook_with_comment(ProducerProfile::Excel, "identity mutation");
    let original =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("original document");
    let reopened =
        open_xlsx_document_bytes(&mutated, OpenOptions::default()).expect("mutated document");

    assert_ne!(original.input_hash(), reopened.input_hash());
    assert_eq!(
        original.workbook().sheets().len(),
        reopened.workbook().sheets().len()
    );
    assert_eq!(archive_manifest(&bytes), archive_manifest(&mutated));
}

#[test]
fn added_sheets_bind_root_and_element_scoped_relationship_namespaces() {
    const TRANSITIONAL: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";

    for (case, prefix, namespace, element_scoped) in [
        ("root-transitional", "r", TRANSITIONAL, false),
        ("element-transitional", "r", TRANSITIONAL, true),
        ("alternate-prefix", "rel", TRANSITIONAL, true),
        ("element-strict", "strictRel", STRICT, true),
    ] {
        let source = workbook_with_relationship_namespace(
            &generated_workbook(ProducerProfile::Excel),
            prefix,
            namespace,
            element_scoped,
        );
        let document = open_xlsx_document_bytes(&source, OpenOptions::default())
            .unwrap_or_else(|error| panic!("{case}: open input: {error}"));
        let inputs = document
            .workbook()
            .sheet_by_name("Inputs")
            .expect("Inputs sheet")
            .id();
        let calculations = document
            .workbook()
            .sheet_by_name("Calculations")
            .expect("Calculations sheet");
        assert_eq!(calculations.visibility(), SheetVisibility::Hidden, "{case}");

        let mut draft = WorkbookDraft::from_document(&document);
        let first_added = draft
            .add_sheet(SheetName::new("Added One").expect("sheet name"))
            .expect("add first sheet");
        let second_added = draft
            .add_sheet(SheetName::new("Added Two").expect("sheet name"))
            .expect("add second sheet");
        draft
            .set_cell_value(
                first_added,
                CellAddress::from_a1("A1").expect("address"),
                CellValue::number(7.0).expect("finite value"),
            )
            .expect("set added value");
        draft
            .set_cell_formula(
                second_added,
                CellAddress::from_a1("A1").expect("address"),
                cellrune::FormulaText::from_xlsx("'Added One'!A1+1").expect("formula"),
            )
            .expect("set added formula");
        let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
        let output =
            write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
                .unwrap_or_else(|error| panic!("{case}: write draft: {error}"));

        let manifest = archive_manifest(output.bytes());
        let workbook_xml =
            std::str::from_utf8(&manifest.get("xl/workbook.xml").expect("workbook part").2)
                .expect("UTF-8 workbook XML");
        let declaration = format!(r#"xmlns:{prefix}="{namespace}""#);
        let relationship = format!("{prefix}:id=");
        assert_eq!(workbook_xml.matches(&relationship).count(), 4, "{case}");
        assert!(
            workbook_xml.matches(&declaration).count() >= 2,
            "{case}: each added sheet needs an in-scope relationship namespace"
        );
        assert!(
            workbook_xml.contains(r#"opaque:marker="preserved""#),
            "{case}"
        );
        assert!(workbook_xml.contains(r#"opaque:id="foreign""#), "{case}");

        let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default())
            .unwrap_or_else(|error| panic!("{case}: reopen output: {error}"));
        assert_eq!(reopened.workbook().sheets().len(), 4, "{case}");
        assert_eq!(
            reopened
                .workbook()
                .sheet_by_id(inputs)
                .expect("existing Inputs sheet")
                .name()
                .as_str(),
            "Inputs",
            "{case}",
        );
        assert_eq!(
            reopened
                .workbook()
                .sheet_by_name("Calculations")
                .expect("existing Calculations sheet")
                .visibility(),
            SheetVisibility::Hidden,
            "{case}",
        );
        assert!(
            reopened.workbook().sheet_by_id(first_added).is_some(),
            "{case}"
        );
        assert!(
            reopened.workbook().sheet_by_id(second_added).is_some(),
            "{case}"
        );
    }
}

#[test]
fn document_and_write_limits_fail_with_stable_codes() {
    let bytes = generated_workbook(ProducerProfile::Excel);
    let read_limits = ReadLimits::default()
        .with_max_archive_bytes(bytes.len() as u64 - 1)
        .expect("positive read limit");
    let read_error =
        open_xlsx_document_bytes(&bytes, OpenOptions::new(ReadOptions::new(read_limits)))
            .expect_err("archive limit must apply before document parsing");
    assert_eq!(read_error.code(), XlsxErrorCode::ArchiveTooLarge);

    let document =
        open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("source document");
    let write_limits = WriteLimits::default()
        .with_max_entries(1)
        .expect("positive write limit");
    let write_error = write_preserved_xlsx_bytes(&document, WriteOptions::new(write_limits))
        .expect_err("entry limit must apply before output generation");
    assert_eq!(
        write_error.code(),
        XlsxWriteErrorCode::ResourceLimitExceeded
    );
}

#[test]
fn owned_bytes_over_the_archive_limit_are_rejected_before_opening() {
    let bytes = generated_workbook_with_sheet_count(1);
    let length = bytes.len() as u64;
    let options = OpenOptions::new(ReadOptions::new(
        ReadLimits::default()
            .with_max_archive_bytes(length - 1)
            .expect("positive archive limit"),
    ));

    let error = cellrune::open_xlsx_document_vec(bytes, options)
        .expect_err("an archive over the limit must be rejected");

    assert_eq!(error.code(), XlsxErrorCode::ArchiveTooLarge);
    assert_eq!(error.observed(), Some(length));
}

#[test]
fn cells_added_to_an_empty_sheet_data_element_save_and_reopen() {
    let document = open_xlsx_document_bytes(
        &generated_empty_sheet_data_workbook(),
        OpenOptions::default(),
    )
    .expect("open empty worksheet");
    let sheet_id = document.workbook().sheets()[0].id();
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            CellAddress::from_a1("A1").expect("address"),
            CellValue::number(7.0).expect("finite value"),
        )
        .expect("set value");
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_a1("B2").expect("address"),
            cellrune::FormulaText::from_xlsx("A1+1").expect("formula"),
        )
        .expect("set formula");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("cells added to an empty sheetData element must save");

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen output");
    let sheet = &reopened.workbook().sheets()[0];
    assert_eq!(
        sheet
            .cell(CellAddress::from_a1("A1").expect("address"))
            .map(|cell| cell.content().clone()),
        Some(CellContent::Literal(
            CellValue::number(7.0).expect("finite value")
        ))
    );
    assert!(matches!(
        sheet
            .cell(CellAddress::from_a1("B2").expect("address"))
            .map(|cell| cell.content().clone()),
        Some(CellContent::Formula(formula)) if formula.text().map(|text| text.as_str()) == Some("A1+1")
    ));
}

#[test]
fn replacing_a_shared_formula_anchor_keeps_its_followers_as_formulas() {
    let document =
        open_xlsx_document_bytes(&generated_shared_formula_workbook(), OpenOptions::default())
            .expect("open shared formula workbook");
    let sheet_id = document.workbook().sheets()[0].id();
    let address = |text| CellAddress::from_a1(text).expect("address");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            address("B1"),
            CellValue::number(100.0).expect("finite value"),
        )
        .expect("replace the one-column anchor");
    draft
        .clear_cell(sheet_id, address("D1"))
        .expect("clear the two-dimensional anchor");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("an edited shared formula anchor must save");

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen output");
    let sheet = &reopened.workbook().sheets()[0];
    for (follower, text) in [
        ("B2", "A2*2"),
        ("B3", "A3*2"),
        ("E1", "$A1+10"),
        ("D2", "$A2+10"),
        ("E2", "$A2+10"),
    ] {
        let Some(CellContent::Formula(formula)) = sheet
            .cell(address(follower))
            .map(|cell| cell.content().clone())
        else {
            panic!("{follower} must stay a formula");
        };
        assert_eq!(
            formula.text().map(|text| text.as_str()),
            Some(text),
            "{follower}"
        );
        assert_eq!(
            formula.metadata(),
            &cellrune::FormulaMetadata::Normal,
            "{follower}"
        );
    }
    assert!(sheet.cell(address("D1")).is_none());
    let recalculated = calculate_workbook(reopened.workbook(), CalculationOptions::default());
    assert_eq!(
        recalculated.cell(CalculationCellId::new(sheet_id, address("B3"))),
        Some(&CalculationCellResult::Value(
            CellValue::number(6.0).expect("finite value")
        ))
    );
}

#[test]
fn edits_inside_a_legacy_array_are_rejected_before_saving() {
    let document =
        open_xlsx_document_bytes(&generated_legacy_array_workbook(), OpenOptions::default())
            .expect("open legacy array workbook");
    let sheet_id = document.workbook().sheets()[0].id();
    let follower = CellAddress::from_a1("D2").expect("address");
    let mut draft = WorkbookDraft::from_document(&document);
    let errors = [
        draft
            .set_cell_value(
                sheet_id,
                follower,
                CellValue::number(5.0).expect("finite value"),
            )
            .expect_err("value inside the array"),
        draft
            .set_cell_formula(
                sheet_id,
                follower,
                cellrune::FormulaText::from_xlsx("1").expect("formula"),
            )
            .expect_err("formula inside the array"),
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                follower,
                cellrune::FormulaText::from_xlsx("SEQUENCE(1)").expect("formula"),
                None,
            )
            .expect_err("dynamic formula inside the array"),
        draft
            .clear_cell(sheet_id, follower)
            .expect_err("clear inside the array"),
        draft
            .set_annotated_text(
                sheet_id,
                follower,
                "text",
                Vec::new(),
                cellrune::PhoneticWriteOptions::new(true),
            )
            .expect_err("annotated text inside the array"),
    ];
    for error in errors {
        assert_eq!(
            error.code(),
            cellrune::ValidationErrorCode::ArrayFormulaPartEdit
        );
    }

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("rejected edits leave the draft saveable");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen output");
    assert_eq!(
        reopened.workbook().sheets()[0]
            .cell(follower)
            .map(|cell| cell.content().clone()),
        document.workbook().sheets()[0]
            .cell(follower)
            .map(|cell| cell.content().clone())
    );
}

#[test]
fn cells_added_and_removed_in_one_draft_leave_nothing_to_save() {
    let document = open_xlsx_document_bytes(
        &generated_workbook_with_sheet_count(1),
        OpenOptions::default(),
    )
    .expect("open workbook");
    let sheet_id = document.workbook().sheets()[0].id();
    let address = |text| CellAddress::from_a1(text).expect("address");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            address("C5"),
            CellValue::number(1.0).expect("finite value"),
        )
        .expect("add C5");
    draft
        .clear_cell(sheet_id, address("C5"))
        .expect("remove the added C5");
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_value(
                sheet_id,
                address("D5"),
                CellValue::number(2.0).expect("finite value"),
            ),
            WorkbookChange::set_cell_value(sheet_id, address("D5"), CellValue::Blank),
            WorkbookChange::set_cell_value(
                sheet_id,
                address("A1"),
                CellValue::number(3.0).expect("finite value"),
            ),
        ]))
        .expect("add and blank D5 in one batch");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("a cell the draft added and removed must not block saving");

    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen output");
    let sheet = &reopened.workbook().sheets()[0];
    assert!(sheet.cell(address("C5")).is_none());
    assert!(sheet.cell(address("D5")).is_none());
    assert_eq!(
        sheet.cell(address("A1")).map(|cell| cell.content().clone()),
        Some(CellContent::Literal(
            CellValue::number(3.0).expect("finite value")
        ))
    );
}

#[test]
fn edited_document_output_is_verified_with_the_source_read_limits() {
    let sheet_count = ReadLimits::default().max_sheets() + 1;
    let bytes = generated_workbook_with_sheet_count(sheet_count);
    let default_error = open_xlsx_document_bytes(&bytes, OpenOptions::default())
        .expect_err("the default sheet limit must reject the source");
    assert_eq!(default_error.code(), XlsxErrorCode::TooManySheets);

    let read_options = ReadOptions::new(
        ReadLimits::default()
            .with_max_sheets(sheet_count)
            .expect("positive sheet limit"),
    );
    let document = open_xlsx_document_bytes(&bytes, OpenOptions::new(read_options))
        .expect("the raised sheet limit opens the source");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            document.workbook().sheets()[0].id(),
            CellAddress::from_a1("A1").expect("address"),
            CellValue::number(7.0).expect("finite value"),
        )
        .expect("set value");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("an edited draft must save under the limits its source was opened with");

    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::new(read_options))
        .expect("reopen output");
    assert_eq!(reopened.workbook().sheets().len() as u64, sheet_count);
    assert_eq!(
        reopened.workbook().sheets()[0]
            .cell(CellAddress::from_a1("A1").expect("address"))
            .map(|cell| cell.content().clone()),
        Some(CellContent::Literal(
            CellValue::number(7.0).expect("finite value")
        ))
    );
}

fn archive_manifest(bytes: &[u8]) -> BTreeMap<String, (zip::CompressionMethod, u32, Vec<u8>)> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid archive");
    let mut manifest = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("archive entry");
        if file.is_dir() {
            continue;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).expect("archive contents");
        let replaced = manifest.insert(
            file.name().expect("entry name").into_owned(),
            (file.compression(), file.crc32(), contents),
        );
        assert!(replaced.is_none(), "duplicate fixture entry");
    }
    manifest
}

fn workbook_with_relationship_namespace(
    bytes: &[u8],
    prefix: &str,
    namespace: &str,
    element_scoped: bool,
) -> Vec<u8> {
    const ROOT_DECLARATION: &str =
        "          xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("source archive");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("source entry");
        let name = file.name().expect("entry name").into_owned();
        let compression = file.compression();
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).expect("source contents");
        if name == "xl/workbook.xml" {
            let mut workbook = String::from_utf8(contents).expect("UTF-8 workbook XML");
            if element_scoped {
                workbook = workbook.replace(ROOT_DECLARATION, "");
                let scoped = format!(r#" xmlns:{prefix}="{namespace}" {prefix}:id="#);
                workbook = workbook.replace(" r:id=", &scoped);
            }
            workbook = workbook.replace(
                r#"name="Calculations" sheetId="2""#,
                r#"name="Calculations" sheetId="2" state="hidden""#,
            );
            let first_relationship = format!(r#"{prefix}:id="rId1"/>"#);
            let marked_relationship = format!(
                r#"xmlns:opaque="urn:cellrune:test" opaque:marker="preserved" {prefix}:id="rId1" opaque:id="foreign"/>"#
            );
            workbook = workbook.replacen(&first_relationship, &marked_relationship, 1);
            contents = workbook.into_bytes();
        }
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(compression),
            )
            .expect("start rewritten entry");
        writer.write_all(&contents).expect("write rewritten entry");
    }
    writer
        .finish()
        .expect("finish rewritten archive")
        .into_inner()
}

fn totals_table_cell(workbook: &cellrune::WorkbookSnapshot, address: &str) -> Option<CellContent> {
    workbook.sheets()[0]
        .cell(CellAddress::from_a1(address).expect("valid address"))
        .map(|cell| cell.content().clone())
}

fn assert_totals_formula(workbook: &cellrune::WorkbookSnapshot, address: &str, expected: &str) {
    let Some(CellContent::Formula(formula)) = totals_table_cell(workbook, address) else {
        panic!("{address} must hold a formula");
    };
    assert_eq!(
        formula.text().map(|text| text.as_str()),
        Some(expected),
        "{address}"
    );
}

fn resize_totals_table(changes: Vec<WorkbookChange>) -> (cellrune::WorkbookSnapshot, Vec<u8>) {
    let document =
        open_xlsx_document_bytes(&generated_totals_table_fixture(), OpenOptions::default())
            .expect("totals table input");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(EditBatch::new(changes))
        .expect("resize");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("strict write succeeds without circular references");
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default())
        .expect("reopen resized table");
    (reopened.workbook().clone(), output.bytes().to_vec())
}

fn number_at(workbook: &cellrune::WorkbookSnapshot, address: &str) -> f64 {
    let sheet = workbook.sheets()[0].id();
    let calculation = calculate_workbook(workbook, CalculationOptions::default());
    match calculation.cell(CalculationCellId::new(
        sheet,
        CellAddress::from_a1(address).expect("valid address"),
    )) {
        Some(CalculationCellResult::Value(CellValue::Number(number))) => number.get(),
        other => panic!("{address}: expected a number, got {other:?}"),
    }
}

/// The totals-table fixture as xlsxwriter writes its `[@hp]` calculated column: the column name
/// follows `[#This Row]` without its own brackets.
fn xlsxwriter_totals_table_fixture() -> Vec<u8> {
    let source = generated_totals_table_fixture();
    let mut archive = ZipArchive::new(Cursor::new(&source)).expect("fixture archive");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("fixture entry");
        let name = file.name().expect("entry name").into_owned();
        let mut text = String::new();
        file.read_to_string(&mut text).expect("fixture entry text");
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start entry");
        writer
            .write_all(
                text.replace("tblChar[[#This Row],[hp]]", "[[#This Row],hp]")
                    .as_bytes(),
            )
            .expect("write entry");
    }
    writer.finish().expect("finish archive").into_inner()
}

#[test]
fn xlsxwriter_calculated_columns_calculate_and_an_unrelated_edit_keeps_saved_results() {
    let document =
        open_xlsx_document_bytes(&xlsxwriter_totals_table_fixture(), OpenOptions::default())
            .expect("xlsxwriter table");
    assert_eq!(number_at(document.workbook(), "D2"), 20.0);
    assert_eq!(number_at(document.workbook(), "D4"), 60.0);
    assert_eq!(number_at(document.workbook(), "D5"), 120.0);

    let sheet = SheetId::new(1).expect("sheet");
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_a1("F1").expect("F1"),
            CellValue::number(1.0).expect("number"),
        )]))
        .expect("unrelated edit");
    let edited = cellrune::write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("edits-only write");
    assert!(edited.report().affected_formulas().is_empty());
    assert!(!edited.report().requests_host_recalculation());

    draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_a1("C1").expect("C1"),
            CellValue::Text("hit.points".into()),
        )]))
        .expect("rename the column");
    assert_totals_formula(draft.workbook(), "D2", "[[#This Row],[hit.points]]*2");
}

#[test]
fn growing_a_totals_table_moves_the_totals_row_and_fills_calculated_columns() {
    let sheet = SheetId::new(1).expect("sheet");
    let address = |a1: &str| CellAddress::from_a1(a1).expect("valid address");
    let (workbook, _) = resize_totals_table(vec![
        WorkbookChange::resize_table_rows(
            TableId::new(1).expect("table"),
            Row::new(2).expect("first data row"),
            Row::new(5).expect("last data row"),
        )
        .expect("valid resize"),
        WorkbookChange::set_cell_value(sheet, address("A5"), CellValue::number(4.0).unwrap()),
        WorkbookChange::set_cell_value(sheet, address("B5"), CellValue::Text("d".into())),
        WorkbookChange::set_cell_value(sheet, address("C5"), CellValue::number(40.0).unwrap()),
    ]);
    // The former totals row is a data row with the calculated column.
    assert_totals_formula(&workbook, "D5", "tblChar[[#This Row],[hp]]*2");
    assert_eq!(number_at(&workbook, "D5"), 80.0);
    // The totals row moved below it, naming its table as Excel does.
    assert_eq!(
        totals_table_cell(&workbook, "A6"),
        Some(CellContent::Literal(CellValue::Text("Total".into())))
    );
    assert_eq!(totals_table_cell(&workbook, "B6"), None);
    assert_totals_formula(&workbook, "C6", "SUBTOTAL(109,tblChar[hp])");
    assert_totals_formula(&workbook, "D6", "SUBTOTAL(109,tblChar[hp2])");
    assert_eq!(number_at(&workbook, "C6"), 100.0);
    assert_eq!(number_at(&workbook, "D6"), 200.0);
}

#[test]
fn writing_a_table_header_cell_renames_its_column() {
    let address = |a1: &str| CellAddress::from_a1(a1).expect("valid address");
    let sheet = SheetId::new(1).expect("sheet");
    let (workbook, _) = resize_totals_table(vec![WorkbookChange::set_cell_value(
        sheet,
        address("C1"),
        CellValue::Text("health".into()),
    )]);
    let table = &workbook.sheets()[0].tables()[0];
    assert_eq!(table.columns()[2].name(), "health");
    assert_eq!(
        totals_table_cell(&workbook, "C1"),
        Some(CellContent::Literal(CellValue::Text("health".into())))
    );
    assert_totals_formula(&workbook, "D2", "tblChar[[#This Row],[health]]*2");
    assert_totals_formula(&workbook, "C5", "SUBTOTAL(109,tblChar[health])");
    assert_eq!(number_at(&workbook, "C5"), 60.0);

    let document =
        open_xlsx_document_bytes(&generated_totals_table_fixture(), OpenOptions::default())
            .expect("totals table input");
    let mut draft = WorkbookDraft::from_document(&document);
    for (change, expected) in [
        (
            WorkbookChange::set_cell_value(sheet, address("C1"), CellValue::Blank),
            cellrune::ValidationErrorCode::TableColumnNameEmpty,
        ),
        (
            WorkbookChange::clear_cell(sheet, address("C1")),
            cellrune::ValidationErrorCode::TableColumnNameEmpty,
        ),
        (
            WorkbookChange::set_cell_value(
                sheet,
                address("C1"),
                CellValue::number(1.0).expect("finite value"),
            ),
            cellrune::ValidationErrorCode::TableHeaderRequiresText,
        ),
        (
            WorkbookChange::set_cell_formula(
                sheet,
                address("C1"),
                cellrune::FormulaText::from_xlsx("\"hp\"").expect("formula"),
            ),
            cellrune::ValidationErrorCode::TableHeaderRequiresText,
        ),
        (
            WorkbookChange::set_cell_value(sheet, address("C1"), CellValue::Text("HP2".into())),
            cellrune::ValidationErrorCode::DuplicateTableColumnName,
        ),
    ] {
        let error = draft
            .apply_changes(EditBatch::new([change]))
            .expect_err("invalid header write");
        assert_eq!(error.code(), expected);
    }
}

#[test]
fn annotated_text_in_a_table_header_cell_renames_its_column() {
    let address = CellAddress::from_a1("C1").expect("valid address");
    let sheet = SheetId::new(1).expect("sheet");
    let document =
        open_xlsx_document_bytes(&generated_totals_table_fixture(), OpenOptions::default())
            .expect("totals table input");
    let mut draft = WorkbookDraft::from_document(&document);
    // The second write replaces annotated header text, which a plain value write refuses.
    for (text, reading) in [("体力", "たいりょく"), ("健康", "けんこう")] {
        draft
            .set_annotated_text(
                sheet,
                address,
                text,
                vec![
                    cellrune::PhoneticRun::new(
                        cellrune::PhoneticTextRange::new(0, 2).expect("range"),
                        reading,
                    )
                    .expect("run"),
                ],
                cellrune::PhoneticWriteOptions::show(),
            )
            .expect("annotated header text");
    }

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("renamed table output");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen output");
    let workbook = reopened.workbook();
    assert_eq!(workbook.sheets()[0].tables()[0].columns()[2].name(), "健康");
    assert_eq!(
        totals_table_cell(workbook, "C1"),
        Some(CellContent::Literal(CellValue::Text("健康".into())))
    );
    assert_totals_formula(workbook, "D2", "tblChar[[#This Row],[健康]]*2");
    assert_totals_formula(workbook, "C5", "SUBTOTAL(109,tblChar[健康])");
    assert_eq!(number_at(workbook, "C5"), 60.0);
    assert_eq!(
        reopened
            .presentation()
            .cell_phonetics(sheet, address)
            .expect("reopened phonetics")
            .runs()[0]
            .text(),
        "けんこう"
    );
}

#[test]
fn growing_a_totals_table_twice_clears_the_totals_row_the_draft_wrote() {
    let resize = |last_row| {
        WorkbookChange::resize_table_rows(
            TableId::new(1).expect("table"),
            Row::new(2).expect("first data row"),
            Row::new(last_row).expect("last data row"),
        )
        .expect("valid resize")
    };
    // The first resize writes a totals row at row 6, which the source never had. The second
    // turns row 6 into a data row and clears those cells again.
    let (workbook, _) = resize_totals_table(vec![resize(5), resize(6)]);
    let address = |a1: &str| CellAddress::from_a1(a1).expect("valid address");
    assert_eq!(totals_table_cell(&workbook, "A6"), None);
    assert_eq!(
        totals_table_cell(&workbook, "A7"),
        Some(CellContent::Literal(CellValue::Text("Total".into())))
    );
    assert_totals_formula(&workbook, "C7", "SUBTOTAL(109,tblChar[hp])");
    assert!(workbook.sheets()[0].cell(address("C6")).is_none());
}

#[test]
fn shrinking_a_totals_table_names_its_table_in_the_new_totals_row() {
    let sheet = SheetId::new(1).expect("sheet");
    let address = |a1: &str| CellAddress::from_a1(a1).expect("valid address");
    let mut changes = ["A4", "B4", "C4", "D4"]
        .into_iter()
        .map(|cell| WorkbookChange::clear_cell(sheet, address(cell)))
        .collect::<Vec<_>>();
    changes.push(
        WorkbookChange::resize_table_rows(
            TableId::new(1).expect("table"),
            Row::new(2).expect("first data row"),
            Row::new(3).expect("last data row"),
        )
        .expect("valid resize"),
    );
    let (workbook, _) = resize_totals_table(changes);
    assert_totals_formula(&workbook, "C4", "SUBTOTAL(109,tblChar[hp])");
    assert_totals_formula(&workbook, "D4", "SUBTOTAL(109,tblChar[hp2])");
    assert_eq!(number_at(&workbook, "C4"), 30.0);
    assert_eq!(number_at(&workbook, "D4"), 60.0);
    // Resizing never deletes cells outside the new range, so the old totals row stays below.
    assert_eq!(
        totals_table_cell(&workbook, "A5"),
        Some(CellContent::Literal(CellValue::Text("Total".into())))
    );
}
