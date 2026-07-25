use crate::{
    Cell, CellContent, CellPresentation, CellRange, CellValue, FormulaCell, FormulaMetadata,
    PhoneticAlignment, PhoneticProperties, PhoneticType, SharedFormulaRole,
};

use super::materialization::MaterializationAction;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};

const DETAIL_INVALID_XML_CHARACTER: &str = "cell text contains a character forbidden by XML 1.0";
const DETAIL_MISSING_FORMULA_TEXT: &str = "formula metadata requires formula text";
const DETAIL_UNSUPPORTED_DATA_TABLE: &str =
    "data-table formula authoring requires a complete table result contract";
const DETAIL_BLANK_RESULT: &str = "blank calculation results have no XLSX cache representation";
const DETAIL_PHONETICS_REQUIRE_TEXT: &str =
    "phonetic annotations can only be serialized on literal text cells";
const DETAIL_ANNOTATED_CELLS: &str = "max_annotated_cells";
const DETAIL_PHONETIC_RUNS_PER_CELL: &str = "max_phonetic_runs_per_cell";
const DETAIL_TOTAL_PHONETIC_RUNS: &str = "max_total_phonetic_runs";
const DETAIL_PHONETIC_TEXT_BYTES: &str = "max_phonetic_text_bytes";
const DETAIL_TOTAL_PHONETIC_TEXT_BYTES: &str = "max_total_phonetic_text_bytes";

pub(crate) fn validate_phonetic_limits(
    presentation: &crate::DocumentPresentation,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    let mut annotated_cells = 0_u64;
    let mut total_runs = 0_u64;
    let mut total_text_bytes = 0_u64;
    for state in presentation.cell_presentations() {
        let Some(annotation) = state.annotation.as_deref() else {
            continue;
        };
        annotated_cells = annotated_cells.saturating_add(1);
        if annotated_cells > limits.max_annotated_cells() {
            return Err(resource_limit(DETAIL_ANNOTATED_CELLS));
        }
        if annotation.runs().len() as u64 > limits.max_phonetic_runs_per_cell() {
            return Err(resource_limit(DETAIL_PHONETIC_RUNS_PER_CELL));
        }
        total_runs = total_runs.saturating_add(annotation.runs().len() as u64);
        if total_runs > limits.max_total_phonetic_runs() {
            return Err(resource_limit(DETAIL_TOTAL_PHONETIC_RUNS));
        }
        for run in annotation.runs() {
            let bytes = run.text().len() as u64;
            if bytes > limits.max_phonetic_text_bytes() {
                return Err(resource_limit(DETAIL_PHONETIC_TEXT_BYTES));
            }
            total_text_bytes = total_text_bytes.saturating_add(bytes);
            if total_text_bytes > limits.max_total_phonetic_text_bytes() {
                return Err(resource_limit(DETAIL_TOTAL_PHONETIC_TEXT_BYTES));
            }
        }
    }
    Ok(())
}

fn resource_limit(detail: &'static str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded).with_detail(detail)
}

pub(crate) fn serialize_cell(
    cell: &Cell,
    style_index: usize,
    calculation: Option<&MaterializationAction>,
    presentation: Option<&CellPresentation>,
) -> Result<String, XlsxWriteError> {
    let mut output = String::new();
    output.push_str("<c r=\"");
    output.push_str(&cell.address().to_string());
    output.push('"');
    if style_index != 0 {
        output.push_str(" s=\"");
        output.push_str(&style_index.to_string());
        output.push('"');
    }
    if matches!(
        cell.content(),
        CellContent::Formula(formula)
            if matches!(formula.metadata(), FormulaMetadata::DynamicArray { .. })
    ) {
        output.push_str(" cm=\"1\"");
    }
    if let Some(visible) = presentation.and_then(|state| state.explicit_visibility) {
        output.push_str(if visible { " ph=\"1\"" } else { " ph=\"0\"" });
    }
    match cell.content() {
        CellContent::Literal(value) => {
            if presentation
                .and_then(|state| state.annotation.as_deref())
                .is_some()
                && !matches!(value, CellValue::Text(_))
            {
                return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
                    .with_detail(DETAIL_PHONETICS_REQUIRE_TEXT));
            }
            push_literal_type(&mut output, value)?;
            output.push('>');
            push_literal(&mut output, value, presentation)?;
        }
        CellContent::Formula(formula) => {
            if let Some(action) = calculation {
                push_cache_type(&mut output, action)?;
            }
            output.push('>');
            push_formula(&mut output, formula)?;
            if let Some(action) = calculation {
                push_cache(&mut output, action)?;
            }
        }
    }
    output.push_str("</c>");
    Ok(output)
}

