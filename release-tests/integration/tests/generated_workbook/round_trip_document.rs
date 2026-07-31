use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationExecutionMode, CalculationOptions,
    CancellationToken, CellAddress, CellContent, CellValue, EditBatch, OpenOptions, ReadLimits,
    ReadOptions, RecalculationMode, SheetId, WorkbookCalculationSession, WorkbookChange,
    WorkbookDraft, WorkbookSourceKind, WriteLimits, WriteOptions, XlsxDocumentKind, XlsxErrorCode,
    XlsxWriteErrorCode, calculate_workbook, open_xlsx_document, open_xlsx_document_bytes,
    open_xlsx_document_path, read_xlsx_bytes, scan_formula_capabilities,
    write_preserved_xlsx_bytes,
};
use sha2::{Digest, Sha256};
use zip::read::ZipArchive;

use crate::support::generated_xlsx::{
    ProducerProfile, TemporaryWorkbook, generated_formula_fixture,
    generated_table_reference_fixture, generated_table_topology_fixture, generated_workbook,
    generated_workbook_with_comment,
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
            file.name().to_owned(),
            (file.compression(), file.crc32(), contents),
        );
        assert!(replaced.is_none(), "duplicate fixture entry");
    }
    manifest
}
