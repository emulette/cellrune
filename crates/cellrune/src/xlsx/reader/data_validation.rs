use quick_xml::name::QName;
use quick_xml::reader::NsReader;

use super::super::XlsxReadError;
use super::super::xml::{XmlAttributes, XmlBudget, is_element_in_namespace};
use super::table::parse_reference;
use crate::workbook::DataValidationRule;
use crate::{
    CellRange, DataValidation, DataValidationErrorStyle, DataValidationOperator, DataValidationType,
};

/// Holds `x14:dataValidations`, Excel 2010's form for rules whose formulas name another sheet.
const X14_MAIN: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
/// Holds the `xm:f` formulas and `xm:sqref` ranges of an `x14:dataValidation`.
const EXCEL_MAIN: &str = "http://schemas.microsoft.com/office/excel/2006/main";

const DATA_VALIDATIONS: &[u8] = b"dataValidations";
const DATA_VALIDATION: &[u8] = b"dataValidation";
const FORMULA1: &[u8] = b"formula1";
const FORMULA2: &[u8] = b"formula2";
const FORMULA: &[u8] = b"f";
const SQREF: &[u8] = b"sqref";

/// The namespace of an element the collector may consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Namespace {
    SpreadsheetMl,
    X14,
    ExcelMain,
    Other,
}

