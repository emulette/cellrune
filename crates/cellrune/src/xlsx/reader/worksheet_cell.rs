use super::super::error::{compatibility, detail};
use super::super::xml::{XmlAttributes, XmlBudget};
use super::super::{XlsxErrorCode, XlsxReadError};
use super::PresentationCapture;
use super::cell_value::{parse_cell_reference, parse_literal_value};
use super::formula_cell::{FormulaResultInput, RawFormula, SharedFormulaTable, finish_formula};
use super::metadata::CellMetadata;
use super::phonetic::{PhoneticItemBuilder, PhoneticReadBudget, parse_bool};
use super::worksheet::WorksheetResources;
use crate::{
    CellAddress, CellContent, CellValue, Diagnostic, DiagnosticCode, DiagnosticSeverity,
    DocumentPresentation, Sheet, SheetId, SourceLocation,
};

const FORMULA: &[u8] = b"f";
const VALUE: &[u8] = b"v";
const INLINE_STRING: &[u8] = b"is";
const TEXT: &[u8] = b"t";
const PHONETIC_RUN: &[u8] = b"rPh";
const PHONETIC_PROPERTIES: &[u8] = b"phoneticPr";

#[derive(Debug)]
pub(super) struct CellBuilder {
    depth: u64,
    address: CellAddress,
    cell_type: Box<str>,
    style_index: usize,
    metadata_index: Option<u32>,
    value: Option<String>,
    inline_text: String,
    inline_present: bool,
    formula: Option<RawFormula>,
    formula_depth: Option<u64>,
    value_depth: Option<u64>,
    inline_depth: Option<u64>,
    text_depth: Option<u64>,
    phonetic_depth: Option<u64>,
    phonetics: Option<PhoneticItemBuilder>,
    explicit_phonetic_visibility: Option<bool>,
    capture: PresentationCapture,
    font_count: u32,
}

pub(super) struct CellFinishContext<'resource, 'state> {
    pub(super) resources: WorksheetResources<'resource>,
    pub(super) shared_formulas: &'state mut SharedFormulaTable,
    pub(super) total_formula_bytes: &'state mut u64,
    pub(super) sheet: &'state mut Sheet,
    pub(super) presentation: &'state mut DocumentPresentation,
    pub(super) phonetic_budget: &'state mut PhoneticReadBudget,
    pub(super) budget: &'state XmlBudget,
}

impl CellBuilder {
    pub(super) fn begin(
        attributes: XmlAttributes,
        depth: u64,
        row_number: Option<u32>,
        capture: PresentationCapture,
        font_count: u32,
        budget: &XmlBudget,
    ) -> Result<Self, XlsxReadError> {
        let reference = attributes.unqualified("r").ok_or_else(|| {
            budget
                .error(XlsxErrorCode::InvalidCellReference)
                .with_detail(format!("{} r", detail::MISSING_ATTRIBUTE))
        })?;
        let address = parse_cell_reference(reference, budget)?;
        if row_number.is_some_and(|row| address.row().get() != row) {
            return Err(budget
                .error(XlsxErrorCode::InvalidCellReference)
                .with_detail(reference.to_owned()));
        }
        let style_index = optional_usize(
            attributes.unqualified("s"),
            XlsxErrorCode::InvalidStyleIndex,
            budget,
        )?
        .unwrap_or(0);
        let metadata_index = optional_u32(
            attributes.unqualified("cm"),
            XlsxErrorCode::InvalidCellMetadata,
            budget,
        )?;
        let cell_type = attributes.unqualified("t").unwrap_or("n");
        let explicit_phonetic_visibility = if capture == PresentationCapture::Document {
            attributes
                .unqualified("ph")
                .map(|value| parse_bool(value, budget))
                .transpose()?
        } else {
            None
        };
        Ok(Self {
            depth,
            address,
            cell_type: cell_type.to_owned().into_boxed_str(),
            style_index,
            metadata_index,
            value: None,
            inline_text: String::new(),
            inline_present: false,
            formula: None,
            formula_depth: None,
            value_depth: None,
            inline_depth: None,
            text_depth: None,
            phonetic_depth: None,
            phonetics: (capture == PresentationCapture::Document)
                .then(PhoneticItemBuilder::default),
            explicit_phonetic_visibility,
            capture,
            font_count,
        })
    }