pub(crate) fn serialize_materialized_follower(
    address: crate::CellAddress,
    action: &MaterializationAction,
) -> Result<Option<String>, XlsxWriteError> {
    if matches!(action, MaterializationAction::Invalidate) {
        return Ok(None);
    }
    let mut output = String::new();
    output.push_str("<c r=\"");
    output.push_str(&address.to_string());
    output.push('"');
    push_cache_type(&mut output, action)?;
    output.push('>');
    push_cache(&mut output, action)?;
    output.push_str("</c>");
    Ok(Some(output))
}

pub(crate) fn escape_attribute(value: &str) -> Result<String, XlsxWriteError> {
    escape(value, true)
}

pub(crate) fn escape_text(value: &str) -> Result<String, XlsxWriteError> {
    escape(value, false)
}

fn push_literal_type(output: &mut String, value: &CellValue) -> Result<(), XlsxWriteError> {
    match value {
        CellValue::Blank | CellValue::Number(_) => {}
        CellValue::Text(text) => {
            validate_xml_text(text)?;
            output.push_str(" t=\"inlineStr\"");
        }
        CellValue::Logical(_) => output.push_str(" t=\"b\""),
        CellValue::Error(_) => output.push_str(" t=\"e\""),
    }
    Ok(())
}

fn push_literal(
    output: &mut String,
    value: &CellValue,
    presentation: Option<&CellPresentation>,
) -> Result<(), XlsxWriteError> {
    match value {
        CellValue::Blank => {}
        CellValue::Number(number) => {
            output.push_str("<v>");
            output.push_str(&number_to_xlsx_text(number.get()));
            output.push_str("</v>");
        }
        CellValue::Text(text) => {
            output.push_str("<is><t");
            if requires_space_preservation(text) {
                output.push_str(" xml:space=\"preserve\"");
            }
            output.push('>');
            output.push_str(&escape_text(text)?);
            output.push_str("</t>");
            if let Some(annotation) = presentation.and_then(|state| state.annotation.as_deref()) {
                for run in annotation.runs() {
                    output.push_str("<rPh sb=\"");
                    output.push_str(&run.base_range().start_utf16().to_string());
                    output.push_str("\" eb=\"");
                    output.push_str(&run.base_range().end_utf16().to_string());
                    output.push_str("\"><t");
                    if requires_space_preservation(run.text()) {
                        output.push_str(" xml:space=\"preserve\"");
                    }
                    output.push('>');
                    output.push_str(&escape_text(run.text())?);
                    output.push_str("</t></rPh>");
                }
                if let Some(properties) = annotation.properties() {
                    push_phonetic_properties(output, properties);
                }
            }
            output.push_str("</is>");
        }
        CellValue::Logical(value) => {
            output.push_str(if *value { "<v>1</v>" } else { "<v>0</v>" });
        }
        CellValue::Error(error) => {
            output.push_str("<v>");
            output.push_str(error.as_str());
            output.push_str("</v>");
        }
    }
    Ok(())
}

fn push_phonetic_properties(output: &mut String, properties: &PhoneticProperties) {
    output.push_str("<phoneticPr fontId=\"");
    output.push_str(&properties.effective_font_id().to_string());
    output.push('"');
    if let Some(phonetic_type) = properties.phonetic_type() {
        output.push_str(" type=\"");
        output.push_str(match phonetic_type {
            PhoneticType::HalfWidthKatakana => "halfwidthKatakana",
            PhoneticType::FullWidthKatakana => "fullwidthKatakana",
            PhoneticType::Hiragana => "Hiragana",
            PhoneticType::NoConversion => "noConversion",
        });
        output.push('"');
    }
    if let Some(alignment) = properties.alignment() {
        output.push_str(" alignment=\"");
        output.push_str(match alignment {
            PhoneticAlignment::NoControl => "noControl",
            PhoneticAlignment::Left => "left",
            PhoneticAlignment::Center => "center",
            PhoneticAlignment::Distributed => "distributed",
        });
        output.push('"');
    }
    output.push_str("/>");
}

