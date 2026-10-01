//! Heap cost of reading package parts, measured with a counting global allocator.
//!
//! The allocator counts every thread of this process, so each test holds one lock for its whole
//! body and this target contains nothing else.

use std::io::{Cursor, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};

use cellrune::{ReadOptions, XlsxErrorCode, inspect_package};
use system_alloc_stats::SystemWithStats;
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

#[global_allocator]
static ALLOCATOR: SystemWithStats = SystemWithStats;

static SERIAL: Mutex<()> = Mutex::new(());

const PART_BYTES: usize = 16 * 1024 * 1024;
const HEAP_SLACK_BYTES: usize = 1024 * 1024;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#;

const ROOT_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK_RELATIONSHIPS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

#[test]
fn under_declared_entry_stops_at_its_declared_size() {
    let _serial = serial();
    let declared = 4 * 1024;
    let mut archive = package(CompressionMethod::Deflated);
    rewrite_declared_sizes(&mut archive, "[Content_Types].xml", None, declared);

    let (result, heap) =
        measure(|| inspect_package(Cursor::new(archive.as_slice()), ReadOptions::default()));

    let error = result.expect_err("an entry that hides its real size must be rejected");
    assert_eq!(error.code(), XlsxErrorCode::DeclaredSizeMismatch, "{error}");
    assert_eq!(error.detail(), Some("declared 4096 bytes, read 4097 bytes"));
    // The entry really inflates to `PART_BYTES`, well within the default entry limit, so only
    // the declared size bounds what reading it may allocate.
    assert!(
        heap.peak_bytes < HEAP_SLACK_BYTES,
        "peak heap {} bytes for a {declared}-byte declaration",
        heap.peak_bytes
    );
}

#[test]
fn declared_entry_is_read_without_growing_its_buffer() {
    let _serial = serial();
    let archive = package(CompressionMethod::Stored);

    let (result, heap) =
        measure(|| inspect_package(Cursor::new(archive.as_slice()), ReadOptions::default()));

    result.expect("a truthfully declared package must be accepted");
    assert!(
        heap.peak_bytes >= PART_BYTES,
        "the whole part is buffered: peak heap {} bytes",
        heap.peak_bytes
    );
    assert!(
        heap.growth_bytes < HEAP_SLACK_BYTES,
        "reallocation grew the heap by {} bytes while reading a {PART_BYTES}-byte part",
        heap.growth_bytes
    );
}

#[test]
fn compressed_size_past_the_archive_end_is_rejected_before_reserving() {
    let _serial = serial();
    // The content-types part is the last entry, so claiming more compressed bytes than follow it
    // overlaps no other entry. Its declared size keeps the ratio within the default limit.
    let mut entries = package_entries(CONTENT_TYPES, CompressionMethod::Stored);
    entries.reverse();
    let mut archive = build_archive(&entries);
    let declared = 60 * 1024 * 1024;
    rewrite_declared_sizes(
        &mut archive,
        "[Content_Types].xml",
        Some(1024 * 1024),
        declared,
    );

    let (result, heap) =
        measure(|| inspect_package(Cursor::new(archive.as_slice()), ReadOptions::default()));

    let error = result.expect_err("compressed data past the archive end must be rejected");
    assert_eq!(error.code(), XlsxErrorCode::InvalidZip, "{error}");
    assert_eq!(
        error.detail(),
        Some("ZIP entry compressed size exceeds the archive length")
    );
    assert!(
        heap.peak_bytes < HEAP_SLACK_BYTES,
        "peak heap {} bytes for a {}-byte archive declaring {declared} bytes",
        heap.peak_bytes,
        archive.len()
    );
}

struct HeapUse {
    peak_bytes: usize,
    growth_bytes: usize,
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
        growth_bytes: ALLOCATOR.realloc_growth_sum(),
    };
    (result, heap)
}

/// Builds a minimal package whose content-types part is padded to `PART_BYTES`.
///
/// The padding is many short comments, so parsing the part never holds more than one of them.
fn package(content_types_compression: CompressionMethod) -> Vec<u8> {
    const COMMENT: &str = "<!--pad-->";
    let padding = PART_BYTES - CONTENT_TYPES.len();
    let content_types = format!(
        "{CONTENT_TYPES}{}{}",
        COMMENT.repeat(padding / COMMENT.len()),
        "\n".repeat(padding % COMMENT.len())
    );
    assert_eq!(content_types.len(), PART_BYTES);
    build_archive(&package_entries(&content_types, content_types_compression))
}

fn package_entries(
    content_types: &str,
    content_types_compression: CompressionMethod,
) -> [(&str, &str, CompressionMethod); 5] {
    [
        (
            "[Content_Types].xml",
            content_types,
            content_types_compression,
        ),
        ("_rels/.rels", ROOT_RELATIONSHIPS, CompressionMethod::Stored),
        ("xl/workbook.xml", "<workbook/>", CompressionMethod::Stored),
        (
            "xl/_rels/workbook.xml.rels",
            WORKBOOK_RELATIONSHIPS,
            CompressionMethod::Stored,
        ),
        (
            "xl/worksheets/sheet1.xml",
            "<worksheet/>",
            CompressionMethod::Stored,
        ),
    ]
}

fn build_archive(entries: &[(&str, &str, CompressionMethod)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for &(name, contents, compression) in entries {
        writer
            .start_file(
                name,
                SimpleFileOptions::default().compression_method(compression),
            )
            .expect("fixture entry must start");
        writer
            .write_all(contents.as_bytes())
            .expect("fixture entry must write");
    }
    writer
        .finish()
        .expect("fixture ZIP must finish")
        .into_inner()
}

/// Rewrites the declared sizes of one entry in both the local file header and the central
/// directory, leaving the stored data and its CRC intact. `compressed` is kept when `None`.
fn rewrite_declared_sizes(
    archive: &mut [u8],
    name: &str,
    compressed: Option<u32>,
    uncompressed: u32,
) {
    const CENTRAL_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const LOCAL_SIGNATURE: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
    const CENTRAL_HEADER_BYTES: usize = 46;

    let mut index = 0;
    while index + CENTRAL_HEADER_BYTES <= archive.len() {
        if archive[index..index + 4] != CENTRAL_SIGNATURE {
            index += 1;
            continue;
        }
        let name_length = u16::from_le_bytes([archive[index + 28], archive[index + 29]]) as usize;
        let name_start = index + CENTRAL_HEADER_BYTES;
        if name_start + name_length > archive.len()
            || archive[name_start..name_start + name_length] != *name.as_bytes()
        {
            index += 1;
            continue;
        }
        let local_offset = u32::from_le_bytes([
            archive[index + 42],
            archive[index + 43],
            archive[index + 44],
            archive[index + 45],
        ]) as usize;
        assert_eq!(
            archive[local_offset..local_offset + 4],
            LOCAL_SIGNATURE,
            "central directory must point at a local file header"
        );
        if let Some(compressed) = compressed {
            archive[index + 20..index + 24].copy_from_slice(&compressed.to_le_bytes());
            archive[local_offset + 18..local_offset + 22]
                .copy_from_slice(&compressed.to_le_bytes());
        }
        archive[index + 24..index + 28].copy_from_slice(&uncompressed.to_le_bytes());
        archive[local_offset + 22..local_offset + 26].copy_from_slice(&uncompressed.to_le_bytes());
        return;
    }
    panic!("fixture entry must exist: {name}");
}