    pub(super) const fn depth(&self) -> u64 {
        self.depth
    }

    pub(super) fn process_start(
        &mut self,
        local_name: &[u8],
        depth: u64,
        attributes: XmlAttributes,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.formula_depth.is_some() || self.value_depth.is_some() {
            return Err(budget.error(if self.formula_depth.is_some() {
                XlsxErrorCode::InvalidFormulaMetadata
            } else {
                XlsxErrorCode::InvalidCellValue
            }));
        }
        if local_name == FORMULA && depth == self.depth + 1 {
            self.begin_formula(attributes, Some(depth), budget)?;
        } else if local_name == VALUE && depth == self.depth + 1 {
            self.begin_value(Some(depth), budget)?;
        } else if local_name == INLINE_STRING && depth == self.depth + 1 {
            self.begin_inline(Some(depth), budget)?;
        } else if self.inline_depth.is_some()
            && local_name == PHONETIC_RUN
            && self.phonetic_depth.is_none()
        {
            self.phonetic_depth = Some(depth);
            if self.capture == PresentationCapture::Document {
                self.phonetics
                    .as_mut()
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                    .begin_run(&attributes, budget.limits(), budget)?;
            }
        } else if self.inline_depth.is_some()
            && local_name == PHONETIC_PROPERTIES
            && self.phonetic_depth.is_none()
            && self.capture == PresentationCapture::Document
        {
            self.phonetics
                .as_mut()
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                .set_properties(&attributes, self.font_count, budget)?;
        } else if self.inline_depth.is_some()
            && local_name == TEXT
            && self.text_depth.replace(depth).is_some()
        {
            return Err(budget.error(XlsxErrorCode::InvalidCellValue));
        }
        Ok(())
    }

    pub(super) fn process_empty(
        &mut self,
        local_name: &[u8],
        depth: u64,
        attributes: XmlAttributes,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if local_name == FORMULA && depth == self.depth + 1 {
            self.begin_formula(attributes, None, budget)?;
        } else if local_name == VALUE && depth == self.depth + 1 {
            self.begin_value(None, budget)?;
        } else if local_name == INLINE_STRING && depth == self.depth + 1 {
            self.begin_inline(None, budget)?;
        } else if self.inline_depth.is_some() && local_name == PHONETIC_RUN {
            if self.capture == PresentationCapture::Document {
                let builder = self
                    .phonetics
                    .as_mut()
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?;
                builder.begin_run(&attributes, budget.limits(), budget)?;
                builder.finish_run(budget)?;
            }
        } else if self.inline_depth.is_some()
            && local_name == PHONETIC_PROPERTIES
            && self.capture == PresentationCapture::Document
        {
            self.phonetics
                .as_mut()
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                .set_properties(&attributes, self.font_count, budget)?;
        }
        Ok(())
    }

