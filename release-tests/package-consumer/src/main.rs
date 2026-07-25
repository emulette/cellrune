use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CancellationToken, CellAddress,
    CellContent, CellRange, CellValue, EditBatch, FiniteNumber, FormulaMetadata, FormulaText,
    MaterializedResultOrigin, OpenOptions, RecalculationMode, RecalculationWriteOptions,
    SavedResult, WorkbookCalculationSession, WorkbookChange, WorkbookDraft, calculate_workbook,
    open_xlsx_document_bytes, scan_formula_capabilities, scan_function_usage,
    supported_function_catalog, write_recalculated_xlsx_bytes, write_xlsx_draft_bytes,
};

const READ_FAILURE: &str = "packaged crate must read the generated XLSX";
const WRITE_FAILURE: &str = "packaged crate must write the recalculated XLSX";
const REOPEN_FAILURE: &str = "packaged crate output must reopen";
const SHEET_FAILURE: &str = "generated workbook must contain Sheet1";
const ADDRESS_FAILURE: &str = "B1 must be a valid cell address";
const RESULT_FAILURE: &str = "packaged crate must calculate Sheet1!B1";
const ZIP_SIZE_FAILURE: &str = "test ZIP must fit in u32";
const ZIP_ENTRY_SIZE_FAILURE: &str = "test ZIP entry must fit in u32";
const ZIP_ENTRY_NAME_FAILURE: &str = "test ZIP entry name must fit in u16";
const ZIP_ENTRY_COUNT_FAILURE: &str = "test ZIP entry count must fit in u16";
const EXPECTED_RESULT: f64 = 5.0;

struct ZipEntry<'a> {
    name: &'a str,
    contents: &'a str,
    crc32: u32,
    offset: u32,
}

