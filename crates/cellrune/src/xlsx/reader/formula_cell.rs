use std::collections::BTreeMap;

use super::super::error::detail;
use super::super::xml::{XmlAttributes, XmlBudget};
use super::super::{XlsxErrorCode, XlsxReadError};
use super::cell_value::{parse_cell_range, parse_cell_reference, parse_saved_result};
use super::formula_reference::shift_formula;
use super::shared_strings::SharedStrings;
use crate::{
    CellAddress, CellRange, FormulaCell, FormulaDialect, FormulaMetadata, FormulaText,
    SharedFormulaRole,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormulaKind {
    Normal,
    Shared,
    Array,
    DataTable,
}

#[derive(Debug)]
pub(super) struct RawFormula {
    kind: FormulaKind,
    range: Option<CellRange>,
    shared_index: Option<u32>,
    always_calculate_array: bool,
    recalculate_always: bool,
    input_cell_1: Option<CellAddress>,
    input_cell_2: Option<CellAddress>,
    two_dimensional: bool,
    row_oriented: bool,
    input_cell_1_deleted: bool,
    input_cell_2_deleted: bool,
    text: String,
}

#[derive(Debug)]
struct SharedFormulaAnchor {
    address: CellAddress,
    range: CellRange,
    text: FormulaText,
}

#[derive(Debug, Default)]
pub(super) struct SharedFormulaTable(BTreeMap<u32, SharedFormulaAnchor>);

impl RawFormula {
    pub(super) fn parse(
        attributes: &XmlAttributes,
        budget: &XmlBudget,
    ) -> Result<Self, XlsxReadError> {
        let kind = match attributes.unqualified("t") {
            None | Some("normal") => FormulaKind::Normal,
            Some("shared") => FormulaKind::Shared,
            Some("array") => FormulaKind::Array,
            Some("dataTable") => FormulaKind::DataTable,
            Some(value) => {
                return Err(budget
                    .error(XlsxErrorCode::InvalidFormulaMetadata)
                    .with_detail(value.to_owned()));
            }
        };
        let range = attributes
            .unqualified("ref")
            .map(|value| parse_cell_range(value, budget))
            .transpose()?;
        let shared_index = optional_u32(attributes.unqualified("si"), budget)?;
        let always_calculate_array = parse_bool(attributes.unqualified("aca"), false, budget)?;
        let recalculate_always = parse_bool(attributes.unqualified("ca"), false, budget)?;
        let input_cell_1 = attributes
            .unqualified("r1")
            .map(|value| parse_formula_address(value, budget))
            .transpose()?;
        let input_cell_2 = attributes
            .unqualified("r2")
            .map(|value| parse_formula_address(value, budget))
            .transpose()?;
        let two_dimensional = parse_bool(attributes.unqualified("dt2D"), false, budget)?;
        let row_oriented = parse_bool(attributes.unqualified("dtr"), false, budget)?;
        let input_cell_1_deleted = parse_bool(attributes.unqualified("del1"), false, budget)?;
        let input_cell_2_deleted = parse_bool(attributes.unqualified("del2"), false, budget)?;
        if parse_bool(attributes.unqualified("bx"), false, budget)? {
            return Err(budget.error(XlsxErrorCode::InvalidFormulaMetadata));
        }
        let formula = Self {
            kind,
            range,
            shared_index,
            always_calculate_array,
            recalculate_always,
            input_cell_1,
            input_cell_2,
            two_dimensional,
            row_oriented,
            input_cell_1_deleted,
            input_cell_2_deleted,
            text: String::new(),
        };
        formula.validate_attributes(budget)?;
        Ok(formula)
    }

    pub(super) fn append(&mut self, text: String, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        let next_length = self.text.len().saturating_add(text.len()) as u64;
        if next_length > budget.limits().max_formula_bytes() {
            return Err(budget.error(XlsxErrorCode::FormulaTooLarge));
        }
        self.text.push_str(&text);
        Ok(())
    }

    fn validate_attributes(&self, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        let has_data_table_attributes = self.input_cell_1.is_some()
            || self.input_cell_2.is_some()
            || self.two_dimensional
            || self.row_oriented
            || self.input_cell_1_deleted
            || self.input_cell_2_deleted;
        let valid = match self.kind {
            FormulaKind::Normal => {
                self.range.is_none()
                    && self.shared_index.is_none()
                    && !self.always_calculate_array
                    && !has_data_table_attributes
            }
            FormulaKind::Shared => {
                self.shared_index.is_some()
                    && !self.always_calculate_array
                    && !has_data_table_attributes
            }
            FormulaKind::Array => self.shared_index.is_none() && !has_data_table_attributes,
            FormulaKind::DataTable => {
                self.range.is_some() && self.shared_index.is_none() && !self.always_calculate_array
            }
        };
        if !valid {
            return Err(budget.error(XlsxErrorCode::InvalidFormulaMetadata));
        }
        Ok(())
    }
}

pub(super) struct FormulaResultInput<'a> {
    pub(super) address: CellAddress,
    pub(super) cell_type: &'a str,
    pub(super) raw_value: Option<&'a str>,
    pub(super) inline_text: Option<&'a str>,
    pub(super) shared_strings: Option<&'a SharedStrings>,
    pub(super) dynamic_array: bool,
}

