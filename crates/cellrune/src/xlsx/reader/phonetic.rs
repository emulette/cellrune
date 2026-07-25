use std::sync::Arc;

use super::super::xml::{XmlAttributes, XmlBudget};
use super::super::{ReadLimits, XlsxErrorCode, XlsxReadError};
use crate::presentation::validate_source_runs;
use crate::{
    PhoneticAlignment, PhoneticAnnotation, PhoneticProperties, PhoneticRun, PhoneticTextRange,
    PhoneticType,
};

#[derive(Debug, Default)]
pub(super) struct PhoneticReadBudget {
    total_runs: u64,
    total_text_bytes: u64,
    annotated_cells: u64,
}

impl PhoneticReadBudget {
    pub(super) fn charge_item(
        &mut self,
        runs: &[PhoneticRun],
        limits: ReadLimits,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if runs.len() as u64 > limits.max_phonetic_runs_per_item() {
            return Err(budget.error(XlsxErrorCode::TooManyPhoneticRuns));
        }
        self.total_runs = self.total_runs.saturating_add(runs.len() as u64);
        if self.total_runs > limits.max_total_phonetic_runs() {
            return Err(budget.error(XlsxErrorCode::TooManyPhoneticRuns));
        }
        let text_bytes = runs
            .iter()
            .map(|run| run.text().len() as u64)
            .fold(0_u64, u64::saturating_add);
        self.total_text_bytes = self.total_text_bytes.saturating_add(text_bytes);
        if self.total_text_bytes > limits.max_total_phonetic_text_bytes() {
            return Err(budget.error(XlsxErrorCode::TotalPhoneticTextTooLarge));
        }
        Ok(())
    }

    pub(super) fn charge_cell(
        &mut self,
        limits: ReadLimits,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        self.annotated_cells = self.annotated_cells.saturating_add(1);
        if self.annotated_cells > limits.max_annotated_cells() {
            return Err(budget.error(XlsxErrorCode::TooManyAnnotatedCells));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct CompletedPhonetics {
    pub(super) annotation: Arc<PhoneticAnnotation>,
    pub(super) overlaps_or_reorders: bool,
}

#[derive(Debug)]
struct RunBuilder {
    range: PhoneticTextRange,
    text: String,
}

#[derive(Debug, Default)]
pub(super) struct PhoneticItemBuilder {
    runs: Vec<PhoneticRun>,
    current_run: Option<RunBuilder>,
    properties: Option<PhoneticProperties>,
}

impl PhoneticItemBuilder {
    pub(super) fn begin_run(
        &mut self,
        attributes: &XmlAttributes,
        limits: ReadLimits,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.current_run.is_some()
            || self.runs.len() as u64 >= limits.max_phonetic_runs_per_item()
        {
            return Err(budget.error(XlsxErrorCode::TooManyPhoneticRuns));
        }
        let start = required_u32(attributes.unqualified("sb"), budget)?;
        let end = required_u32(attributes.unqualified("eb"), budget)?;
        let range = PhoneticTextRange::new(start, end).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })?;
        self.current_run = Some(RunBuilder {
            range,
            text: String::new(),
        });
        Ok(())
    }

    pub(super) fn append_run_text(
        &mut self,
        text: String,
        limits: ReadLimits,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        let current = self
            .current_run
            .as_mut()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?;
        if current.text.len().saturating_add(text.len()) as u64 > limits.max_phonetic_text_bytes() {
            return Err(budget.error(XlsxErrorCode::PhoneticTextTooLarge));
        }
        current.text.push_str(&text);
        Ok(())
    }

    pub(super) fn finish_run(&mut self, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        let run = self
            .current_run
            .take()
            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?;
        let run = PhoneticRun::new(run.range, run.text.into_boxed_str()).map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })?;
        self.runs.push(run);
        Ok(())
    }

    pub(super) fn set_properties(
        &mut self,
        attributes: &XmlAttributes,
        font_count: u32,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.properties.is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidPhoneticMetadata));
        }
        self.properties = Some(parse_properties(attributes, font_count, budget)?);
        Ok(())
    }

    pub(super) fn finish(
        self,
        base_text: &str,
        read_budget: &mut PhoneticReadBudget,
        limits: ReadLimits,
        budget: &XmlBudget,
    ) -> Result<Option<CompletedPhonetics>, XlsxReadError> {
        if self.current_run.is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidPhoneticMetadata));
        }
        if self.runs.is_empty() && self.properties.is_none() {
            return Ok(None);
        }
        let overlaps_or_reorders =
            validate_source_runs(base_text, &self.runs).map_err(|error| {
                budget
                    .error(XlsxErrorCode::InvalidPhoneticMetadata)
                    .with_cause(error)
            })?;
        read_budget.charge_item(&self.runs, limits, budget)?;
        Ok(Some(CompletedPhonetics {
            annotation: Arc::new(PhoneticAnnotation::new(self.runs, self.properties)),
            overlaps_or_reorders,
        }))
    }
}