/// Resolves the namespace of an element whose local name [`DataValidationCollector::wants`].
pub(super) fn namespace(
    reader: &NsReader<&[u8]>,
    name: QName<'_>,
    is_spreadsheet: bool,
    budget: &XmlBudget,
) -> Result<Namespace, XlsxReadError> {
    Ok(if is_spreadsheet {
        Namespace::SpreadsheetMl
    } else if is_element_in_namespace(reader, name, X14_MAIN, budget)? {
        Namespace::X14
    } else if is_element_in_namespace(reader, name, EXCEL_MAIN, budget)? {
        Namespace::ExcelMain
    } else {
        Namespace::Other
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    /// `<dataValidations>`: formulas are element text and ranges the `sqref` attribute.
    Main,
    /// `<x14:dataValidations>`: formulas are `xm:f` children and ranges an `xm:sqref` element.
    X14,
}

impl Dialect {
    const fn namespace(self) -> Namespace {
        match self {
            Self::Main => Namespace::SpreadsheetMl,
            Self::X14 => Namespace::X14,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Formula1,
    Formula2,
    Ranges,
}

#[derive(Debug)]
struct PendingValidation {
    depth: u64,
    rule: Option<DataValidationRule>,
    ranges: Option<String>,
    formula1: Option<String>,
    formula2: Option<String>,
    repeated: bool,
    /// The open `x14:formula1` or `x14:formula2` whose `xm:f` child holds the formula.
    formula_parent: Option<(u64, Slot)>,
    /// The open element whose text is being collected.
    capture: Option<(u64, Slot, String)>,
}

impl PendingValidation {
    fn store(&mut self, slot: Slot, text: String) {
        let target = match slot {
            Slot::Formula1 => &mut self.formula1,
            Slot::Formula2 => &mut self.formula2,
            Slot::Ranges => &mut self.ranges,
        };
        if target.replace(text).is_some() {
            self.repeated = true;
        }
    }

    fn finish(self) -> Option<DataValidation> {
        if self.repeated {
            return None;
        }
        let ranges = parse_ranges(self.ranges.as_deref()?)?;
        Some(DataValidation::from_xlsx(
            ranges,
            self.rule?,
            self.formula1.map(String::into_boxed_str),
            self.formula2.map(String::into_boxed_str),
        ))
    }
}

/// Collects the worksheet's `<dataValidations>` and `<x14:dataValidations>` rules.
///
/// A rule with a missing or malformed range list, a repeated formula or range element, or an
/// unknown attribute token is left unmodeled. Its XML is preserved either way, so such a rule keeps
/// its source form.
#[derive(Debug, Default)]
pub(super) struct DataValidationCollector {
    container: Option<(u64, Dialect)>,
    pending: Option<PendingValidation>,
    validations: Vec<DataValidation>,
}

impl DataValidationCollector {
    /// Returns whether an element with `local_name` may belong to a data validation, so that the
    /// caller resolves its namespace only for those.
    pub(super) fn wants(&self, local_name: &[u8]) -> bool {
        local_name == DATA_VALIDATIONS
            || (self.container.is_some()
                && matches!(
                    local_name,
                    DATA_VALIDATION | FORMULA1 | FORMULA2 | FORMULA | SQREF
                ))
    }

    /// Records one start or empty element; an empty element is followed by [`Self::end`].
    pub(super) fn start(
        &mut self,
        local_name: &[u8],
        namespace: Namespace,
        depth: u64,
        attributes: &XmlAttributes,
    ) {
        let Some((container_depth, dialect)) = self.container else {
            match (local_name, namespace) {
                (DATA_VALIDATIONS, Namespace::SpreadsheetMl) if depth == 2 => {
                    self.container = Some((depth, Dialect::Main));
                }
                (DATA_VALIDATIONS, Namespace::X14) => {
                    self.container = Some((depth, Dialect::X14));
                }
                _ => {}
            }
            return;
        };
        let Some(pending) = &mut self.pending else {
            if depth == container_depth + 1
                && local_name == DATA_VALIDATION
                && namespace == dialect.namespace()
            {
                self.pending = Some(PendingValidation {
                    depth,
                    rule: parse_rule(attributes),
                    ranges: match dialect {
                        Dialect::Main => attributes.unqualified("sqref").map(str::to_owned),
                        Dialect::X14 => None,
                    },
                    formula1: None,
                    formula2: None,
                    repeated: false,
                    formula_parent: None,
                    capture: None,
                });
            }
            return;
        };
        if pending.capture.is_some() {
            return;
        }
        let formula_slot = match local_name {
            FORMULA1 => Some(Slot::Formula1),
            FORMULA2 => Some(Slot::Formula2),
            _ => None,
        };
        if depth == pending.depth + 1 {
            match (dialect, formula_slot, local_name, namespace) {
                (Dialect::Main, Some(slot), _, Namespace::SpreadsheetMl) => {
                    pending.capture = Some((depth, slot, String::new()));
                }
                (Dialect::X14, Some(slot), _, Namespace::X14) => {
                    pending.formula_parent = Some((depth, slot));
                }
                (Dialect::X14, None, SQREF, Namespace::ExcelMain) => {
                    pending.capture = Some((depth, Slot::Ranges, String::new()));
                }
                _ => {}
            }
        } else if let Some((parent_depth, slot)) = pending.formula_parent
            && depth == parent_depth + 1
            && local_name == FORMULA
            && namespace == Namespace::ExcelMain
        {
            pending.capture = Some((depth, slot, String::new()));
        }
    }

    /// Returns whether text events belong to an open formula or range element.
    pub(super) fn is_capturing(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.capture.is_some())
    }

    /// Appends decoded text to the open formula or range element; `None` marks text that did not
    /// decode, which leaves the rule unmodeled.
    pub(super) fn append(&mut self, text: Option<&str>) {
        if let Some(pending) = &mut self.pending
            && let Some((_, _, buffer)) = &mut pending.capture
        {
            match text {
                Some(text) => buffer.push_str(text),
                None => pending.rule = None,
            }
        }
    }

    /// Closes whichever collected element is open at `depth`.
    pub(super) fn end(&mut self, depth: u64) {
        let Some((container_depth, _)) = self.container else {
            return;
        };
        if let Some(pending) = &mut self.pending {
            if pending
                .capture
                .as_ref()
                .is_some_and(|(capture_depth, _, _)| *capture_depth == depth)
            {
                let (_, slot, text) = pending.capture.take().expect("open capture");
                pending.store(slot, text);
            } else if pending
                .formula_parent
                .is_some_and(|(parent_depth, _)| parent_depth == depth)
            {
                pending.formula_parent = None;
            } else if pending.depth == depth {
                let pending = self.pending.take().expect("open validation");
                self.validations.extend(pending.finish());
            }
        } else if container_depth == depth {
            self.container = None;
        }
    }

    pub(super) fn finish(self) -> Vec<DataValidation> {
        self.validations
    }
}

fn parse_rule(attributes: &XmlAttributes) -> Option<DataValidationRule> {
    let token = |name, default| attributes.unqualified(name).unwrap_or(default);
    let flag = |name| match attributes.unqualified(name) {
        None | Some("0" | "false") => Some(false),
        Some("1" | "true") => Some(true),
        Some(_) => None,
    };
    let text = |name| attributes.unqualified(name).map(Box::from);
    Some(DataValidationRule {
        validation_type: DataValidationType::from_xlsx(token("type", "none"))?,
        operator: DataValidationOperator::from_xlsx(token("operator", "between"))?,
        error_style: DataValidationErrorStyle::from_xlsx(token("errorStyle", "stop"))?,
        allow_blank: flag("allowBlank")?,
        in_cell_dropdown: !flag("showDropDown")?,
        show_input_message: flag("showInputMessage")?,
        show_error_message: flag("showErrorMessage")?,
        error_title: text("errorTitle"),
        error: text("error"),
        prompt_title: text("promptTitle"),
        prompt: text("prompt"),
    })
}

/// Parses an `sqref` list of space-separated cell and range references.
fn parse_ranges(sqref: &str) -> Option<Vec<CellRange>> {
    let ranges = sqref
        .split_ascii_whitespace()
        .map(parse_reference)
        .collect::<Option<Vec<_>>>()?;
    (!ranges.is_empty()).then_some(ranges)
}
