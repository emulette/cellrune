use cellrune::{
    Alignment, Border, BorderEdge, BorderStyle, CalculationOptions, CellAddress, CellFormat,
    CellFormatEdit, CellValue, Color, Column, EditBatch, Fill, FillPattern, Font, FontUnderline,
    HorizontalAlignment, NumberFormatKind, OpenOptions, PatternFill, RecalculationWriteOptions,
    Row, SheetId, ValidationError, VerticalAlignment, WorkbookChange, WorkbookDraft, WriteOptions,
    calculate_workbook, open_xlsx_document_bytes, write_xlsx_draft_bytes,
    write_xlsx_draft_edits_bytes,
};

use super::recalculation_write_support as support;
use crate::support::generated_xlsx::{generated_format_workbook, generated_package};

const SHEET_PART: &str = "xl/worksheets/sheet1.xml";

fn address(a1: &str) -> CellAddress {
    CellAddress::from_a1(a1).expect("address")
}

fn column(index: u32) -> Column {
    Column::new(index).expect("column")
}

fn row(index: u32) -> Row {
    Row::new(index).expect("row")
}

fn styled_sheet(draft: &WorkbookDraft) -> SheetId {
    draft
        .workbook()
        .sheet_by_name("Styled")
        .expect("sheet")
        .id()
}

fn pattern(format: &CellFormat) -> PatternFill {
    match format.fill() {
        Fill::Pattern(fill) => fill,
        other => panic!("expected a pattern fill, found {other:?}"),
    }
}

fn widths(draft: &WorkbookDraft, sheet: SheetId) -> Vec<(u32, u32, f64)> {
    draft
        .presentation()
        .column_widths(sheet)
        .iter()
        .map(|span| (span.first().get(), span.last().get(), span.width()))
        .collect()
}

fn heights(draft: &WorkbookDraft, sheet: SheetId) -> Vec<(u32, f64)> {
    draft
        .presentation()
        .row_heights(sheet)
        .map(|(row, height)| (row.get(), height))
        .collect()
}

#[test]
fn reads_fonts_fills_borders_alignments_widths_and_heights() {
    let document = open_xlsx_document_bytes(&generated_format_workbook(), OpenOptions::default())
        .expect("open");
    let draft = WorkbookDraft::from_document(&document);
    let sheet = styled_sheet(&draft);

    let styled = draft.cell_format(sheet, address("A1")).expect("A1");
    let font = styled.font();
    assert_eq!(
        (font.name(), font.size(), font.bold(), font.italic()),
        (Some("Arial"), Some(12.0), true, false)
    );
    assert_eq!(font.color(), Some(Color::argb(0xFFFF_0000)));
    let fill = pattern(&styled);
    assert_eq!(fill.pattern(), FillPattern::Solid);
    assert_eq!(fill.foreground(), Some(Color::argb(0xFFFF_FF00)));
    assert_eq!(fill.background(), Some(Color::indexed(64)));
    let border = styled.border();
    assert_eq!(
        border.left(),
        BorderEdge::new(BorderStyle::Thin, Some(Color::indexed(64)))
    );
    assert_eq!(
        border.right(),
        BorderEdge::new(BorderStyle::Thin, Some(Color::automatic()))
    );
    assert_eq!(border.top(), BorderEdge::default());
    assert_eq!(border.bottom(), BorderEdge::new(BorderStyle::Double, None));
    assert_eq!(border.diagonal(), BorderEdge::new(BorderStyle::Hair, None));
    assert!(border.diagonal_down() && !border.diagonal_up());
    assert_eq!(
        styled.alignment(),
        Alignment::new()
            .with_horizontal(HorizontalAlignment::Center)
            .with_wrap_text(true)
    );
    assert_eq!(
        draft.cell_format(sheet, address("B1")).expect("B1"),
        styled,
        "a blank cell keeps its own style"
    );

    let gradient = draft.cell_format(sheet, address("C2")).expect("C2");
    assert_eq!(gradient.fill(), Fill::Gradient);

    let rotated = Alignment::new()
        .with_vertical(VerticalAlignment::Top)
        .with_indent(2)
        .and_then(|alignment| alignment.with_text_rotation(45))
        .expect("alignment");
    for position in ["D7", "E5"] {
        assert_eq!(
            draft
                .cell_format(sheet, address(position))
                .expect(position)
                .alignment(),
            rotated,
            "{position} takes its column or custom row style"
        );
    }

    let default = draft.cell_format(sheet, address("Z99")).expect("Z99");
    assert_eq!(
        (default.font().name(), default.font().size()),
        (Some("Calibri"), Some(11.0))
    );
    assert_eq!(default.font().color(), Some(Color::theme(1)));
    assert_eq!(default.fill(), Fill::Pattern(PatternFill::default()));
    assert_eq!(default.border(), Border::default());

    assert_eq!(widths(&draft, sheet), [(1, 2, 20.0), (4, 4, 9.5)]);
    assert_eq!(draft.presentation().column_width(sheet, column(3)), None);
    assert_eq!(heights(&draft, sheet), [(1, 30.0)]);
}

