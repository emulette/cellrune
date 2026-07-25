use super::super::error::detail;
use super::super::xml::{XmlAttributes, XmlBudget};
use super::super::{XlsxErrorCode, XlsxReadError};
use super::workbook_xml::SheetMetadata;
use crate::{DefinedName, DefinedNameScope, FormulaText};

const DEFINED_NAME: &[u8] = b"definedName";

#[derive(Debug)]
struct DefinedNameBuilder {
    depth: u64,
    name: Box<str>,
    local_sheet_index: Option<usize>,
    hidden: bool,
    formula: String,
}

#[derive(Debug, Default)]
pub(super) struct DefinedNamesState {
    saw_container: bool,
    current: Option<DefinedNameBuilder>,
    values: Vec<DefinedNameBuilder>,
}

impl DefinedNamesState {
    pub(super) fn begin_container(&mut self, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        if std::mem::replace(&mut self.saw_container, true) {
            return Err(budget.error(XlsxErrorCode::InvalidDefinedName));
        }
        Ok(())
    }

    pub(super) fn begin(
        &mut self,
        attributes: &XmlAttributes,
        depth: u64,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self.current.is_some() || self.values.len() as u64 >= budget.limits().max_defined_names()
        {
            return Err(budget.error(if self.current.is_some() {
                XlsxErrorCode::InvalidDefinedName
            } else {
                XlsxErrorCode::TooManyDefinedNames
            }));
        }
        let name = required(attributes.unqualified("name"), "name", budget)?;
        let local_sheet_index = attributes
            .unqualified("localSheetId")
            .map(|value| {
                value.parse::<usize>().map_err(|error| {
                    budget
                        .error(XlsxErrorCode::InvalidDefinedName)
                        .with_cause(error)
                })
            })
            .transpose()?;
        let hidden = parse_bool(attributes.unqualified("hidden"), false, budget)?;
        self.current = Some(DefinedNameBuilder {
            depth,
            name: name.to_owned().into_boxed_str(),
            local_sheet_index,
            hidden,
            formula: String::new(),
        });
        Ok(())
    }

    pub(super) fn append(&mut self, text: String, budget: &XmlBudget) -> Result<(), XlsxReadError> {
        let Some(current) = &mut self.current else {
            return Ok(());
        };
        let next_length = current.formula.len().saturating_add(text.len()) as u64;
        if next_length > budget.limits().max_formula_bytes() {
            return Err(budget.error(XlsxErrorCode::FormulaTooLarge));
        }
        current.formula.push_str(&text);
        Ok(())
    }

    pub(super) fn finish(
        &mut self,
        local_name: &[u8],
        depth: u64,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.depth == depth && local_name == DEFINED_NAME)
        {
            let value = self
                .current
                .take()
                .ok_or_else(|| budget.error(XlsxErrorCode::InvalidDefinedName))?;
            self.values.push(value);
        }
        Ok(())
    }

    pub(super) fn resolve(
        self,
        sheets: &[SheetMetadata],
        budget: &XmlBudget,
    ) -> Result<Vec<DefinedName>, XlsxReadError> {
        if self.current.is_some() {
            return Err(budget.error(XlsxErrorCode::InvalidDefinedName));
        }
        self.values
            .into_iter()
            .map(|value| {
                let scope = match value.local_sheet_index {
                    None => DefinedNameScope::Workbook,
                    Some(index) => DefinedNameScope::Sheet(
                        sheets
                            .get(index)
                            .ok_or_else(|| budget.error(XlsxErrorCode::InvalidDefinedName))?
                            .id,
                    ),
                };
                let formula = FormulaText::from_xlsx(value.formula).map_err(|error| {
                    budget
                        .error(XlsxErrorCode::InvalidDefinedName)
                        .with_cause(error)
                })?;
                DefinedName::new(value.name.into_string(), scope, formula, value.hidden).map_err(
                    |error| {
                        budget
                            .error(XlsxErrorCode::InvalidDefinedName)
                            .with_cause(error)
                    },
                )
            })
            .collect()
    }
}

fn required<'a>(
    value: Option<&'a str>,
    name: &str,
    budget: &XmlBudget,
) -> Result<&'a str, XlsxReadError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(budget
            .error(XlsxErrorCode::InvalidDefinedName)
            .with_detail(format!("{} {name}", detail::MISSING_ATTRIBUTE))),
    }
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
            .error(XlsxErrorCode::InvalidDefinedName)
            .with_detail(value.to_owned())),
    }
}