pub(super) fn parse_properties(
    attributes: &XmlAttributes,
    font_count: u32,
    budget: &XmlBudget,
) -> Result<PhoneticProperties, XlsxReadError> {
    let font_id = required_u32(attributes.unqualified("fontId"), budget)?;
    let mut properties = PhoneticProperties::from_source(font_id, font_count);
    if let Some(value) = attributes.unqualified("type") {
        let phonetic_type = match value {
            "halfwidthKatakana" => PhoneticType::HalfWidthKatakana,
            "fullwidthKatakana" => PhoneticType::FullWidthKatakana,
            "Hiragana" => PhoneticType::Hiragana,
            "noConversion" => PhoneticType::NoConversion,
            _ => {
                return Err(budget
                    .error(XlsxErrorCode::InvalidPhoneticMetadata)
                    .with_detail(value.to_owned()));
            }
        };
        properties = properties.with_phonetic_type(phonetic_type);
    }
    if let Some(value) = attributes.unqualified("alignment") {
        let alignment = match value {
            "noControl" => PhoneticAlignment::NoControl,
            "left" => PhoneticAlignment::Left,
            "center" => PhoneticAlignment::Center,
            "distributed" => PhoneticAlignment::Distributed,
            _ => {
                return Err(budget
                    .error(XlsxErrorCode::InvalidPhoneticMetadata)
                    .with_detail(value.to_owned()));
            }
        };
        properties = properties.with_alignment(alignment);
    }
    Ok(properties)
}

pub(super) fn parse_bool(value: &str, budget: &XmlBudget) -> Result<bool, XlsxReadError> {
    match value {
        "0" | "false" => Ok(false),
        "1" | "true" => Ok(true),
        _ => Err(budget
            .error(XlsxErrorCode::InvalidPhoneticMetadata)
            .with_detail(value.to_owned())),
    }
}

fn required_u32(value: Option<&str>, budget: &XmlBudget) -> Result<u32, XlsxReadError> {
    value
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
        .parse::<u32>()
        .map_err(|error| {
            budget
                .error(XlsxErrorCode::InvalidPhoneticMetadata)
                .with_cause(error)
        })
}

#[cfg(test)]
mod tests {
    use quick_xml::events::Event;

    use super::{PhoneticItemBuilder, PhoneticReadBudget, parse_bool, parse_properties};
    use crate::xlsx::xml::{XmlAttributes, XmlBudget, read_attributes, reader};
    use crate::{
        PhoneticAlignment, PhoneticRun, PhoneticTextRange, PhoneticType, ReadLimits, SourceId,
        XlsxErrorCode,
    };

    fn budget() -> XmlBudget {
        XmlBudget::new(
            ReadLimits::default(),
            SourceId::new("phonetic-test.xml").expect("source"),
            XlsxErrorCode::InvalidPhoneticMetadata,
        )
    }

