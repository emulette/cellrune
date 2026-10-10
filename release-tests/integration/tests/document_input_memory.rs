//! Heap cost of the archive bytes a document retains, measured with a counting global allocator.
//!
//! The allocator counts every thread of this process, so each test holds one lock for its whole
//! body and this target contains nothing else.

use std::io::{Cursor, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};

use cellrune::{
    OpenOptions, WorkbookDraft, open_xlsx_document, open_xlsx_document_bytes,
    open_xlsx_document_vec,
};
use sha2::{Digest, Sha256};
use system_alloc_stats::SystemWithStats;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

#[global_allocator]
static ALLOCATOR: SystemWithStats = SystemWithStats;

static SERIAL: Mutex<()> = Mutex::new(());

const PADDING_BYTES: usize = 16 * 1024 * 1024;
const HEAP_SLACK_BYTES: usize = 1024 * 1024;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="bin" ContentType="application/octet-stream"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

const WORKSHEET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#;

#[test]
fn owned_bytes_become_the_retained_archive_without_a_copy() {
    let _serial = serial();
    let archive = package();
    let expected = archive.clone();

    let (document, heap) = measure(|| open_xlsx_document_vec(archive, OpenOptions::default()));

    let document = document.expect("open owned bytes");
    assert!(
        heap.peak_bytes < HEAP_SLACK_BYTES,
        "peak heap {} bytes while opening an owned {}-byte archive",
        heap.peak_bytes,
        expected.len()
    );
    assert_eq!(document.input_bytes(), expected.as_slice());
}

#[test]
fn slice_and_reader_opens_hold_one_archive_copy() {
    let _serial = serial();
    let archive = package();

    let (document, heap) = measure(|| open_xlsx_document_bytes(&archive, OpenOptions::default()));
    document.expect("open borrowed bytes");
    assert!(
        heap.peak_bytes < archive.len() + HEAP_SLACK_BYTES,
        "peak heap {} bytes while opening a borrowed {}-byte archive",
        heap.peak_bytes,
        archive.len()
    );

    let reader = Cursor::new(archive.clone());
    let (document, heap) = measure(|| open_xlsx_document(reader, OpenOptions::default()));
    document.expect("open a reader");
    assert!(
        heap.peak_bytes < archive.len() + HEAP_SLACK_BYTES,
        "peak heap {} bytes while opening a {}-byte archive from a reader",
        heap.peak_bytes,
        archive.len()
    );
}

#[test]
fn retained_bytes_move_out_of_a_sole_owner_and_copy_out_of_a_shared_one() {
    let _serial = serial();
    let archive = package();

    let document =
        open_xlsx_document_vec(archive.clone(), OpenOptions::default()).expect("open archive");
    let input_hash = document.input_hash();
    let (returned, heap) = measure(|| document.into_input_bytes());
    assert!(
        heap.peak_bytes < HEAP_SLACK_BYTES,
        "peak heap {} bytes while returning the bytes of a sole owner",
        heap.peak_bytes
    );
    assert_eq!(returned, archive);
    assert_eq!(Sha256::digest(&returned).as_slice(), input_hash.as_bytes());

    let document =
        open_xlsx_document_vec(archive.clone(), OpenOptions::default()).expect("open archive");
    let draft = WorkbookDraft::from_document(&document);
    let (returned, heap) = measure(|| document.into_input_bytes());
    assert!(
        heap.peak_bytes >= archive.len(),
        "a draft still shares the bytes, so they are copied: peak heap {} bytes",
        heap.peak_bytes
    );
    assert_eq!(returned, archive);
    drop(draft);
}

struct HeapUse {
    peak_bytes: usize,
}

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, HeapUse) {
    ALLOCATOR.reset();
    let initial = ALLOCATOR.use_curr();
    let result = operation();
    let heap = HeapUse {
        peak_bytes: ALLOCATOR.use_max().saturating_sub(initial),
    };
    (result, heap)
}

/// Builds a minimal workbook carrying a stored padding part the reader preserves but never reads,
/// so the archive is large while opening it parses only small parts.
fn package() -> Vec<u8> {
    let padding = vec![0_u8; PADDING_BYTES];
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, contents) in [
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", ROOT_RELATIONSHIPS.as_bytes()),
        ("xl/workbook.xml", WORKBOOK.as_bytes()),
        (
            "xl/_rels/workbook.xml.rels",
            WORKBOOK_RELATIONSHIPS.as_bytes(),
        ),
        ("xl/worksheets/sheet1.xml", WORKSHEET.as_bytes()),
        ("xl/padding.bin", padding.as_slice()),
    ] {
        writer.start_file(name, options).expect("start part");
        writer.write_all(contents).expect("write part");
    }
    writer.finish().expect("finish archive").into_inner()
}