    pub(super) fn append(&mut self, text: String, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        if self.formula_depth.is_some() {
            self.formula
                .as_mut()
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidFormulaMetadata))?
                .append(text, budget)
        } else if self.value_depth.is_some() {
            self.value
                .as_mut()
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellValue))?
                .push_str(&text);
            Ok(())
        } else if self.text_depth.is_some() {
            if self.phonetic_depth.is_some() {
                if self.capture == PresentationCapture::Document {
                    self.phonetics
                        .as_mut()
                        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                        .append_run_text(text, budget.limits(), budget)?;
                }
            } else {
                self.inline_text.push_str(&text);
            }
            Ok(())
        } else {
            Ok(())
        }
    }

    pub(super) fn process_end(
        &mut self,
        local_name: &[u8],
        depth: u64,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.formula_depth == Some(depth) && local_name == FORMULA {
            self.formula_depth = None;
        }
        if self.value_depth == Some(depth) && local_name == VALUE {
            self.value_depth = None;
        }
        if self.text_depth == Some(depth) && local_name == TEXT {
            self.text_depth = None;
        }
        if self.phonetic_depth == Some(depth) && local_name == PHONETIC_RUN {
            if self.capture == PresentationCapture::Document {
                self.phonetics
                    .as_mut()
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                    .finish_run(budget)?;
            }
            self.phonetic_depth = None;
        }
        if self.inline_depth == Some(depth) && local_name == INLINE_STRING {
            self.inline_depth = None;
        }
        Ok(())
    }

    pub(super) fn finish(self, context: CellFinishContext<'_, '_>) -> Result<(), XlsxReadError> {
        let CellFinishContext {
            resources,
            shared_formulas,
            total_formula_bytes,
            sheet,
            presentation,
            phonetic_budget,
            budget,
        } = context;
        let sheet_id = sheet.id();
        let had_formula = self.formula.is_some();
        let inline_text = self.inline_present.then_some(self.inline_text.as_str());
        let content = match self.formula {
            Some(formula) => {
                let dynamic_array =
                    resolve_dynamic_array(self.metadata_index, resources.cell_metadata, budget)?;
                let formula = finish_formula(
                    formula,
                    FormulaResultInput {
                        address: self.address,
                        cell_type: &self.cell_type,
                        raw_value: self.value.as_deref(),
                        inline_text,
                        shared_strings: resources.shared_strings,
                        dynamic_array,
                    },
                    shared_formulas,
                    budget,
                )?;
                charge_formula_bytes(&formula, total_formula_bytes, budget)?;
                Some(CellContent::Formula(formula))
            }
            None => {
                resolve_dynamic_array(self.metadata_index, resources.cell_metadata, budget)?;
                parse_literal_value(
                    &self.cell_type,
                    self.value.as_deref(),
                    inline_text,
                    resources.shared_strings,
                    budget,
                )?
                .map(CellContent::Literal)
            }
        };
        let mut annotation = None;
        let mut overlaps_or_reorders = false;
        if self.capture == PresentationCapture::Document {
            if self.inline_present {
                let completed = self
                    .phonetics
                    .ok_or_else(|| budget.error(XlsxErrorCode::InvalidPhoneticMetadata))?
                    .finish(&self.inline_text, phonetic_budget, budget.limits(), budget)?;
                if let Some(completed) = completed {
                    annotation = Some(completed.annotation);
                    overlaps_or_reorders = completed.overlaps_or_reorders;
                }
            } else if !had_formula && self.cell_type.as_ref() == "s" {
                let index = shared_string_index(self.value.as_deref(), budget)?;
                if let (Some(shared_strings), Some(index)) = (resources.shared_strings, index)
                    && let Some((shared_annotation, overlaps)) = shared_strings.annotation(index)
                {
                    annotation = Some(shared_annotation);
                    overlaps_or_reorders = overlaps;
                }
            }
        }
        if annotation.is_some()
            && !matches!(
                content.as_ref(),
                Some(CellContent::Literal(CellValue::Text(_)))
            )
        {
            return Err(budget.error(XlsxErrorCode::InvalidPhoneticMetadata));
        }
        if annotation.is_some() {
            phonetic_budget.charge_cell(budget.limits(), budget)?;
        }
        if overlaps_or_reorders {
            push_overlap_diagnostic(presentation, sheet_id, self.address, budget)?;
        }
        presentation.source_cell_phonetics(
            sheet_id,
            self.address,
            annotation,
            self.explicit_phonetic_visibility,
        );
        let Some(content) = content else {
            return Ok(());
        };
        let number_format = resources
            .styles
            .format(self.style_index)
            .cloned()
            .ok_or_else(|| {
                budget
                    .error(XlsxErrorCode::InvalidStyleIndex)
                    .with_detail(self.style_index.to_string())
            })?;
        sheet
            .insert_cell_with_number_format(self.address, content, number_format)
            .map_err(|error| {
                budget
                    .error(XlsxErrorCode::InvalidWorksheet)
                    .with_cause(error)
            })
    }

    fn begin_formula(
        &mut self,
        attributes: XmlAttributes,
        depth: Option<u64>,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.formula.is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidFormulaMetadata));
        }
        self.formula = Some(RawFormula::parse(&attributes, budget)?);
        self.formula_depth = depth;
        Ok(())
    }

    fn begin_value(&mut self, depth: Option<u64>, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        if self.value.is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidCellValue));
        }
        self.value = Some(String::new());
        self.value_depth = depth;
        Ok(())
    }

    fn begin_inline(
        &mut self,
        depth: Option<u64>,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.inline_present {
            return Err(budget.error(XlsxErrorCode::InvalidCellValue));
        }
        self.inline_present = true;
        self.inline_depth = depth;
        Ok(())
    }
}

