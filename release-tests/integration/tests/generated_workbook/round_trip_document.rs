use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use cellrune::{
    CalculationOptions, OpenOptions, ReadLimits, ReadOptions, SheetId, WorkbookSourceKind,
    WriteLimits, WriteOptions, XlsxDocumentKind, XlsxErrorCode, XlsxWriteErrorCode,
    calculate_workbook, open_xlsx_document, open_xlsx_document_bytes, open_xlsx_document_path,
    read_xlsx_bytes, write_preserved_xlsx_bytes,
};
use sha2::{Digest, Sha256};
use zip::read::ZipArchive;

use crate::support::generated_xlsx::{
    ProducerProfile, TemporaryWorkbook, generated_workbook, generated_workbook_with_comment,
};

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
