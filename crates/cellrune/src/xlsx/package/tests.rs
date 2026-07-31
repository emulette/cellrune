use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use super::super::{ReadLimits, ReadOptions, ReadOptionsError, XlsxErrorCode};
use super::inspect_package;

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
fn minimal_package_is_discovered_from_relationships() {
    let summary = inspect_package(
        Cursor::new(build_archive(minimal_entries())),
        ReadOptions::default(),
    )
    .expect("minimal package");

    assert_eq!(summary.workbook_part().as_str(), "xl/workbook.xml");
    assert_eq!(summary.worksheet_parts().len(), 1);
    assert_eq!(
        summary.worksheet_parts()[0].as_str(),
        "xl/worksheets/sheet1.xml"
    );
    assert_eq!(summary.entry_count(), 5);
    assert_eq!(summary.external_relationship_count(), 0);
}

#[test]
fn maximum_entry_limit_does_not_overflow_required_part_reads() {
    let limits = ReadLimits::default()
        .with_max_entry_uncompressed_bytes(u64::MAX)
        .expect("nonzero maximum limit");

    let summary = inspect_package(
        Cursor::new(build_archive(minimal_entries())),
        ReadOptions::new(limits),
    )
    .expect("maximum entry limit");

    assert_eq!(summary.entry_count(), 5);
}

#[test]
fn package_absolute_workbook_target_is_supported() {
    let relationships =
        ROOT_RELATIONSHIPS.replace("Target=\"xl/workbook.xml\"", "Target=\"/xl/workbook.xml\"");
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "_rels/.rels", &relationships);

    let summary = inspect_package(Cursor::new(build_archive(entries)), ReadOptions::default())
        .expect("package-absolute target");
    assert_eq!(summary.workbook_part().as_str(), "xl/workbook.xml");
}

#[test]
fn relationship_parent_segment_can_resolve_within_package_root() {
    let content_types = CONTENT_TYPES.replace("/xl/workbook.xml", "/xl/subdirectory/workbook.xml");
    let root_relationships =
        ROOT_RELATIONSHIPS.replace("xl/workbook.xml", "xl/subdirectory/workbook.xml");
    let workbook_relationships =
        WORKBOOK_RELATIONSHIPS.replace("worksheets/sheet1.xml", "../worksheets/sheet1.xml");
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "[Content_Types].xml", &content_types);
    replace_entry(&mut entries, "_rels/.rels", &root_relationships);
    rename_entry(
        &mut entries,
        "xl/workbook.xml",
        "xl/subdirectory/workbook.xml",
    );
    rename_entry(
        &mut entries,
        "xl/_rels/workbook.xml.rels",
        "xl/subdirectory/_rels/workbook.xml.rels",
    );
    replace_entry(
        &mut entries,
        "xl/subdirectory/_rels/workbook.xml.rels",
        &workbook_relationships,
    );

    let summary = inspect_package(Cursor::new(build_archive(entries)), ReadOptions::default())
        .expect("safe parent target");
    assert_eq!(
        summary.workbook_part().as_str(),
        "xl/subdirectory/workbook.xml"
    );
    assert_eq!(
        summary.worksheet_parts()[0].as_str(),
        "xl/worksheets/sheet1.xml"
    );
}

#[test]
fn external_relationship_is_recorded_and_never_opened() {
    let relationships = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
      <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
      <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/never-open" TargetMode="External"/>
    </Relationships>"#;
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "xl/_rels/workbook.xml.rels", relationships);

    let summary = inspect_package(Cursor::new(build_archive(entries)), ReadOptions::default())
        .expect("package with external metadata");
    assert_eq!(summary.external_relationship_count(), 1);
}