fn push_formula(output: &mut String, formula: &FormulaCell) -> Result<(), XlsxWriteError> {
    output.push_str("<f");
    match formula.metadata() {
        FormulaMetadata::Normal => {}
        FormulaMetadata::Shared {
            group_index,
            role,
            range,
        } => {
            output.push_str(" t=\"shared\" si=\"");
            output.push_str(&group_index.to_string());
            output.push('"');
            if matches!(role, SharedFormulaRole::Anchor)
                && let Some(range) = range
            {
                push_range_attribute(output, "ref", *range);
            }
        }
        FormulaMetadata::Array {
            range,
            always_calculate,
        } => {
            output.push_str(" t=\"array\"");
            push_range_attribute(output, "ref", *range);
            if *always_calculate {
                output.push_str(" aca=\"1\"");
            }
        }
        FormulaMetadata::DynamicArray {
            range,
            always_calculate,
        } => {
            output.push_str(" t=\"array\"");
            if let Some(range) = range {
                push_range_attribute(output, "ref", *range);
            }
            if *always_calculate {
                output.push_str(" aca=\"1\"");
            }
        }
        FormulaMetadata::DataTable { .. } => {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedResultMaterialization)
                    .with_detail(DETAIL_UNSUPPORTED_DATA_TABLE),
            );
        }
    }
    if formula.recalculate_always() {
        output.push_str(" ca=\"1\"");
    }
    let text = match formula.metadata() {
        FormulaMetadata::Shared {
            role: SharedFormulaRole::Follower { .. },
            ..
        } => None,
        _ => formula.text(),
    };
    if let Some(text) = text {
        output.push('>');
        output.push_str(&escape_text(text.as_str())?);
        output.push_str("</f>");
    } else if matches!(
        formula.metadata(),
        FormulaMetadata::Normal
            | FormulaMetadata::Shared {
                role: SharedFormulaRole::Follower { .. },
                ..
            }
    ) {
        output.push_str("/>");
    } else {
        return Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
            .with_detail(DETAIL_MISSING_FORMULA_TEXT));
    }
    Ok(())
}

fn push_cache_type(
    output: &mut String,
    action: &MaterializationAction,
) -> Result<(), XlsxWriteError> {
    match action {
        MaterializationAction::Invalidate => {}
        MaterializationAction::Set(CellValue::Blank) => {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedResultMaterialization)
                    .with_detail(DETAIL_BLANK_RESULT),
            );
        }
        MaterializationAction::Set(CellValue::Number(_)) => {}
        MaterializationAction::Set(CellValue::Text(_)) => output.push_str(" t=\"str\""),
        MaterializationAction::Set(CellValue::Logical(_)) => output.push_str(" t=\"b\""),
        MaterializationAction::Set(CellValue::Error(_)) => output.push_str(" t=\"e\""),
    }
    Ok(())
}

fn push_cache(output: &mut String, action: &MaterializationAction) -> Result<(), XlsxWriteError> {
    let MaterializationAction::Set(value) = action else {
        return Ok(());
    };
    output.push_str("<v>");
    match value {
        CellValue::Blank => {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedResultMaterialization)
                    .with_detail(DETAIL_BLANK_RESULT),
            );
        }
        CellValue::Number(number) => output.push_str(&number_to_xlsx_text(number.get())),
        CellValue::Text(text) => output.push_str(&escape_text(text)?),
        CellValue::Logical(value) => output.push_str(if *value { "1" } else { "0" }),
        CellValue::Error(error) => output.push_str(error.as_str()),
    }
    output.push_str("</v>");
    Ok(())
}

