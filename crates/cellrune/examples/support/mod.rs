const ZIP_SIZE_FAILURE: &str = "test ZIP must fit in u32";
const ZIP_ENTRY_SIZE_FAILURE: &str = "test ZIP entry must fit in u32";
const ZIP_ENTRY_NAME_FAILURE: &str = "test ZIP entry name must fit in u16";
const ZIP_ENTRY_COUNT_FAILURE: &str = "test ZIP entry count must fit in u16";

struct ZipEntry<'a> {
    name: &'a str,
    contents: &'a str,
    crc32: u32,
    offset: u32,
}

/// Builds a minimal in-memory XLSX with one sheet: `A1` holds the literal `2` and `B1` holds
/// `=SUM(A1,3)` with a saved result of `5`. Used by examples that need a workbook to read
/// without shipping a binary fixture file.
pub fn minimal_workbook_bytes() -> Vec<u8> {
    let sources = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Sheet1" sheetId="1" r:id="rId1"/>
  </sheets>
</workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1"><v>2</v></c>
      <c r="B1"><f>SUM(A1,3)</f><v>5</v></c>
    </row>
  </sheetData>
</worksheet>"#,
        ),
    ];

    stored_zip(&sources)
}

fn stored_zip(sources: &[(&str, &str)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut entries = Vec::with_capacity(sources.len());

    for &(name, contents) in sources {
        let name_bytes = name.as_bytes();
        let content_bytes = contents.as_bytes();
        let entry = ZipEntry {
            name,
            contents,
            crc32: crc32(content_bytes),
            offset: u32::try_from(archive.len()).expect(ZIP_SIZE_FAILURE),
        };

        write_u32(&mut archive, 0x0403_4b50);
        write_u16(&mut archive, 20);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u32(&mut archive, entry.crc32);
        let size = u32::try_from(content_bytes.len()).expect(ZIP_ENTRY_SIZE_FAILURE);
        write_u32(&mut archive, size);
        write_u32(&mut archive, size);
        write_u16(
            &mut archive,
            u16::try_from(name_bytes.len()).expect(ZIP_ENTRY_NAME_FAILURE),
        );
        write_u16(&mut archive, 0);
        archive.extend_from_slice(name_bytes);
        archive.extend_from_slice(content_bytes);
        entries.push(entry);
    }

    let central_offset = u32::try_from(archive.len()).expect(ZIP_SIZE_FAILURE);
    for entry in &entries {
        let name_bytes = entry.name.as_bytes();
        let content_bytes = entry.contents.as_bytes();
        write_u32(&mut archive, 0x0201_4b50);
        write_u16(&mut archive, 20);
        write_u16(&mut archive, 20);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u32(&mut archive, entry.crc32);
        let size = u32::try_from(content_bytes.len()).expect(ZIP_ENTRY_SIZE_FAILURE);
        write_u32(&mut archive, size);
        write_u32(&mut archive, size);
        write_u16(
            &mut archive,
            u16::try_from(name_bytes.len()).expect(ZIP_ENTRY_NAME_FAILURE),
        );
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u16(&mut archive, 0);
        write_u32(&mut archive, 0);
        write_u32(&mut archive, entry.offset);
        archive.extend_from_slice(name_bytes);
    }

    let central_size = u32::try_from(archive.len()).expect(ZIP_SIZE_FAILURE) - central_offset;
    let entry_count = u16::try_from(entries.len()).expect(ZIP_ENTRY_COUNT_FAILURE);
    write_u32(&mut archive, 0x0605_4b50);
    write_u16(&mut archive, 0);
    write_u16(&mut archive, 0);
    write_u16(&mut archive, entry_count);
    write_u16(&mut archive, entry_count);
    write_u32(&mut archive, central_size);
    write_u32(&mut archive, central_offset);
    write_u16(&mut archive, 0);

    archive
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn write_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