#[test]
fn external_workbook_relationship_is_rejected() {
    let relationships = ROOT_RELATIONSHIPS.replace(
        "Target=\"xl/workbook.xml\"",
        "Target=\"https://example.invalid/book.xlsx\" TargetMode=\"External\"",
    );
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "_rels/.rels", &relationships);

    assert_error(
        entries,
        ReadLimits::default(),
        XlsxErrorCode::ExternalWorkbookRelationship,
    );
}

#[test]
fn missing_required_part_has_a_stable_source_linked_error() {
    let mut entries = minimal_entries();
    entries.retain(|entry| entry.name != "xl/workbook.xml");
    let error = inspect_package(Cursor::new(build_archive(entries)), ReadOptions::default())
        .expect_err("missing workbook must fail");

    assert_eq!(error.code(), XlsxErrorCode::MissingPart);
    assert_eq!(
        error.source_id().map(|source| source.as_str()),
        Some("xl/workbook.xml")
    );
}

#[test]
fn duplicate_workbook_relationship_is_rejected() {
    let relationships = ROOT_RELATIONSHIPS.replace(
        "</Relationships>",
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    );
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "_rels/.rels", &relationships);

    assert_error(
        entries,
        ReadLimits::default(),
        XlsxErrorCode::DuplicateWorkbookRelationship,
    );
}

#[test]
fn relationship_cannot_escape_the_package_root() {
    let relationships = ROOT_RELATIONSHIPS.replace("xl/workbook.xml", "%2e%2e/%2e%2e/escape.xml");
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "_rels/.rels", &relationships);

    assert_error(
        entries,
        ReadLimits::default(),
        XlsxErrorCode::InvalidRelationshipTarget,
    );
}

#[test]
fn duplicate_normalized_part_name_is_rejected() {
    let mut entries = minimal_entries();
    entries.push(FixtureEntry::stored("xl/./workbook.xml", "<workbook/>"));
    assert_error(entries, ReadLimits::default(), XlsxErrorCode::DuplicatePart);
}

#[test]
fn entry_count_budget_is_enforced_before_xml_parsing() {
    let limits = ReadLimits::default()
        .with_max_entries(4)
        .expect("non-zero limit");
    assert_error(minimal_entries(), limits, XlsxErrorCode::TooManyEntries);
}

#[test]
fn archive_byte_budget_is_enforced_before_zip_parsing() {
    let archive = build_archive(minimal_entries());
    let limits = ReadLimits::default()
        .with_max_archive_bytes(archive.len() as u64 - 1)
        .expect("non-zero limit");
    let error = inspect_package(Cursor::new(archive), ReadOptions::new(limits))
        .expect_err("oversized archive must fail");
    assert_eq!(error.code(), XlsxErrorCode::ArchiveTooLarge);
}

#[test]
fn malformed_zip_has_a_stable_error_code() {
    let error = inspect_package(
        Cursor::new(b"not a ZIP archive".as_slice()),
        ReadOptions::default(),
    )
    .expect_err("malformed ZIP must fail");
    assert_eq!(error.code(), XlsxErrorCode::InvalidZip);
}

#[test]
fn entry_size_budget_is_enforced_before_decompression() {
    let limits = ReadLimits::default()
        .with_max_entry_uncompressed_bytes(32)
        .expect("non-zero limit");
    assert_error(minimal_entries(), limits, XlsxErrorCode::EntryTooLarge);
}

#[test]
fn total_size_budget_is_enforced_from_central_directory() {
    let limits = ReadLimits::default()
        .with_max_total_uncompressed_bytes(128)
        .expect("non-zero limit");
    assert_error(
        minimal_entries(),
        limits,
        XlsxErrorCode::TotalUncompressedTooLarge,
    );
}

