use std::collections::BTreeMap;
use std::fs;

use sha2::{Digest, Sha256};

use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CellAddress, CellContent,
    CellValue, FormulaMetadata, FrozenPane, OpenOptions, PhoneticRun, PhoneticTextRange,
    PhoneticWriteOptions, RecalculationWriteOptions, RecalculationWritePolicy, SavedResult,
    WorkbookDraft, WriteOptions, XlsxDocumentKind, XlsxWriteErrorCode, calculate_workbook,
    open_xlsx_document_bytes, write_recalculated_xlsx, write_recalculated_xlsx_bytes,
    write_recalculated_xlsx_path, write_xlsx_draft_bytes,
};

use crate::support::generated_xlsx::{
    ProducerProfile, generated_workbook, generated_workbook_with_comment,
};

use super::recalculation_write_support as support;

use support::{
    TemporaryOutput, archive_parts, formula_at, part_text, replace_part_text, rewrite_archive,
};

#[test]
fn complete_recalculation_writes_every_typed_cache_and_preserves_semantics() {
    let source_bytes = generated_workbook(ProducerProfile::Excel);
    let document =
        open_xlsx_document_bytes(&source_bytes, OpenOptions::default()).expect("source document");
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());

    let output = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("complete recalculation output");
    assert!(output.report().is_complete());
    assert_eq!(
        output.report().materialized_count(),
        calculation.materialized_cells().len()
    );
    assert!(output.report().invalidated_cells().is_empty());
    assert!(output.report().diagnostics().is_empty());
    assert_eq!(
        output.report().provenance().input_hash(),
        Some(document.input_hash())
    );
    assert_eq!(
        output.report().provenance().semantic_revision(),
        document.semantic_revision()
    );
    let output_hash: [u8; 32] = Sha256::digest(output.bytes()).into();
    assert_eq!(output.report().output_hash().as_bytes(), &output_hash);

    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default())
        .expect("verified output must reopen");
    assert_eq!(
        reopened.workbook().defined_names(),
        document.workbook().defined_names()
    );
    assert_eq!(
        reopened.workbook().date_system(),
        document.workbook().date_system()
    );
    for source_sheet in document.workbook().sheets() {
        let output_sheet = reopened
            .workbook()
            .sheet_by_id(source_sheet.id())
            .expect("same sheet ID");
        assert_eq!(output_sheet.name(), source_sheet.name());
        assert_eq!(output_sheet.visibility(), source_sheet.visibility());
        for source_cell in source_sheet.cells() {
            let output_cell = output_sheet
                .cell(source_cell.address())
                .expect("source cell remains present");
            assert_eq!(output_cell.number_format(), source_cell.number_format());
            match source_cell.content() {
                CellContent::Literal(value) => {
                    assert_eq!(output_cell.content(), &CellContent::Literal(value.clone()));
                }
                CellContent::Formula(source_formula) => {
                    let CellContent::Formula(output_formula) = output_cell.content() else {
                        panic!("formula remains a formula");
                    };
                    assert_eq!(output_formula.text(), source_formula.text());
                    assert_eq!(output_formula.metadata(), source_formula.metadata());
                    assert_eq!(
                        output_formula.recalculate_always(),
                        source_formula.recalculate_always()
                    );
                    let id = CalculationCellId::new(source_sheet.id(), source_cell.address());
                    let CalculationCellResult::Value(expected) =
                        calculation.cell(id).expect("formula calculation")
                    else {
                        panic!("fixture formula must calculate");
                    };
                    assert_eq!(
                        output_formula.saved_result(),
                        &SavedResult::Present(expected.clone())
                    );
                }
            }
        }
    }

    let source_parts = archive_parts(&source_bytes);
    let output_parts = archive_parts(output.bytes());
    for name in [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/_rels/workbook.xml.rels",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
    ] {
        assert_eq!(output_parts.get(name), source_parts.get(name), "{name}");
    }
    assert_ne!(
        output_parts.get("xl/worksheets/sheet2.xml"),
        source_parts.get("xl/worksheets/sheet2.xml")
    );

    let mut writer_bytes = Vec::new();
    let writer_report = write_recalculated_xlsx(
        &document,
        &calculation,
        &mut writer_bytes,
        RecalculationWriteOptions::default(),
    )
    .expect("writer adapter");
    assert!(writer_report.is_complete());
    let writer_hash: [u8; 32] = Sha256::digest(&writer_bytes).into();
    assert_eq!(writer_report.output_hash().as_bytes(), &writer_hash);
    open_xlsx_document_bytes(&writer_bytes, OpenOptions::default()).expect("writer output reopens");
}

