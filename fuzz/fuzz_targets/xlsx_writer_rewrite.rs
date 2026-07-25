#![no_main]

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CalculationSnapshot, CellAddress,
    CellContent, CellValue, FiniteNumber, FormulaText, OpenOptions, RecalculationWriteOptions,
    SheetId, SheetName, WorkbookDraft, WorkbookSnapshot, calculate_workbook,
    open_xlsx_document_bytes, write_preserved_xlsx_bytes, write_recalculated_xlsx_bytes,
    write_xlsx_draft_bytes,
};
use libfuzzer_sys::fuzz_target;

const MAX_GENERATED_CELLS: usize = 32;
const MAX_TEXT_CHARACTERS: usize = 256;

fuzz_target!(|data: &[u8]| {
    exercise_arbitrary_document(data);
    exercise_generated_document(data);
});

fn exercise_arbitrary_document(data: &[u8]) {
    let Ok(document) = open_xlsx_document_bytes(data, OpenOptions::default()) else {
        return;
    };
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let _ = write_preserved_xlsx_bytes(&document, cellrune::WriteOptions::default());
    let _ = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    );
}

fn exercise_generated_document(data: &[u8]) {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();

    for (index, chunk) in data.chunks(8).take(MAX_GENERATED_CELLS).enumerate() {
        let row = u32::try_from(index + 1).expect("bounded generated row");
        let address = CellAddress::from_indices(row, 1).expect("bounded generated address");
        let content = match chunk.first().copied().unwrap_or_default() % 4 {
            0 => CellValue::Number(
                FiniteNumber::new(f64::from(chunk.get(1).copied().unwrap_or_default()))
                    .expect("byte-derived number is finite"),
            ),
            1 => CellValue::Text(
                String::from_utf8_lossy(&chunk[1.min(chunk.len())..])
                    .chars()
                    .filter(|character| is_xml_10_character(*character))
                    .take(MAX_TEXT_CHARACTERS)
                    .collect(),
            ),
            2 => CellValue::Logical(chunk.get(1).is_some_and(|value| value & 1 == 1)),
            _ => CellValue::Blank,
        };
        draft
            .set_cell_value(sheet_id, address, content)
            .expect("bounded generated edit");

        if chunk.get(2).is_some_and(|value| value & 1 == 1) {
            let formula_address =
                CellAddress::from_indices(row, 2).expect("bounded generated formula address");
            let formula =
                FormulaText::from_xlsx(&format!("A{row}+1")).expect("bounded generated formula");
            draft
                .set_cell_formula(sheet_id, formula_address, formula)
                .expect("bounded generated formula edit");
        }
    }

    let referenced_sheet = draft
        .add_sheet(SheetName::new("Data Input").expect("constant sheet name"))
        .expect("unique generated sheet");
    draft
        .set_cell_value(
            referenced_sheet,
            CellAddress::from_indices(1, 1).expect("constant cross-sheet address"),
            CellValue::Number(FiniteNumber::new(2.0).expect("constant finite number")),
        )
        .expect("bounded cross-sheet edit");
    draft
        .set_cell_formula(
            sheet_id,
            CellAddress::from_indices(1, 3).expect("constant cross-sheet formula address"),
            FormulaText::from_xlsx("'Data Input'!A1+1")
                .expect("constant quoted cross-sheet formula"),
        )
        .expect("bounded cross-sheet formula edit");
    let renamed = if data.first().is_some_and(|value| value & 1 == 1) {
        "Renamed Data"
    } else {
        "O'Brien"
    };
    draft
        .rename_sheet(
            referenced_sheet,
            SheetName::new(renamed).expect("constant renamed sheet"),
        )
        .expect("bounded rename and formula rewrite");

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    assert_renamed_formula_invariant(draft.workbook(), &calculation, sheet_id, renamed);
    let first = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("generated draft must write");
    let document = open_xlsx_document_bytes(first.bytes(), OpenOptions::default())
        .expect("generated draft output must reopen");
    let reopened_calculation =
        calculate_workbook(document.workbook(), CalculationOptions::default());
    assert_renamed_formula_invariant(
        document.workbook(),
        &reopened_calculation,
        sheet_id,
        renamed,
    );

    let mut rewritten = WorkbookDraft::from_document(&document);
    let rewrite_sheet = rewritten.workbook().sheets()[0].id();
    let value = data.last().copied().map_or(0.0, |byte| f64::from(byte));
    rewritten
        .set_cell_value(
            rewrite_sheet,
            CellAddress::from_indices(1, 1).expect("constant rewrite address"),
            CellValue::Number(FiniteNumber::new(value).expect("byte-derived number is finite")),
        )
        .expect("bounded rewrite edit");
    let recalculation = calculate_workbook(rewritten.workbook(), CalculationOptions::default());
    assert_renamed_formula_invariant(rewritten.workbook(), &recalculation, rewrite_sheet, renamed);
    let second = write_xlsx_draft_bytes(
        &rewritten,
        &recalculation,
        RecalculationWriteOptions::default(),
    )
    .expect("rewritten generated draft must write");
    let final_document = open_xlsx_document_bytes(second.bytes(), OpenOptions::default())
        .expect("rewritten generated draft output must reopen");
    let final_calculation =
        calculate_workbook(final_document.workbook(), CalculationOptions::default());
    assert_renamed_formula_invariant(
        final_document.workbook(),
        &final_calculation,
        rewrite_sheet,
        renamed,
    );
}

const fn is_xml_10_character(character: char) -> bool {
    matches!(
        character as u32,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
}

fn assert_renamed_formula_invariant(
    workbook: &WorkbookSnapshot,
    calculation: &CalculationSnapshot,
    formula_sheet: SheetId,
    renamed: &str,
) {
    let formula_address =
        CellAddress::from_indices(1, 3).expect("constant cross-sheet formula address");
    let formula = workbook
        .sheet_by_id(formula_sheet)
        .and_then(|sheet| sheet.cell(formula_address))
        .expect("renamed cross-sheet formula cell must remain present");
    let CellContent::Formula(formula) = formula.content() else {
        panic!("renamed cross-sheet formula cell must remain a formula");
    };
    let expected_formula = format!("'{}'!A1+1", renamed.replace('\'', "''"));
    assert_eq!(
        formula.text().map(FormulaText::as_str),
        Some(expected_formula.as_str()),
        "renamed cross-sheet formula text must preserve the new sheet reference",
    );
    assert!(
        workbook.sheet_by_name(renamed).is_some(),
        "renamed referenced sheet must remain present",
    );
    assert_eq!(
        calculation.cell(CalculationCellId::new(formula_sheet, formula_address)),
        Some(&CalculationCellResult::Value(CellValue::Number(
            FiniteNumber::new(3.0).expect("constant expected result is finite"),
        ))),
        "renamed cross-sheet formula must preserve its calculated result",
    );
}