#[test]
fn under_declared_entry_size_is_rejected_when_the_part_is_read() {
    let padded = format!("{CONTENT_TYPES}<!--{}-->", "p".repeat(1024 * 1024));
    let mut entries = minimal_entries();
    entries[0] = FixtureEntry::deflated("[Content_Types].xml", padded.into_bytes());
    let mut archive = build_archive(entries);

    under_declare_uncompressed_size(&mut archive, "[Content_Types].xml", 1);

    // Every central-directory budget accepts the archive because it declares a
    // single byte. The mismatch is only observable once the entry is inflated,
    // so a `DeclaredSizeMismatch` here proves the read path, not the index path,
    // is what stops the amplification.
    let error = inspect_package(Cursor::new(archive), ReadOptions::default())
        .expect_err("an entry that hides its real size must be rejected");
    assert_eq!(error.code(), XlsxErrorCode::DeclaredSizeMismatch, "{error}");
}

#[test]
fn compression_ratio_budget_blocks_highly_compressible_entries() {
    let mut entries = minimal_entries();
    entries.push(FixtureEntry::deflated("large.bin", vec![0; 64 * 1024]));
    let limits = ReadLimits::default()
        .with_max_compression_ratio(2)
        .expect("non-zero limit");
    assert_error(entries, limits, XlsxErrorCode::CompressionRatioExceeded);
}

#[test]
fn xml_depth_and_attribute_budgets_are_enforced() {
    let depth_limits = ReadLimits::default()
        .with_max_xml_depth(1)
        .expect("non-zero limit");
    assert_error(
        minimal_entries(),
        depth_limits,
        XlsxErrorCode::XmlDepthExceeded,
    );

    let attribute_limits = ReadLimits::default()
        .with_max_xml_attributes(1)
        .expect("non-zero limit");
    assert_error(
        minimal_entries(),
        attribute_limits,
        XlsxErrorCode::XmlAttributesExceeded,
    );
}

#[test]
fn document_type_declaration_is_rejected() {
    let content_types = CONTENT_TYPES.replace(
        "<Types",
        "<!DOCTYPE Types [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]><Types",
    );
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "[Content_Types].xml", &content_types);
    assert_error(
        entries,
        ReadLimits::default(),
        XlsxErrorCode::ForbiddenXmlConstruct,
    );
}

#[test]
fn package_xml_requires_the_opc_namespace() {
    let content_types = CONTENT_TYPES.replace(
        "http://schemas.openxmlformats.org/package/2006/content-types",
        "https://example.invalid/not-opc",
    );
    let mut entries = minimal_entries();
    replace_entry(&mut entries, "[Content_Types].xml", &content_types);
    assert_error(entries, ReadLimits::default(), XlsxErrorCode::InvalidXml);
}

#[test]
fn read_limits_reject_zero_values() {
    assert_eq!(
        ReadLimits::default().with_max_archive_bytes(0),
        Err(ReadOptionsError::ZeroLimit {
            name: "max_archive_bytes"
        })
    );
    assert_eq!(
        ReadLimits::default().with_max_compression_ratio(0),
        Err(ReadOptionsError::ZeroLimit {
            name: "max_compression_ratio"
        })
    );
    assert_eq!(
        ReadLimits::default().with_max_formula_bytes(0),
        Err(ReadOptionsError::ZeroLimit {
            name: "max_formula_bytes"
        })
    );
    assert_eq!(
        ReadLimits::default().with_max_total_formula_bytes(0),
        Err(ReadOptionsError::ZeroLimit {
            name: "max_total_formula_bytes"
        })
    );
    assert_eq!(
        ReadLimits::default().with_max_merged_ranges(0),
        Err(ReadOptionsError::ZeroLimit {
            name: "max_merged_ranges"
        })
    );
    type LimitSetter = fn(ReadLimits, u64) -> Result<ReadLimits, ReadOptionsError>;
    let table_setters: [(LimitSetter, &str); 5] = [
        (ReadLimits::with_max_tables, "max_tables"),
        (ReadLimits::with_max_table_columns, "max_table_columns"),
        (
            ReadLimits::with_max_table_name_bytes,
            "max_table_name_bytes",
        ),
        (
            ReadLimits::with_max_table_filter_items,
            "max_table_filter_items",
        ),
        (
            ReadLimits::with_max_table_filter_text_bytes,
            "max_table_filter_text_bytes",
        ),
    ];
    for (setter, name) in table_setters {
        assert_eq!(
            setter(ReadLimits::default(), 0),
            Err(ReadOptionsError::ZeroLimit { name })
        );
    }
    let phonetic_setters = [
        ReadLimits::with_max_phonetic_runs_per_item,
        ReadLimits::with_max_total_phonetic_runs,
        ReadLimits::with_max_annotated_cells,
        ReadLimits::with_max_phonetic_text_bytes,
        ReadLimits::with_max_total_phonetic_text_bytes,
    ];
    for setter in phonetic_setters {
        assert!(matches!(
            setter(ReadLimits::default(), 0),
            Err(ReadOptionsError::ZeroLimit { .. })
        ));
    }
}