pub(super) fn finish_formula(
    raw: RawFormula,
    input: FormulaResultInput<'_>,
    shared_formulas: &mut SharedFormulaTable,
    budget: &XmlBudget,
) -> Result<FormulaCell, XlsxReadError> {
    let saved_result = parse_saved_result(
        input.cell_type,
        input.raw_value,
        input.inline_text,
        input.shared_strings,
        budget,
    )?;
    let (text, metadata) = if input.dynamic_array {
        if !matches!(raw.kind, FormulaKind::Normal | FormulaKind::Array) {
            return Err(budget.error(XlsxErrorCode::InvalidFormulaMetadata));
        }
        if let Some(range) = raw.range {
            require_master_at_range_start(input.address, range, budget)?;
        }
        let text = required_formula_text(raw.text, budget)?;
        (
            Some(text),
            FormulaMetadata::DynamicArray {
                range: raw.range,
                always_calculate: raw.always_calculate_array,
            },
        )
    } else {
        finish_by_kind(&raw, input.address, shared_formulas, budget)?
    };
    Ok(FormulaCell::from_xlsx_parts(
        FormulaDialect::ExcelA1,
        text,
        saved_result,
        metadata,
        raw.recalculate_always,
    ))
}

fn finish_by_kind(
    raw: &RawFormula,
    address: CellAddress,
    shared_formulas: &mut SharedFormulaTable,
    budget: &XmlBudget,
) -> Result<(Option<FormulaText>, FormulaMetadata), XlsxReadError> {
    match raw.kind {
        FormulaKind::Normal => Ok((
            optional_formula_text(raw.text.clone(), budget)?,
            FormulaMetadata::Normal,
        )),
        FormulaKind::Shared => finish_shared(raw, address, shared_formulas, budget),
        FormulaKind::Array => {
            let range = raw
                .range
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidFormulaMetadata))?;
            require_master_at_range_start(address, range, budget)?;
            Ok((
                Some(required_formula_text(raw.text.clone(), budget)?),
                FormulaMetadata::Array {
                    range,
                    always_calculate: raw.always_calculate_array,
                },
            ))
        }
        FormulaKind::DataTable => {
            let range = raw
                .range
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidFormulaMetadata))?;
            require_master_at_range_start(address, range, budget)?;
            let text = optional_formula_text(raw.text.clone(), budget)?;
            Ok((
                text,
                FormulaMetadata::DataTable {
                    range,
                    input_cell_1: raw.input_cell_1,
                    input_cell_2: raw.input_cell_2,
                    two_dimensional: raw.two_dimensional,
                    row_oriented: raw.row_oriented,
                    input_cell_1_deleted: raw.input_cell_1_deleted,
                    input_cell_2_deleted: raw.input_cell_2_deleted,
                },
            ))
        }
    }
}