fn charge_formula_bytes(
    formula: &crate::FormulaCell,
    total_formula_bytes: &mut u64,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let text_bytes = formula.text().map_or(0, |text| text.as_str().len() as u64);
    *total_formula_bytes = total_formula_bytes.saturating_add(text_bytes);
    if *total_formula_bytes > budget.limits().max_total_formula_bytes() {
        return Err(budget.error(XlsxErrorCode::TotalFormulaBytesTooLarge));
    }
    Ok(())
}

fn shared_string_index(
    raw_index: Option<&str>,
    budget: &XmlBudget,
) -> Result<Option<usize>, XlsxReadError> {
    raw_index
        .map(|value| {
            value.trim().parse::<usize>().map_err(|error| {
                budget
                    .error(XlsxErrorCode::InvalidCellValue)
                    .with_cause(error)
            })
        })
        .transpose()
}

fn push_overlap_diagnostic(
    presentation: &mut DocumentPresentation,
    sheet_id: SheetId,
    address: CellAddress,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let code = DiagnosticCode::new(compatibility::PHONETIC_OVERLAP_CODE).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidPhoneticMetadata)
            .with_cause(error)
    })?;
    let diagnostic = Diagnostic::new(
        code,
        DiagnosticSeverity::Warning,
        compatibility::PHONETIC_OVERLAP_MESSAGE,
        Some(SourceLocation::cell(
            budget.source_id().clone(),
            sheet_id,
            address,
        )),
    )
    .map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidPhoneticMetadata)
            .with_cause(error)
    })?;
    presentation.push_diagnostic(diagnostic);
    Ok(())
}

fn resolve_dynamic_array(
    metadata_index: Option<u32>,
    cell_metadata: Option<&CellMetadata>,
    budget: &XmlBudget,
) -> Result<bool, XlsxReadError> {
    let Some(index) = metadata_index else {
        return Ok(false);
    };
    let metadata = cell_metadata.ok_or_else(|| {
        budget
            .error(XlsxErrorCode::InvalidCellMetadata)
            .with_detail(detail::METADATA_PART_REQUIRED)
    })?;
    metadata
        .is_dynamic_array(index)
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidCellMetadata))
}

fn optional_usize(
    value: Option<&str>,
    code: XlsxErrorCode,
    budget: &XmlBudget,
) -> Result<Option<usize>, XlsxReadError> {
    value
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|error| budget.error(code).with_cause(error))
        })
        .transpose()
}

fn optional_u32(
    value: Option<&str>,
    code: XlsxErrorCode,
    budget: &XmlBudget,
) -> Result<Option<u32>, XlsxReadError> {
    value
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|error| budget.error(code).with_cause(error))
        })
        .transpose()
}