fn minimal_entries() -> Vec<FixtureEntry> {
    vec![
        FixtureEntry::stored("[Content_Types].xml", CONTENT_TYPES),
        FixtureEntry::stored("_rels/.rels", ROOT_RELATIONSHIPS),
        FixtureEntry::stored("xl/workbook.xml", "<workbook/>"),
        FixtureEntry::stored("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONSHIPS),
        FixtureEntry::stored("xl/worksheets/sheet1.xml", "<worksheet/>"),
    ]
}

fn replace_entry(entries: &mut [FixtureEntry], name: &str, contents: &str) {
    let entry = entries
        .iter_mut()
        .find(|entry| entry.name == name)
        .expect("fixture entry must exist");
    entry.contents = contents.as_bytes().to_vec();
}

fn rename_entry(entries: &mut [FixtureEntry], old_name: &str, new_name: &str) {
    let entry = entries
        .iter_mut()
        .find(|entry| entry.name == old_name)
        .expect("fixture entry must exist");
    entry.name = new_name.to_owned();
}

fn assert_error(entries: Vec<FixtureEntry>, limits: ReadLimits, expected: XlsxErrorCode) {
    let error = inspect_package(
        Cursor::new(build_archive(entries)),
        ReadOptions::new(limits),
    )
    .expect_err("fixture must be rejected");
    assert_eq!(error.code(), expected, "{error}");
}

/// Rewrites the declared uncompressed size of one entry in both the local file
/// header and the central directory, leaving the compressed data and its CRC
/// intact. A Deflated entry still inflates to its true length, which is how a
/// crafted archive hides its real cost from central-directory budgets.
fn under_declare_uncompressed_size(archive: &mut [u8], name: &str, declared: u32) {
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
        archive[index + 24..index + 28].copy_from_slice(&declared.to_le_bytes());
        archive[local_offset + 22..local_offset + 26].copy_from_slice(&declared.to_le_bytes());
        return;
    }
    panic!("fixture entry must exist: {name}");
}

fn build_archive(entries: Vec<FixtureEntry>) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    for entry in entries {
        let options = SimpleFileOptions::default().compression_method(entry.compression);
        writer
            .start_file(&entry.name, options)
            .expect("fixture entry must start");
        writer
            .write_all(&entry.contents)
            .expect("fixture entry must write");
    }
    writer
        .finish()
        .expect("fixture ZIP must finish")
        .into_inner()
}

struct FixtureEntry {
    name: String,
    contents: Vec<u8>,
    compression: CompressionMethod,
}

impl FixtureEntry {
    fn stored(name: &str, contents: impl AsRef<[u8]>) -> Self {
        Self {
            name: name.to_owned(),
            contents: contents.as_ref().to_vec(),
            compression: CompressionMethod::Stored,
        }
    }

    fn deflated(name: &str, contents: Vec<u8>) -> Self {
        Self {
            name: name.to_owned(),
            contents,
            compression: CompressionMethod::Deflated,
        }
    }
}