#[test]
fn strict_and_invalidating_policies_are_transactional_and_explicit() {
    let source = generated_workbook(ProducerProfile::Excel);
    let unsupported = replace_part_text(
        &source,
        "xl/worksheets/sheet2.xml",
        "Inputs!B2*2",
        "CELLRUNE_UNKNOWN(Inputs!B2)",
    );
    let document =
        open_xlsx_document_bytes(&unsupported, OpenOptions::default()).expect("source document");
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let strict_error = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    )
    .expect_err("strict output rejects an unavailable result");
    assert_eq!(
        strict_error.code(),
        XlsxWriteErrorCode::IncompleteCalculation
    );

    let destination = TemporaryOutput::new("xlsx");
    fs::write(destination.path(), b"existing destination").expect("existing sentinel");
    let replacement_options =
        RecalculationWriteOptions::new(WriteOptions::default().with_replace_existing(true));
    let path_error = write_recalculated_xlsx_path(
        &document,
        &calculation,
        destination.path(),
        replacement_options,
    )
    .expect_err("failed preparation must not touch the destination");
    assert_eq!(path_error.code(), XlsxWriteErrorCode::IncompleteCalculation);
    assert_eq!(
        fs::read(destination.path()).expect("sentinel remains readable"),
        b"existing destination"
    );

    let invalidating = RecalculationWriteOptions::default()
        .with_policy(RecalculationWritePolicy::InvalidateUnavailable);
    let output = write_recalculated_xlsx_bytes(&document, &calculation, invalidating)
        .expect("invalidating output");
    assert!(!output.report().is_complete());
    assert_eq!(output.report().invalidated_cells().len(), 1);
    assert_eq!(output.report().diagnostics().len(), 1);
    assert_eq!(
        output.report().diagnostics()[0].code().as_str(),
        "xlsx.write.invalidated_result"
    );
    let reopened = open_xlsx_document_bytes(output.bytes(), OpenOptions::default())
        .expect("invalidated output reopens");
    let formula = formula_at(&reopened, "Calculations", "B2");
    assert_eq!(formula.saved_result(), &SavedResult::Missing);
    assert_eq!(
        reopened
            .workbook()
            .calculation_hints()
            .full_calculation_on_load(),
        Some(true)
    );
    assert_eq!(
        reopened
            .workbook()
            .calculation_hints()
            .force_full_calculation(),
        Some(true)
    );
}

#[test]
fn calculation_identity_and_destination_contracts_are_enforced() {
    let source = generated_workbook(ProducerProfile::Excel);
    let equivalent_archive =
        generated_workbook_with_comment(ProducerProfile::Excel, "different input identity");
    let document =
        open_xlsx_document_bytes(&source, OpenOptions::default()).expect("source document");
    let other = open_xlsx_document_bytes(&equivalent_archive, OpenOptions::default())
        .expect("other document");
    let other_calculation = calculate_workbook(other.workbook(), CalculationOptions::default());
    let mismatch = write_recalculated_xlsx_bytes(
        &document,
        &other_calculation,
        RecalculationWriteOptions::default(),
    )
    .expect_err("calculation from another archive must fail");
    assert_eq!(mismatch.code(), XlsxWriteErrorCode::SourceIdentityMismatch);

    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let wrong_extension = TemporaryOutput::new("xlsm");
    let kind_error = write_recalculated_xlsx_path(
        &document,
        &calculation,
        wrong_extension.path(),
        RecalculationWriteOptions::default(),
    )
    .expect_err("XLSX cannot be written as XLSM");
    assert_eq!(kind_error.code(), XlsxWriteErrorCode::OutputKindMismatch);
    assert!(!wrong_extension.path().exists());

    let destination = TemporaryOutput::new("xlsx");
    fs::write(destination.path(), b"do not replace").expect("existing destination");
    let exists = write_recalculated_xlsx_path(
        &document,
        &calculation,
        destination.path(),
        RecalculationWriteOptions::default(),
    )
    .expect_err("replacement is opt in");
    assert_eq!(exists.code(), XlsxWriteErrorCode::DestinationExists);
    assert_eq!(
        fs::read(destination.path()).expect("destination remains"),
        b"do not replace"
    );

    let replace =
        RecalculationWriteOptions::new(WriteOptions::default().with_replace_existing(true));
    let report = write_recalculated_xlsx_path(&document, &calculation, destination.path(), replace)
        .expect("explicit replacement");
    assert!(report.is_complete());
    let replaced = fs::read(destination.path()).expect("replacement bytes");
    let replaced_hash: [u8; 32] = Sha256::digest(&replaced).into();
    assert_eq!(report.output_hash().as_bytes(), &replaced_hash);
    open_xlsx_document_bytes(&replaced, OpenOptions::default()).expect("replacement reopens");
}