fn main() {
    let bytes = minimal_workbook();
    let document = open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect(READ_FAILURE);
    let workbook = document.workbook();

    assert_eq!(workbook.sheets().len(), 1);
    let capability = scan_formula_capabilities(&workbook);
    assert_eq!(capability.formula_count(), 1);
    assert!(capability.is_supported());
    let usage = scan_function_usage(&workbook);
    assert_eq!(usage.entries().len(), 1);
    assert_eq!(usage.entries()[0].name(), "SUM");
    assert!(
        supported_function_catalog()
            .iter()
            .any(|entry| entry.name() == "FILTER" && entry.returns_array())
    );

    let sheet = workbook.sheet_by_name("Sheet1").expect(SHEET_FAILURE);
    let address = CellAddress::from_a1("B1").expect(ADDRESS_FAILURE);
    let cell_id = CalculationCellId::new(sheet.id(), address);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) = calculation.cell(cell_id)
    else {
        panic!("{RESULT_FAILURE}");
    };

    assert_eq!(actual.get(), EXPECTED_RESULT);
    let output = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    )
    .expect(WRITE_FAILURE);
    assert!(output.report().is_complete());
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect(REOPEN_FAILURE);
    let output_cell = reopened
        .workbook()
        .sheet_by_name("Sheet1")
        .expect(SHEET_FAILURE)
        .cell(address)
        .expect(RESULT_FAILURE);
    let CellContent::Formula(output_formula) = output_cell.content() else {
        panic!("{RESULT_FAILURE}");
    };
    assert_eq!(
        output_formula.saved_result(),
        &SavedResult::Present(CellValue::Number(*actual))
    );

    let mut interactive = WorkbookCalculationSession::new(WorkbookDraft::from_document(&document));
    interactive
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect(RESULT_FAILURE);
    let receipt = interactive
        .apply_changes(
            interactive.workbook().semantic_revision(),
            EditBatch::new([WorkbookChange::set_cell_value(
                sheet.id(),
                CellAddress::from_a1("A1").expect(ADDRESS_FAILURE),
                CellValue::Number(FiniteNumber::new(4.0).expect(RESULT_FAILURE)),
            )]),
        )
        .expect(WRITE_FAILURE);
    let delta = interactive
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect(RESULT_FAILURE);
    assert_eq!(delta.result_revision(), receipt.result_revision());
    assert_eq!(delta.evaluated_count(), 1);
    let Some(CalculationCellResult::Value(CellValue::Number(interactive_value))) = interactive
        .calculation()
        .and_then(|calculation| calculation.cell(cell_id))
    else {
        panic!("{RESULT_FAILURE}");
    };
    assert_eq!(interactive_value.get(), 7.0);

    let mut draft = WorkbookDraft::new();
    let draft_sheet = draft.workbook().sheets()[0].id();
    draft
        .set_cell_value(
            draft_sheet,
            CellAddress::from_a1("A1").expect(ADDRESS_FAILURE),
            CellValue::Number(FiniteNumber::new(2.0).expect(RESULT_FAILURE)),
        )
        .expect(WRITE_FAILURE);
    draft
        .set_cell_formula(
            draft_sheet,
            address,
            FormulaText::from_xlsx("A1+3").expect(RESULT_FAILURE),
        )
        .expect(WRITE_FAILURE);
    let dynamic_anchor = CellAddress::from_a1("D1").expect(ADDRESS_FAILURE);
    draft
        .set_cell_dynamic_formula(
            draft_sheet,
            dynamic_anchor,
            FormulaText::from_xlsx("FILTER({1,2;3,4},{1;0})").expect(RESULT_FAILURE),
            None,
        )
        .expect(WRITE_FAILURE);
    let draft_calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let dynamic_range = CellRange::new(
        dynamic_anchor,
        CellAddress::from_a1("E1").expect(ADDRESS_FAILURE),
    )
    .expect(RESULT_FAILURE);
    let dynamic_follower = CalculationCellId::new(
        draft_sheet,
        CellAddress::from_a1("E1").expect(ADDRESS_FAILURE),
    );
    let materialized = draft_calculation
        .materialized_cell(dynamic_follower)
        .expect(RESULT_FAILURE);
    assert_eq!(
        materialized.origin(),
        MaterializedResultOrigin::DynamicSpill {
            anchor: CalculationCellId::new(draft_sheet, dynamic_anchor),
            range: dynamic_range,
        }
    );
    assert_eq!(
        materialized.result(),
        &CalculationCellResult::Value(CellValue::Number(
            FiniteNumber::new(2.0).expect(RESULT_FAILURE)
        ))
    );
    let draft_output = write_xlsx_draft_bytes(
        &draft,
        &draft_calculation,
        RecalculationWriteOptions::default(),
    )
    .expect(WRITE_FAILURE);
    let draft_reopened = open_xlsx_document_bytes(draft_output.bytes(), OpenOptions::default())
        .expect(REOPEN_FAILURE);
    let CellContent::Formula(draft_formula) = draft_reopened
        .workbook()
        .sheet_by_name("Sheet1")
        .expect(SHEET_FAILURE)
        .cell(address)
        .expect(RESULT_FAILURE)
        .content()
    else {
        panic!("{RESULT_FAILURE}");
    };
    assert_eq!(
        draft_formula.saved_result(),
        &SavedResult::Present(CellValue::Number(
            FiniteNumber::new(EXPECTED_RESULT).expect(RESULT_FAILURE)
        ))
    );
    let dynamic_sheet = draft_reopened
        .workbook()
        .sheet_by_name("Sheet1")
        .expect(SHEET_FAILURE);
    let CellContent::Formula(dynamic_formula) = dynamic_sheet
        .cell(dynamic_anchor)
        .expect(RESULT_FAILURE)
        .content()
    else {
        panic!("{RESULT_FAILURE}");
    };
    assert_eq!(
        dynamic_formula.metadata(),
        &FormulaMetadata::DynamicArray {
            range: None,
            always_calculate: false,
        }
    );
    assert_eq!(
        dynamic_sheet
            .cell(CellAddress::from_a1("E1").expect(ADDRESS_FAILURE))
            .expect(RESULT_FAILURE)
            .content(),
        &CellContent::Literal(CellValue::Number(
            FiniteNumber::new(2.0).expect(RESULT_FAILURE)
        ))
    );
    println!(
        "fresh package consumer: open/recalculate/write, interactive delta, function usage, and dynamic draft/reopen"
    );
}

fn minimal_workbook() -> Vec<u8> {
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