    fn attributes(xml: &[u8], budget: &XmlBudget) -> XmlAttributes {
        let mut xml_reader = reader(xml);
        let mut buffer = Vec::new();
        match xml_reader
            .read_event_into(&mut buffer)
            .expect("attribute fixture")
        {
            Event::Start(element) | Event::Empty(element) => {
                read_attributes(&element, &xml_reader, budget).expect("attributes")
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    fn run(text: &str) -> PhoneticRun {
        PhoneticRun::new(
            PhoneticTextRange::new(0, 1).expect("range"),
            text.to_owned(),
        )
        .expect("run")
    }

    #[test]
    fn read_budget_accepts_exact_limits_and_rejects_the_next_item() {
        let xml_budget = budget();
        let limits = ReadLimits::default()
            .with_max_phonetic_runs_per_item(1)
            .expect("per-item limit")
            .with_max_total_phonetic_runs(2)
            .expect("run limit")
            .with_max_annotated_cells(2)
            .expect("cell limit")
            .with_max_total_phonetic_text_bytes(3)
            .expect("text limit");
        let first = run("ab");
        let second = run("c");
        let mut read_budget = PhoneticReadBudget::default();

        read_budget
            .charge_item(std::slice::from_ref(&first), limits, &xml_budget)
            .expect("first item");
        read_budget
            .charge_item(std::slice::from_ref(&second), limits, &xml_budget)
            .expect("exact total limits");
        assert_eq!(
            read_budget
                .charge_item(std::slice::from_ref(&second), limits, &xml_budget)
                .expect_err("third run exceeds total")
                .code(),
            XlsxErrorCode::TooManyPhoneticRuns
        );

        read_budget
            .charge_cell(limits, &xml_budget)
            .expect("first cell");
        read_budget
            .charge_cell(limits, &xml_budget)
            .expect("exact cell limit");
        assert_eq!(
            read_budget
                .charge_cell(limits, &xml_budget)
                .expect_err("third cell exceeds total")
                .code(),
            XlsxErrorCode::TooManyAnnotatedCells
        );
    }

    #[test]
    fn read_budget_rejects_per_item_and_total_text_overages() {
        let xml_budget = budget();
        let limits = ReadLimits::default()
            .with_max_phonetic_runs_per_item(1)
            .expect("per-item limit")
            .with_max_total_phonetic_runs(10)
            .expect("run limit")
            .with_max_total_phonetic_text_bytes(3)
            .expect("text limit");
        let item = run("ab");
        let mut read_budget = PhoneticReadBudget::default();

        assert_eq!(
            read_budget
                .charge_item(&[item.clone(), item.clone()], limits, &xml_budget)
                .expect_err("two runs exceed the per-item limit")
                .code(),
            XlsxErrorCode::TooManyPhoneticRuns
        );
        read_budget
            .charge_item(std::slice::from_ref(&item), limits, &xml_budget)
            .expect("two bytes");
        assert_eq!(
            read_budget
                .charge_item(std::slice::from_ref(&item), limits, &xml_budget)
                .expect_err("four bytes exceed total")
                .code(),
            XlsxErrorCode::TotalPhoneticTextTooLarge
        );
    }

    #[test]
    fn item_builder_enforces_run_state_count_and_text_boundaries() {
        let xml_budget = budget();
        let attributes = attributes(br#"<rPh sb="0" eb="1"/>"#, &xml_budget);
        let limits = ReadLimits::default()
            .with_max_phonetic_runs_per_item(1)
            .expect("run limit")
            .with_max_phonetic_text_bytes(2)
            .expect("text limit");
        let mut builder = PhoneticItemBuilder::default();

        builder
            .begin_run(&attributes, limits, &xml_budget)
            .expect("first run");
        assert_eq!(
            builder
                .begin_run(&attributes, limits, &xml_budget)
                .expect_err("a nested run is invalid")
                .code(),
            XlsxErrorCode::TooManyPhoneticRuns
        );
        builder
            .append_run_text("ab".to_owned(), limits, &xml_budget)
            .expect("exact text limit");
        assert_eq!(
            builder
                .append_run_text("c".to_owned(), limits, &xml_budget)
                .expect_err("text exceeds limit")
                .code(),
            XlsxErrorCode::PhoneticTextTooLarge
        );
        builder.finish_run(&xml_budget).expect("finish run");
        assert_eq!(
            builder
                .begin_run(&attributes, limits, &xml_budget)
                .expect_err("second run exceeds item limit")
                .code(),
            XlsxErrorCode::TooManyPhoneticRuns
        );
    }

    #[test]
    fn property_parser_covers_all_ooxml_enum_values_and_font_fallback() {
        let xml_budget = budget();
        let cases = [
            (
                "halfwidthKatakana",
                PhoneticType::HalfWidthKatakana,
                "noControl",
                PhoneticAlignment::NoControl,
            ),
            (
                "fullwidthKatakana",
                PhoneticType::FullWidthKatakana,
                "left",
                PhoneticAlignment::Left,
            ),
            (
                "Hiragana",
                PhoneticType::Hiragana,
                "center",
                PhoneticAlignment::Center,
            ),
            (
                "noConversion",
                PhoneticType::NoConversion,
                "distributed",
                PhoneticAlignment::Distributed,
            ),
        ];

        for (kind, expected_kind, alignment, expected_alignment) in cases {
            let xml = format!(r#"<phoneticPr fontId="7" type="{kind}" alignment="{alignment}"/>"#);
            let attributes = attributes(xml.as_bytes(), &xml_budget);
            let properties = parse_properties(&attributes, 2, &xml_budget).expect("properties");
            assert_eq!(properties.font_id(), 7);
            assert_eq!(properties.effective_font_id(), 0);
            assert_eq!(properties.phonetic_type(), Some(expected_kind));
            assert_eq!(properties.alignment(), Some(expected_alignment));
        }
    }

    #[test]
    fn boolean_parser_accepts_ooxml_spellings_and_rejects_other_values() {
        let xml_budget = budget();
        assert!(!parse_bool("0", &xml_budget).expect("numeric false"));
        assert!(!parse_bool("false", &xml_budget).expect("literal false"));
        assert!(parse_bool("1", &xml_budget).expect("numeric true"));
        assert!(parse_bool("true", &xml_budget).expect("literal true"));
        assert_eq!(
            parse_bool("yes", &xml_budget)
                .expect_err("invalid Boolean")
                .code(),
            XlsxErrorCode::InvalidPhoneticMetadata
        );
    }
}