fn finish_shared(
    raw: &RawFormula,
    address: CellAddress,
    shared_formulas: &mut SharedFormulaTable,
    budget: &XmlBudget,
) -> Result<(Option<FormulaText>, FormulaMetadata), XlsxReadError> {
    let group_index = raw
        .shared_index
        .ok_or_else(|| budget.error(XlsxErrorCode::InvalidFormulaMetadata))?;
    if let Some(range) = raw.range {
        if !range.contains(address) {
            return Err(budget
                .error(XlsxErrorCode::InvalidFormulaMetadata)
                .with_detail(detail::SHARED_FORMULA_OUTSIDE_RANGE));
        }
        let text = required_formula_text(raw.text.clone(), budget)?;
        let anchor = SharedFormulaAnchor {
            address,
            range,
            text: text.clone(),
        };
        if shared_formulas.0.insert(group_index, anchor).is_some() {
            return Err(budget
                .error(XlsxErrorCode::InvalidFormulaMetadata)
                .with_detail(detail::DUPLICATE_SHARED_FORMULA_GROUP));
        }
        return Ok((
            Some(text),
            FormulaMetadata::Shared {
                group_index,
                role: SharedFormulaRole::Anchor,
                range: Some(range),
            },
        ));
    }

    let anchor = shared_formulas.0.get(&group_index).ok_or_else(|| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_detail(detail::UNKNOWN_SHARED_FORMULA_GROUP)
    })?;
    if !anchor.range.contains(address) {
        return Err(budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_detail(detail::SHARED_FORMULA_OUTSIDE_RANGE));
    }
    if !raw.text.is_empty() {
        return Err(budget.error(XlsxErrorCode::InvalidFormulaMetadata));
    }
    let text = shift_formula(anchor.text.as_str(), anchor.address, address).map_err(|_| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_detail(detail::SHARED_FORMULA_SHIFT_FAILED)
    })?;
    let text = required_formula_text(text, budget)?;
    Ok((
        Some(text),
        FormulaMetadata::Shared {
            group_index,
            role: SharedFormulaRole::Follower {
                anchor: anchor.address,
            },
            range: None,
        },
    ))
}

fn required_formula_text(text: String, budget: &XmlBudget) -> Result<FormulaText, XlsxReadError> {
    if text.len() as u64 > budget.limits().max_formula_bytes() {
        return Err(budget.error(XlsxErrorCode::FormulaTooLarge));
    }
    FormulaText::from_xlsx(text).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_cause(error)
    })
}

fn optional_formula_text(
    text: String,
    budget: &XmlBudget,
) -> Result<Option<FormulaText>, XlsxReadError> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        required_formula_text(text, budget).map(Some)
    }
}

fn parse_formula_address(value: &str, budget: &XmlBudget) -> Result<CellAddress, XlsxReadError> {
    parse_cell_reference(value, budget).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_cause(error)
    })
}

fn require_master_at_range_start(
    address: CellAddress,
    range: CellRange,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    if range.start() != address {
        return Err(budget.error(XlsxErrorCode::InvalidFormulaMetadata));
    }
    Ok(())
}

fn optional_u32(value: Option<&str>, budget: &XmlBudget) -> Result<Option<u32>, XlsxReadError> {
    value
        .map(|value| {
            value.parse::<u32>().map_err(|error| {
                budget
                    .error(XlsxErrorCode::InvalidFormulaMetadata)
                    .with_cause(error)
            })
        })
        .transpose()
}

fn parse_bool(
    value: Option<&str>,
    default: bool,
    budget: &XmlBudget,
) -> Result<bool, XlsxReadError> {
    match value {
        None => Ok(default),
        Some("0" | "false") => Ok(false),
        Some("1" | "true") => Ok(true),
        Some(value) => Err(budget
            .error(XlsxErrorCode::InvalidFormulaMetadata)
            .with_detail(value.to_owned())),
    }
}