#[test]
fn document_writer_patches_edited_formats_widths_and_heights() {
    let source = generated_format_workbook();
    let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
    let mut draft = WorkbookDraft::from_document(&document);
    let sheet = styled_sheet(&draft);
    let semantic_revision = draft.semantic_revision();
    let presentation_revision = draft.presentation_revision();

    let font = Font::new()
        .with_name("Times New Roman")
        .and_then(|font| font.with_size(14.5))
        .expect("font")
        .with_italic(true)
        .with_underline(FontUnderline::Double);
    let tinted = Color::theme(4).with_tint(0.4).expect("tint");
    let outline = BorderEdge::new(BorderStyle::Thick, Some(Color::argb(0xFF00_00FF)));
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_format(
                sheet,
                address("A1"),
                CellFormatEdit::new().with_font(font.clone()),
            ),
            WorkbookChange::set_cell_format(
                sheet,
                address("E5"),
                CellFormatEdit::new().with_fill(PatternFill::solid(tinted)),
            ),
            WorkbookChange::set_cell_format(
                sheet,
                address("G9"),
                CellFormatEdit::new()
                    .with_border(Border::new().with_outline(outline))
                    .with_alignment(Alignment::new().with_horizontal(HorizontalAlignment::Right)),
            ),
            WorkbookChange::set_column_width(sheet, column(2), column(2), None),
            WorkbookChange::set_column_width(sheet, column(3), column(3), Some(15.0)),
            WorkbookChange::set_row_height(sheet, row(1), None),
            WorkbookChange::set_row_height(sheet, row(10), Some(42.0)),
        ]))
        .expect("format edits");
    assert_eq!(
        draft.semantic_revision(),
        semantic_revision,
        "format edits are presentation changes"
    );
    assert!(draft.presentation_revision() > presentation_revision);

    let a1 = draft.cell_format(sheet, address("A1")).expect("A1");
    assert_eq!(a1.font(), &font);
    assert_eq!(
        pattern(&a1).foreground(),
        Some(Color::argb(0xFFFF_FF00)),
        "the components an edit leaves alone keep their source value"
    );
    let e5 = draft.cell_format(sheet, address("E5")).expect("E5");
    assert_eq!(pattern(&e5), PatternFill::solid(tinted));
    assert_eq!(e5.alignment().indent(), 2, "E5 keeps its row style");

    let saved = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
        .expect("edits write")
        .into_bytes();
    let reopened = open_xlsx_document_bytes(&saved, OpenOptions::default()).expect("reopen");
    let reread = WorkbookDraft::from_document(&reopened);
    for position in ["A1", "B1", "C2", "D7", "E5", "G9", "Z99"] {
        assert_eq!(
            reread
                .cell_format(sheet, address(position))
                .expect(position),
            draft.cell_format(sheet, address(position)).expect(position),
            "{position}"
        );
    }
    assert_eq!(
        widths(&reread, sheet),
        [(1, 1, 20.0), (3, 3, 15.0), (4, 4, 9.5)]
    );
    assert_eq!(heights(&reread, sheet), [(10, 42.0)]);
    let value = reread
        .workbook()
        .sheet_by_id(sheet)
        .and_then(|sheet| sheet.cell(address("A1")))
        .map(|cell| cell.content().clone());
    assert_eq!(
        value,
        Some(cellrune::CellContent::Literal(
            CellValue::number(1.0).expect("number")
        ))
    );

    let worksheet = support::part_text(&saved, SHEET_PART);
    assert!(
        worksheet.contains(r#"<col min="4" max="4" width="9.5" style="3"/>"#),
        "{worksheet}"
    );
    assert!(
        worksheet.contains(r#"<row r="10" ht="42" customHeight="1"/>"#),
        "{worksheet}"
    );
    assert!(worksheet.contains(r#"<row r="1">"#), "{worksheet}");
    assert!(
        worksheet.contains(r#"<c r="C2" s="2"><v>2.5</v></c>"#),
        "an unedited cell keeps its source bytes"
    );
    let styles = support::part_text(&saved, "xl/styles.xml");
    assert!(
        styles.contains(r#"<stop position="1"><color theme="4" tint="-0.25"/></stop>"#),
        "unedited records keep their source bytes"
    );
}

#[test]
fn canonical_writer_writes_formats_widths_and_heights() {
    let mut draft = WorkbookDraft::new();
    let sheet = SheetId::new(1).expect("sheet");
    let bold = Font::new()
        .with_name("Calibri")
        .and_then(|font| font.with_size(11.0))
        .expect("font")
        .with_bold(true);
    draft
        .apply_changes(EditBatch::new([
            WorkbookChange::set_cell_value(
                sheet,
                address("A1"),
                CellValue::number(5.0).expect("number"),
            ),
            WorkbookChange::set_cell_format(
                sheet,
                address("A1"),
                CellFormatEdit::new()
                    .with_font(bold.clone())
                    .with_fill(PatternFill::solid(Color::argb(0xFFDD_EEFF))),
            ),
            WorkbookChange::set_cell_format(
                sheet,
                address("C3"),
                CellFormatEdit::new().with_border(
                    Border::new().with_bottom(BorderEdge::new(BorderStyle::Medium, None)),
                ),
            ),
            WorkbookChange::set_column_width(sheet, column(1), column(3), Some(12.25)),
            WorkbookChange::set_row_height(sheet, row(3), Some(21.0)),
        ]))
        .expect("edits");
    let default = draft.cell_format(sheet, address("B2")).expect("B2");
    assert_eq!(
        (default.font().name(), default.font().size()),
        (Some("Calibri"), Some(11.0))
    );

    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let written =
        write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
            .expect("canonical write");
    let reopened =
        open_xlsx_document_bytes(written.bytes(), OpenOptions::default()).expect("reopen");
    let reread = WorkbookDraft::from_document(&reopened);
    for position in ["A1", "B2", "C3"] {
        assert_eq!(
            reread
                .cell_format(sheet, address(position))
                .expect(position),
            draft.cell_format(sheet, address(position)).expect(position),
            "{position}"
        );
    }
    assert_eq!(
        reread.cell_format(sheet, address("A1")).expect("A1").font(),
        &bold
    );
    assert_eq!(widths(&reread, sheet), [(1, 3, 12.25)]);
    assert_eq!(heights(&reread, sheet), [(3, 21.0)]);
}

#[test]
fn format_edits_validate_their_values_and_roll_back_the_batch() {
    let mut draft = WorkbookDraft::new();
    let sheet = SheetId::new(1).expect("sheet");
    assert!(matches!(
        Font::new().with_size(0.5),
        Err(ValidationError::FormatValueOutOfRange {
            property: "font_size",
            ..
        })
    ));
    assert!(matches!(
        Font::new().with_name(""),
        Err(ValidationError::FontNameInvalid)
    ));
    assert!(matches!(
        Color::theme(1).with_tint(1.5),
        Err(ValidationError::FormatValueOutOfRange {
            property: "color_tint",
            ..
        })
    ));
    assert!(matches!(
        Alignment::new().with_text_rotation(200),
        Err(ValidationError::FormatValueOutOfRange {
            property: "text_rotation",
            ..
        })
    ));
    let revision = draft.presentation_revision();
    for change in [
        WorkbookChange::set_column_width(sheet, column(1), column(1), Some(256.0)),
        WorkbookChange::set_row_height(sheet, row(1), Some(f64::NAN)),
        WorkbookChange::set_column_width(sheet, column(3), column(2), Some(10.0)),
    ] {
        let error = draft
            .apply_changes(EditBatch::new([
                WorkbookChange::set_row_height(sheet, row(2), Some(18.0)),
                change,
            ]))
            .expect_err("an invalid layout value fails the batch");
        assert!(
            matches!(
                error,
                ValidationError::FormatValueOutOfRange { .. } | ValidationError::RangeStartAfterEnd
            ),
            "{error:?}"
        );
        assert_eq!(draft.presentation_revision(), revision);
        assert_eq!(draft.presentation().row_height(sheet, row(2)), None);
    }
    let unchanged = draft
        .apply_changes(EditBatch::new([WorkbookChange::set_cell_format(
            sheet,
            address("A1"),
            CellFormatEdit::new().with_alignment(Alignment::new()),
        )]))
        .expect("a no-op format edit");
    assert_eq!(unchanged.result_revision(), unchanged.base_revision());
    assert_eq!(draft.presentation_revision(), revision);
}

#[test]
fn a_session_rejects_a_batch_prepared_before_a_presentation_only_install() {
    let mut session = cellrune::WorkbookCalculationSession::create();
    let sheet = SheetId::new(1).expect("sheet");
    let revision = session.workbook().semantic_revision();
    let height =
        |height| EditBatch::new([WorkbookChange::set_row_height(sheet, row(1), Some(height))]);
    let first = session
        .prepare_changes(revision, height(20.0))
        .expect("first");
    let second = session
        .prepare_changes(revision, height(30.0))
        .expect("second");
    session.install_changes(first).expect("install first");
    assert_eq!(
        session.workbook().semantic_revision(),
        revision,
        "a presentation-only batch keeps the semantic revision"
    );
    let error = session
        .install_changes(second)
        .expect_err("the second batch would drop the first one's height");
    assert!(
        matches!(
            &error,
            cellrune::ApplyChangesError::Session(error)
                if error.code() == cellrune::SessionErrorCode::RevisionMismatch
        ),
        "{error:?}"
    );
    assert_eq!(
        session.draft().presentation().row_height(sheet, row(1)),
        Some(20.0)
    );
}

#[test]
fn clearing_the_width_of_a_styled_column_keeps_a_standard_width() {
    use super::style_record_write::{one_sheet_package, sheet1};

    for (format_pr, standard) in [
        ("", "8.7109375"),
        (
            r#"<sheetFormatPr baseColWidth="10" defaultRowHeight="15"/>"#,
            "10.7109375",
        ),
        (
            r#"<sheetFormatPr defaultColWidth="12.5" defaultRowHeight="15"/>"#,
            "12.5",
        ),
    ] {
        let worksheet = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{format_pr}<cols><col min="1" max="5" width="20" customWidth="1" style="1"/></cols><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData></worksheet>"#
        );
        const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="14" applyNumberFormat="1"/></cellXfs></styleSheet>"#;
        let source = one_sheet_package(&worksheet, Some(STYLES));
        let document = open_xlsx_document_bytes(&source, OpenOptions::default()).expect("open");
        let mut draft = WorkbookDraft::from_document(&document);
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_column_width(
                sheet1(),
                column(3),
                column(3),
                None,
            )]))
            .expect("clear");
        let saved = write_xlsx_draft_edits_bytes(&draft, WriteOptions::default())
            .expect("edits write")
            .into_bytes();
        let written = support::part_text(&saved, "xl/worksheets/sheet1.xml");
        // Excel reads a <col> without `width` as zero wide, hiding the column.
        assert!(
            written.contains(&format!(
                r#"<col min="3" max="3" style="1" width="{standard}"/>"#
            )),
            "{written}"
        );
        let reopened = WorkbookDraft::from_document(
            &open_xlsx_document_bytes(&saved, OpenOptions::default()).expect("reopen"),
        );
        assert_eq!(
            reopened.presentation().column_width(sheet1(), column(3)),
            None
        );
        assert!(
            reopened
                .presentation()
                .column_width(sheet1(), column(2))
                .is_some()
        );
    }
}

/// Numbers writes `<numFmt numFmtId="0" formatCode="General"/>`, redeclaring the built-in General.
#[test]
fn a_general_format_declared_as_custom_reads_as_general_and_saves_edits() {
    let styles = r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="0" formatCode="General"/></numFmts><fonts count="1"><font><sz val="10"/><name val="Helvetica Neue"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" applyNumberFormat="0"/></cellXfs></styleSheet>"#;
    let bytes = generated_package(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet 1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        ),
        ("xl/styles.xml", styles),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="0"><v>3</v></c></row></sheetData></worksheet>"#,
        ),
    ]);
    let document = open_xlsx_document_bytes(&bytes, OpenOptions::default()).expect("open");
    let sheet_id = document.workbook().sheets()[0].id();
    let a1 = document.workbook().sheets()[0]
        .cell(address("A1"))
        .expect("A1");
    assert_eq!(a1.number_format().kind(), NumberFormatKind::General);
    let mut draft = WorkbookDraft::from_document(&document);
    draft
        .set_cell_value(
            sheet_id,
            address("B1"),
            CellValue::number(9.0).expect("number"),
        )
        .expect("edit");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("an edit of the sheet saves");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("reopen");
    assert_eq!(
        reopened.workbook().sheets()[0]
            .cell(address("B1"))
            .map(|cell| cell.number_format().kind()),
        Some(NumberFormatKind::General)
    );
}