pub(super) fn number_to_xlsx_text(value: f64) -> String {
    if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    }
}

fn push_range_attribute(output: &mut String, name: &str, range: CellRange) {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    output.push_str(&range.start().to_string());
    output.push(':');
    output.push_str(&range.end().to_string());
    output.push('"');
}

fn escape(value: &str, attribute: bool) -> Result<String, XlsxWriteError> {
    validate_xml_text(value)?;
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            '\'' if attribute => output.push_str("&apos;"),
            '\t' if attribute => output.push_str("&#x9;"),
            '\n' if attribute => output.push_str("&#xA;"),
            '\r' => output.push_str("&#xD;"),
            _ => output.push(character),
        }
    }
    Ok(output)
}

fn validate_xml_text(value: &str) -> Result<(), XlsxWriteError> {
    if value.chars().all(is_xml_10_character) {
        Ok(())
    } else {
        Err(XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
            .with_detail(DETAIL_INVALID_XML_CHARACTER))
    }
}

const fn is_xml_10_character(character: char) -> bool {
    matches!(
        character as u32,
        0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
    )
}

fn requires_space_preservation(value: &str) -> bool {
    value.is_empty()
        || value.chars().next().is_some_and(char::is_whitespace)
        || value.chars().next_back().is_some_and(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DETAIL_ANNOTATED_CELLS, DETAIL_PHONETIC_RUNS_PER_CELL, DETAIL_PHONETIC_TEXT_BYTES,
        DETAIL_TOTAL_PHONETIC_RUNS, DETAIL_TOTAL_PHONETIC_TEXT_BYTES, escape_attribute,
        escape_text, serialize_cell, validate_phonetic_limits,
    };
    use crate::xlsx::write::materialization::MaterializationAction;
    use crate::{
        Cell, CellAddress, CellContent, CellValue, DocumentPresentation, ExcelError, FormulaCell,
        FormulaDialect, FormulaMetadata, PhoneticAnnotation, PhoneticRun, PhoneticTextRange,
        SavedResult, SheetId, WriteLimits, XlsxWriteErrorCode,
    };

    fn annotation(runs: &[(u32, u32, &str)]) -> Arc<PhoneticAnnotation> {
        Arc::new(PhoneticAnnotation::new(
            runs.iter()
                .map(|(start, end, text)| {
                    PhoneticRun::new(PhoneticTextRange::new(*start, *end).expect("range"), *text)
                        .expect("run")
                })
                .collect(),
            None,
        ))
    }

    fn two_cell_presentation() -> DocumentPresentation {
        let mut presentation = DocumentPresentation::default();
        let sheet = SheetId::new(1).expect("sheet");
        presentation.source_cell_phonetics(
            sheet,
            CellAddress::from_a1("A1").expect("address"),
            Some(annotation(&[(0, 1, "ab")])),
            Some(true),
        );
        presentation.source_cell_phonetics(
            sheet,
            CellAddress::from_a1("A2").expect("address"),
            Some(annotation(&[(0, 1, "c")])),
            Some(true),
        );
        presentation
    }

    fn two_run_presentation() -> DocumentPresentation {
        let mut presentation = DocumentPresentation::default();
        presentation.source_cell_phonetics(
            SheetId::new(1).expect("sheet"),
            CellAddress::from_a1("A1").expect("address"),
            Some(annotation(&[(0, 1, "a"), (1, 2, "b")])),
            Some(true),
        );
        presentation
    }

    fn exact_limits() -> WriteLimits {
        WriteLimits::default()
            .with_max_annotated_cells(2)
            .expect("cell limit")
            .with_max_phonetic_runs_per_cell(1)
            .expect("per-cell limit")
            .with_max_total_phonetic_runs(2)
            .expect("run limit")
            .with_max_phonetic_text_bytes(2)
            .expect("per-run text limit")
            .with_max_total_phonetic_text_bytes(3)
            .expect("total text limit")
    }

    fn assert_limit(
        presentation: &DocumentPresentation,
        limits: WriteLimits,
        expected_detail: &'static str,
    ) {
        let error = validate_phonetic_limits(presentation, limits).expect_err("limit must fail");
        assert_eq!(error.code(), XlsxWriteErrorCode::ResourceLimitExceeded);
        assert_eq!(error.detail(), Some(expected_detail));
    }

    #[test]
    fn writer_accepts_exact_phonetic_resource_limits() {
        validate_phonetic_limits(&two_cell_presentation(), exact_limits())
            .expect("all exact limits are inclusive");
    }

    #[test]
    fn writer_reports_each_phonetic_resource_limit() {
        let presentation = two_cell_presentation();
        assert_limit(
            &presentation,
            exact_limits()
                .with_max_annotated_cells(1)
                .expect("cell limit"),
            DETAIL_ANNOTATED_CELLS,
        );
        assert_limit(
            &presentation,
            exact_limits()
                .with_max_total_phonetic_runs(1)
                .expect("run limit"),
            DETAIL_TOTAL_PHONETIC_RUNS,
        );
        assert_limit(
            &presentation,
            exact_limits()
                .with_max_phonetic_text_bytes(1)
                .expect("text limit"),
            DETAIL_PHONETIC_TEXT_BYTES,
        );
        assert_limit(
            &presentation,
            exact_limits()
                .with_max_total_phonetic_text_bytes(2)
                .expect("text limit"),
            DETAIL_TOTAL_PHONETIC_TEXT_BYTES,
        );

        let two_runs = two_run_presentation();
        assert_limit(
            &two_runs,
            WriteLimits::default()
                .with_max_phonetic_runs_per_cell(1)
                .expect("run limit"),
            DETAIL_PHONETIC_RUNS_PER_CELL,
        );
        validate_phonetic_limits(
            &two_runs,
            WriteLimits::default()
                .with_max_phonetic_runs_per_cell(2)
                .expect("run limit"),
        )
        .expect("exact per-cell run limit is inclusive");
    }

    #[test]
    fn canonical_cells_preserve_double_precision_and_empty_normal_formulas() {
        let number = Cell::new(
            CellAddress::from_a1("A1").expect("address"),
            CellContent::Literal(CellValue::number(1.234_567_890_123_456).expect("finite number")),
        );
        assert_eq!(
            serialize_cell(&number, 0, None, None).expect("serialize number"),
            r#"<c r="A1"><v>1.234567890123456</v></c>"#
        );

        let formula = Cell::new(
            CellAddress::from_a1("B1").expect("address"),
            CellContent::Formula(FormulaCell::from_xlsx_parts(
                FormulaDialect::ExcelA1,
                None,
                SavedResult::Present(CellValue::Error(ExcelError::Value)),
                FormulaMetadata::Normal,
                true,
            )),
        );
        let action = MaterializationAction::Set(CellValue::Error(ExcelError::Value));
        assert_eq!(
            serialize_cell(&formula, 0, Some(&action), None).expect("serialize formula"),
            r#"<c r="B1" t="e"><f ca="1"/><v>#VALUE!</v></c>"#
        );
    }

    #[test]
    fn xml_escaping_preserves_normalization_sensitive_characters() {
        assert_eq!(
            escape_text("a&<>\tb\nc\r\nd").expect("valid text"),
            "a&amp;&lt;&gt;\tb\nc&#xD;\nd"
        );
        assert_eq!(
            escape_attribute("a&<>\"'\tb\nc\r\nd").expect("valid attribute"),
            "a&amp;&lt;&gt;&quot;&apos;&#x9;b&#xA;c&#xD;&#xA;d"
        );
    }

    #[test]
    fn xml_escaping_rejects_xml_1_0_forbidden_characters() {
        for value in ["\0", "\u{0008}", "\u{000b}", "\u{001f}"] {
            let text_error = escape_text(value).expect_err("text character must be rejected");
            assert_eq!(text_error.code(), XlsxWriteErrorCode::InvalidGeneratedXml);

            let attribute_error =
                escape_attribute(value).expect_err("attribute character must be rejected");
            assert_eq!(
                attribute_error.code(),
                XlsxWriteErrorCode::InvalidGeneratedXml
            );
        }
    }
}