#[test]
fn calc_chain_is_removed_and_unknown_content_is_preserved() {
    let source = generated_workbook(ProducerProfile::Excel);
    let mut replacements = BTreeMap::new();
    replacements.insert(
        "[Content_Types].xml".to_owned(),
        part_text(&source, "[Content_Types].xml").replace(
            "</Types>",
            r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#,
        ),
    );
    replacements.insert(
        "xl/_rels/workbook.xml.rels".to_owned(),
        part_text(&source, "xl/_rels/workbook.xml.rels").replace(
            "</Relationships>",
            r#"<Relationship Id="rId99" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#,
        ),
    );
    replacements.insert(
        "xl/worksheets/sheet2.xml".to_owned(),
        part_text(&source, "xl/worksheets/sheet2.xml")
            .replace(
                r#"<c r="B2">"#,
                r#"<c r="B2" custom="preserved"><foreign:marker xmlns:foreign="urn:cellrune:test"/>"#,
            )
            .replace(
                "</worksheet>",
                r#"<extLst><ext uri="cellrune-preservation-test"/></extLst></worksheet>"#,
            ),
    );
    let fixture = rewrite_archive(
        &source,
        &replacements,
        &[(
            "xl/calcChain.xml",
            br#"<?xml version="1.0"?><calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="B2" i="2"/></calcChain>"#,
        )],
    );
    let document =
        open_xlsx_document_bytes(&fixture, OpenOptions::default()).expect("extended document");
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let output = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("output with stale calc chain");

    assert!(
        output
            .report()
            .removed_parts()
            .iter()
            .any(|part| part.as_str() == "xl/calcChain.xml")
    );
    let parts = archive_parts(output.bytes());
    assert!(!parts.contains_key("xl/calcChain.xml"));
    let relationships =
        String::from_utf8(parts["xl/_rels/workbook.xml.rels"].clone()).expect("relationship XML");
    let content_types =
        String::from_utf8(parts["[Content_Types].xml"].clone()).expect("content-types XML");
    let worksheet =
        String::from_utf8(parts["xl/worksheets/sheet2.xml"].clone()).expect("worksheet XML");
    assert!(!relationships.contains("calcChain"));
    assert!(!content_types.contains("calcChain"));
    assert!(worksheet.contains(r#"custom="preserved""#));
    assert!(worksheet.contains("foreign:marker"));
    assert!(worksheet.contains("cellrune-preservation-test"));
}

#[test]
fn legacy_array_followers_are_materialized_even_when_source_cells_are_absent() {
    let source = generated_workbook(ProducerProfile::Excel);
    let array_source = replace_part_text(
        &source,
        "xl/worksheets/sheet1.xml",
        "  </sheetData>",
        r#"    <row r="10"><c r="A10"><f t="array" ref="A10:B11">SEQUENCE(2,2)</f><v>99</v></c></row>
  </sheetData>"#,
    );
    let document =
        open_xlsx_document_bytes(&array_source, OpenOptions::default()).expect("array document");
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let output = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("legacy array output");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("array reopen");
    let inputs = reopened
        .workbook()
        .sheet_by_name("Inputs")
        .expect("Inputs sheet");
    for (address, expected) in [("A10", 1.0), ("B10", 2.0), ("A11", 3.0), ("B11", 4.0)] {
        let cell = inputs
            .cell_by_a1(address)
            .expect("valid address")
            .expect("cell");
        if address == "A10" {
            let CellContent::Formula(formula) = cell.content() else {
                panic!("array anchor remains a formula");
            };
            assert!(matches!(formula.metadata(), FormulaMetadata::Array { .. }));
            assert_eq!(
                formula.saved_result(),
                &SavedResult::Present(CellValue::number(expected).expect("finite value"))
            );
        } else {
            assert_eq!(
                cell.content(),
                &CellContent::Literal(CellValue::number(expected).expect("finite value"))
            );
        }
    }
}

#[test]
fn shared_formula_containers_survive_cache_updates() {
    let source = generated_workbook(ProducerProfile::Excel);
    let worksheet = part_text(&source, "xl/worksheets/sheet2.xml")
        .replace(
            "<f>Inputs!B2*2</f>",
            r#"<f t="shared" si="5" ref="B2:B3">Inputs!B2*2</f>"#,
        )
        .replace("<f>SUM(Inputs!B2,7.5)</f>", r#"<f t="shared" si="5"/>"#);
    let fixture = rewrite_archive(
        &source,
        &BTreeMap::from([("xl/worksheets/sheet2.xml".to_owned(), worksheet)]),
        &[],
    );
    let document =
        open_xlsx_document_bytes(&fixture, OpenOptions::default()).expect("shared document");
    let calculation = calculate_workbook(document.workbook(), CalculationOptions::default());
    let output = write_recalculated_xlsx_bytes(
        &document,
        &calculation,
        RecalculationWriteOptions::default(),
    )
    .expect("shared formula output");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("shared reopen");

    for address in ["B2", "B3"] {
        let source_formula = formula_at(&document, "Calculations", address);
        let output_formula = formula_at(&reopened, "Calculations", address);
        assert_eq!(output_formula.text(), source_formula.text());
        assert_eq!(output_formula.metadata(), source_formula.metadata());
        assert!(matches!(
            output_formula.metadata(),
            FormulaMetadata::Shared { group_index: 5, .. }
        ));
    }
    let worksheet =
        String::from_utf8(archive_parts(output.bytes())["xl/worksheets/sheet2.xml"].clone())
            .expect("worksheet XML");
    assert!(worksheet.contains(r#"<f t="shared" si="5" ref="B2:B3">"#));
    assert!(worksheet.contains(r#"<f t="shared" si="5"/>"#));
}

#[test]
fn macro_enabled_packages_preserve_vba_bytes_while_editing_presentation() {
    let source = generated_workbook(ProducerProfile::Excel);
    let content_types = part_text(&source, "[Content_Types].xml")
        .replace(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        )
        .replace(
            "</Types>",
            r#"<Override PartName="/xl/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/></Types>"#,
        );
    let relationships = part_text(&source, "xl/_rels/workbook.xml.rels").replace(
        "</Relationships>",
        r#"<Relationship Id="rId98" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/></Relationships>"#,
    );
    let vba_bytes = b"\x00CellRune does not parse or execute VBA\xff";
    let fixture = rewrite_archive(
        &source,
        &BTreeMap::from([
            ("[Content_Types].xml".to_owned(), content_types),
            ("xl/_rels/workbook.xml.rels".to_owned(), relationships),
        ]),
        &[("xl/vbaProject.bin", vba_bytes)],
    );
    let document =
        open_xlsx_document_bytes(&fixture, OpenOptions::default()).expect("XLSM document");
    assert_eq!(document.kind(), XlsxDocumentKind::Xlsm);
    let mut draft = WorkbookDraft::from_document(&document);
    let sheet_id = document
        .workbook()
        .sheet_by_name("Inputs")
        .expect("Inputs sheet")
        .id();
    let address = CellAddress::from_a1("B3").expect("address");
    draft
        .set_phonetics(
            sheet_id,
            address,
            vec![
                PhoneticRun::new(PhoneticTextRange::new(0, 4).expect("range"), "セル")
                    .expect("run"),
            ],
            PhoneticWriteOptions::show(),
        )
        .expect("phonetics");
    draft
        .set_frozen_pane(sheet_id, FrozenPane::new(1, 1).expect("pane"))
        .expect("frozen pane");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let output = write_xlsx_draft_bytes(&draft, &calculation, RecalculationWriteOptions::default())
        .expect("macro-preserving output");
    let reopened =
        open_xlsx_document_bytes(output.bytes(), OpenOptions::default()).expect("XLSM reopen");
    assert_eq!(reopened.kind(), XlsxDocumentKind::Xlsm);
    let phonetics = reopened
        .presentation()
        .cell_phonetics(sheet_id, address)
        .expect("reopened phonetics");
    assert_eq!(phonetics.runs()[0].text(), "セル");
    assert_eq!(
        reopened.presentation().frozen_pane(sheet_id),
        Some(FrozenPane::new(1, 1).expect("pane"))
    );
    assert_eq!(
        archive_parts(output.bytes())["xl/vbaProject.bin"],
        vba_bytes
    );
}
