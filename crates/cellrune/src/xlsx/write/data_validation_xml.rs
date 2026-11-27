//! Writes a sheet's data validation rules: `<dataValidations>` for rules whose formulas stay on
//! the sheet, and Excel 2010's `<x14:dataValidations>` extension for rules whose formulas name
//! another sheet or workbook, which is the form Excel writes for them.

use super::grid_xml::{XmlDocument, XmlElement, XmlNode};
use super::serialization::{escape_attribute, escape_text};
use super::table_patch::range_text;
use super::{WriteLimits, XlsxWriteError};
use crate::calculation::formula_references::formula_references;
use crate::workbook::{DataValidationRule, parse_sqref};
use crate::xlsx::package::PartPath;
use crate::{
    CellRange, DataValidation, DataValidationErrorStyle, DataValidationOperator,
    DataValidationType, Sheet, SheetName,
};

const MAIN_NAMESPACE: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const X14_NAMESPACE: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const XM_NAMESPACE: &str = "http://schemas.microsoft.com/office/excel/2006/main";
/// The `ext` URI under which Excel keeps `x14:dataValidations`.
const EXTENSION_URI: &str = "{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}";

/// Worksheet elements that follow `<dataValidations>` in CT_Worksheet order, before the first of
/// which a new `<dataValidations>` goes.
const AFTER_DATA_VALIDATIONS: &[&str] = &[
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

/// Appends a new sheet's `<dataValidations>`, the rules whose formulas stay on the sheet.
pub(super) fn push_data_validations(xml: &mut String, sheet: &Sheet) -> Result<(), XlsxWriteError> {
    let rules = sheet
        .data_validations()
        .iter()
        .filter(|validation| !is_extension_rule(validation, sheet.name()))
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return Ok(());
    }
    xml.push_str(&format!("<dataValidations count=\"{}\">", rules.len()));
    for validation in rules {
        xml.push_str(&main_element_xml(validation, "")?);
    }
    xml.push_str("</dataValidations>");
    Ok(())
}

/// Appends a new sheet's `<extLst>` holding the rules whose formulas name another sheet or
/// workbook, the last element of a worksheet.
pub(super) fn push_extension_validations(
    xml: &mut String,
    sheet: &Sheet,
) -> Result<(), XlsxWriteError> {
    let rules = sheet
        .data_validations()
        .iter()
        .filter(|validation| is_extension_rule(validation, sheet.name()))
        .collect::<Vec<_>>();
    if rules.is_empty() {
        return Ok(());
    }
    xml.push_str("<extLst>");
    xml.push_str(&extension_xml("", &rules)?);
    xml.push_str("</extLst>");
    Ok(())
}

/// Brings the data validation rules a worksheet part holds in line with `sheet`'s.
///
/// An element holding a rule of the sheet, ranges included, stays as it is. An element holding a
/// rule of the sheet over other ranges keeps its markup and takes the new ranges, so attributes
/// the model does not read, such as `imeMode`, stay. A rule no element holds becomes a new
/// element, and an element holding a rule the sheet no longer has goes. An element the reader
/// could not model stays. Returns `None` when the part already holds the sheet's rules.
pub(super) fn patch_data_validations(
    bytes: &[u8],
    part: &PartPath,
    sheet: &Sheet,
    limits: WriteLimits,
) -> Result<Option<Vec<u8>>, XlsxWriteError> {
    let mut document = XmlDocument::parse(bytes, part, limits)?;
    let root = document.root_mut();
    let main_container = main_container_index(root);
    let mut slots = Vec::new();
    if let Some(index) = main_container {
        collect_slots(element_at(root, index), Container::Main, part, &mut slots)?;
    }
    let extension_containers = extension_container_paths(root);
    for (position, path) in extension_containers.iter().enumerate() {
        collect_slots(
            element_at_path(root, path),
            Container::Extension(position),
            part,
            &mut slots,
        )?;
    }

    let model = sheet.data_validations();
    let mut actions = vec![Action::Keep; slots.len()];
    let mut used = vec![false; slots.len()];
    let mut placed = vec![false; model.len()];
    for (rule, validation) in model.iter().enumerate() {
        if let Some(slot) = (0..slots.len())
            .find(|slot| !used[*slot] && slots[*slot].validation.as_ref() == Some(validation))
        {
            used[slot] = true;
            placed[rule] = true;
        }
    }
    for (rule, validation) in model.iter().enumerate() {
        if placed[rule] {
            continue;
        }
        if let Some(slot) = (0..slots.len()).find(|slot| {
            !used[*slot]
                && slots[*slot]
                    .validation
                    .as_ref()
                    .is_some_and(|held| held.same_rule(validation))
        }) {
            used[slot] = true;
            placed[rule] = true;
            actions[slot] = Action::Ranges(validation.ranges().to_vec());
        }
    }
    for (slot, held) in slots.iter().enumerate() {
        if held.validation.is_some() && !used[slot] {
            actions[slot] = Action::Remove;
        }
    }
    let (new_extension, new_main): (Vec<_>, Vec<_>) = model
        .iter()
        .zip(&placed)
        .filter(|(_, placed)| !**placed)
        .map(|(validation, _)| validation)
        .partition(|validation| is_extension_rule(validation, sheet.name()));
    if new_main.is_empty()
        && new_extension.is_empty()
        && actions.iter().all(|action| matches!(action, Action::Keep))
    {
        return Ok(None);
    }

    let prefix = root.prefix();
    // Containers are visited from the last in document order, so that removing one keeps the
    // paths of those before it; the main container is found again afterwards.
    for (position, path) in extension_containers.iter().enumerate().rev() {
        let container = element_at_path_mut(root, path);
        apply_actions(
            container,
            Container::Extension(position),
            &slots,
            &actions,
            part,
        )?;
        if position == 0 {
            let x14 = container.prefix();
            for validation in &new_extension {
                container.children.push(XmlNode::Element(fragment(
                    &extension_element_xml(validation, &x14)?,
                    part,
                    limits,
                )?));
            }
        }
        if finish_container(container, part)? {
            remove_with_empty_ancestors(root, path);
        }
    }
    if extension_containers.is_empty() && !new_extension.is_empty() {
        let ext = fragment(&extension_xml(&prefix, &new_extension)?, part, limits)?;
        match root
            .children
            .iter()
            .position(|node| matches!(node, XmlNode::Element(element) if element.is("extLst")))
        {
            Some(index) => element_at_mut(root, index)
                .children
                .push(XmlNode::Element(ext)),
            None => {
                let mut list =
                    fragment(&format!("<{prefix}extLst></{prefix}extLst>"), part, limits)?;
                list.children = vec![XmlNode::Element(ext)];
                root.children.push(XmlNode::Element(list));
            }
        }
    }
    if let Some(index) = main_container_index(root) {
        let container = element_at_mut(root, index);
        apply_actions(container, Container::Main, &slots, &actions, part)?;
        let main = container.prefix();
        for validation in &new_main {
            container.children.push(XmlNode::Element(fragment(
                &main_element_xml(validation, &main)?,
                part,
                limits,
            )?));
        }
        if finish_container(container, part)? {
            root.children.remove(index);
        }
    } else if !new_main.is_empty() {
        let mut xml = format!("<{prefix}dataValidations count=\"{}\">", new_main.len());
        for validation in &new_main {
            xml.push_str(&main_element_xml(validation, &prefix)?);
        }
        xml.push_str(&format!("</{prefix}dataValidations>"));
        let container = fragment(&xml, part, limits)?;
        let at = root
            .children
            .iter()
            .position(|node| {
                matches!(node, XmlNode::Element(element)
                    if AFTER_DATA_VALIDATIONS.iter().any(|name| element.is(name)))
            })
            .unwrap_or(root.children.len());
        root.children.insert(at, XmlNode::Element(container));
    }

    document.write(part, limits).map(Some)
}

/// Whether a rule goes to the Excel 2010 extension: a formula names another sheet or workbook.
fn is_extension_rule(validation: &DataValidation, sheet: &SheetName) -> bool {
    [validation.formula1(), validation.formula2()]
        .into_iter()
        .flatten()
        .filter_map(formula_references)
        .any(|references| {
            references.external
                || references.sheets.iter().any(|name| {
                    SheetName::new(name.as_str())
                        .map_or(true, |name| name.lookup_key() != sheet.lookup_key())
                })
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Main,
    /// The n-th `x14:dataValidations` in document order.
    Extension(usize),
}

/// One `dataValidation` child of a container, and the rule it holds when the reader models it.
struct Slot {
    container: Container,
    /// Position among the container's children.
    child: usize,
    validation: Option<DataValidation>,
}

#[derive(Debug, Clone)]
enum Action {
    Keep,
    Ranges(Vec<CellRange>),
    Remove,
}

fn collect_slots(
    container: &XmlElement,
    kind: Container,
    part: &PartPath,
    slots: &mut Vec<Slot>,
) -> Result<(), XlsxWriteError> {
    for (child, node) in container.children.iter().enumerate() {
        if let XmlNode::Element(element) = node
            && element.is("dataValidation")
        {
            slots.push(Slot {
                container: kind,
                child,
                validation: element_validation(element, kind != Container::Main, part)?,
            });
        }
    }
    Ok(())
}

/// Reads an element as the reader does, `None` for one it leaves unmodeled.
fn element_validation(
    element: &XmlElement,
    extension: bool,
    part: &PartPath,
) -> Result<Option<DataValidation>, XlsxWriteError> {
    let attributes = element.attributes(part)?;
    let attribute = |name: &str| {
        attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let Some(rule) = DataValidationRule::from_attributes(attribute) else {
        return Ok(None);
    };
    let single = |local_name: &str| {
        let mut matching = element.elements().filter(|child| child.is(local_name));
        let first = matching.next();
        if matching.next().is_some() {
            Err(())
        } else {
            Ok(first)
        }
    };
    let mut formulas = [None, None];
    for (slot, local_name) in ["formula1", "formula2"].into_iter().enumerate() {
        let Ok(holder) = single(local_name) else {
            return Ok(None);
        };
        let Some(holder) = holder else {
            continue;
        };
        let holder = if extension {
            let mut inner = holder.elements().filter(|child| child.is("f"));
            match (inner.next(), inner.next()) {
                (Some(inner), None) => inner,
                _ => return Ok(None),
            }
        } else {
            holder
        };
        let Some(text) = holder.text(part)? else {
            return Ok(None);
        };
        formulas[slot] = Some(text.into_boxed_str());
    }
    let sqref = if extension {
        match single("sqref") {
            Ok(Some(holder)) => holder.text(part)?,
            _ => return Ok(None),
        }
    } else {
        attribute("sqref").map(str::to_owned)
    };
    let Some(ranges) = sqref.as_deref().and_then(parse_sqref) else {
        return Ok(None);
    };
    let [formula1, formula2] = formulas;
    Ok(Some(DataValidation::from_xlsx(
        ranges, rule, formula1, formula2,
    )))
}

/// Applies the actions of the container's slots, then sets its `count`.
fn apply_actions(
    container: &mut XmlElement,
    kind: Container,
    slots: &[Slot],
    actions: &[Action],
    part: &PartPath,
) -> Result<(), XlsxWriteError> {
    let mut removed = Vec::new();
    for (slot, action) in slots.iter().zip(actions) {
        if slot.container != kind {
            continue;
        }
        match action {
            Action::Keep => {}
            Action::Remove => removed.push(slot.child),
            Action::Ranges(ranges) => {
                let XmlNode::Element(element) = &mut container.children[slot.child] else {
                    unreachable!("a slot names an element child");
                };
                let text = sqref_text(ranges);
                if kind == Container::Main {
                    element.set_attribute("sqref", Some(&text), part)?;
                } else if let Some(holder) = element.child_mut("sqref") {
                    holder.set_text(&text)?;
                }
            }
        }
    }
    for child in removed.into_iter().rev() {
        container.children.remove(child);
    }
    Ok(())
}

/// Sets a container's `count` to its rules and returns whether it holds none, so that it goes.
fn finish_container(container: &mut XmlElement, part: &PartPath) -> Result<bool, XlsxWriteError> {
    let count = container
        .elements()
        .filter(|element| element.is("dataValidation"))
        .count();
    if count == 0 {
        return Ok(true);
    }
    container.set_attribute("count", Some(&count.to_string()), part)?;
    Ok(false)
}

/// The position among the root's children of the main `<dataValidations>`.
fn main_container_index(root: &XmlElement) -> Option<usize> {
    root.children
        .iter()
        .position(|node| matches!(node, XmlNode::Element(element) if element.is("dataValidations")))
}

/// The path, as indexes of children from the root, of each `x14:dataValidations` in document
/// order: every `dataValidations` below the root's own children, wherever an extension or
/// markup-compatibility wrapper holds it, as the reader finds it.
fn extension_container_paths(root: &XmlElement) -> Vec<Vec<usize>> {
    fn visit(element: &XmlElement, path: &mut Vec<usize>, paths: &mut Vec<Vec<usize>>) {
        for (index, node) in element.children.iter().enumerate() {
            let XmlNode::Element(child) = node else {
                continue;
            };
            path.push(index);
            if child.is("dataValidations") {
                paths.push(path.clone());
            } else {
                visit(child, path, paths);
            }
            path.pop();
        }
    }
    let mut paths = Vec::new();
    for (index, node) in root.children.iter().enumerate() {
        if let XmlNode::Element(child) = node
            && !child.is("dataValidations")
        {
            visit(child, &mut vec![index], &mut paths);
        }
    }
    paths
}

fn element_at_path<'a>(root: &'a XmlElement, path: &[usize]) -> &'a XmlElement {
    path.iter()
        .fold(root, |element, index| element_at(element, *index))
}

fn element_at_path_mut<'a>(root: &'a mut XmlElement, path: &[usize]) -> &'a mut XmlElement {
    path.iter()
        .fold(root, |element, index| element_at_mut(element, *index))
}

/// Removes an emptied `x14:dataValidations`, then each wrapper above it, such as its `ext` and
/// `extLst`, that is left without elements.
fn remove_with_empty_ancestors(root: &mut XmlElement, path: &[usize]) {
    for depth in (1..=path.len()).rev() {
        let (index, parent) = (path[depth - 1], &path[..depth - 1]);
        let parent = element_at_path_mut(root, parent);
        let empty = match &parent.children[index] {
            XmlNode::Element(element) => depth == path.len() || element.elements().next().is_none(),
            XmlNode::Other(_) => false,
        };
        if !empty {
            return;
        }
        parent.children.remove(index);
    }
}

fn element_at(parent: &XmlElement, index: usize) -> &XmlElement {
    match &parent.children[index] {
        XmlNode::Element(element) => element,
        XmlNode::Other(_) => unreachable!("the index names an element child"),
    }
}

fn element_at_mut(parent: &mut XmlElement, index: usize) -> &mut XmlElement {
    match &mut parent.children[index] {
        XmlNode::Element(element) => element,
        XmlNode::Other(_) => unreachable!("the index names an element child"),
    }
}

/// Parses one element written with `prefix`-qualified names, declaring the namespaces it may use.
fn fragment(xml: &str, part: &PartPath, limits: WriteLimits) -> Result<XmlElement, XlsxWriteError> {
    let wrapped = format!(
        "<fragment xmlns=\"{MAIN_NAMESPACE}\" xmlns:x14=\"{X14_NAMESPACE}\" xmlns:xm=\"{XM_NAMESPACE}\">{xml}</fragment>"
    );
    let mut wrapper = XmlDocument::parse(wrapped.as_bytes(), part, limits)?.into_root();
    let element = wrapper
        .children
        .drain(..)
        .find_map(|node| match node {
            XmlNode::Element(element) => Some(element),
            XmlNode::Other(_) => None,
        })
        .expect("a fragment holds its element");
    Ok(element)
}

/// The `ext` holding a new `x14:dataValidations`, its names qualified with `prefix`.
fn extension_xml(prefix: &str, rules: &[&DataValidation]) -> Result<String, XlsxWriteError> {
    let mut xml = format!(
        "<{prefix}ext uri=\"{EXTENSION_URI}\" xmlns:x14=\"{X14_NAMESPACE}\"><x14:dataValidations xmlns:xm=\"{XM_NAMESPACE}\" count=\"{}\">",
        rules.len()
    );
    for validation in rules {
        xml.push_str(&extension_element_xml(validation, "x14:")?);
    }
    xml.push_str(&format!("</x14:dataValidations></{prefix}ext>"));
    Ok(xml)
}

fn main_element_xml(validation: &DataValidation, prefix: &str) -> Result<String, XlsxWriteError> {
    let mut xml = format!("<{prefix}dataValidation");
    push_attributes(&mut xml, validation)?;
    xml.push_str(" sqref=\"");
    xml.push_str(&sqref_text(validation.ranges()));
    xml.push_str("\">");
    for (name, formula) in [
        ("formula1", validation.formula1()),
        ("formula2", validation.formula2()),
    ] {
        if let Some(formula) = formula {
            xml.push_str(&format!(
                "<{prefix}{name}>{}</{prefix}{name}>",
                escape_text(formula)?
            ));
        }
    }
    xml.push_str(&format!("</{prefix}dataValidation>"));
    Ok(xml)
}

/// An `x14:dataValidation`, which declares the `xm` namespace of its formulas and ranges itself.
fn extension_element_xml(validation: &DataValidation, x14: &str) -> Result<String, XlsxWriteError> {
    let mut xml = format!("<{x14}dataValidation xmlns:xm=\"{XM_NAMESPACE}\"");
    push_attributes(&mut xml, validation)?;
    xml.push('>');
    for (name, formula) in [
        ("formula1", validation.formula1()),
        ("formula2", validation.formula2()),
    ] {
        if let Some(formula) = formula {
            xml.push_str(&format!(
                "<{x14}{name}><xm:f>{}</xm:f></{x14}{name}>",
                escape_text(formula)?
            ));
        }
    }
    xml.push_str("<xm:sqref>");
    xml.push_str(&sqref_text(validation.ranges()));
    xml.push_str(&format!("</xm:sqref></{x14}dataValidation>"));
    Ok(xml)
}

/// Writes the rule's attributes, leaving out those at their OOXML default.
fn push_attributes(xml: &mut String, validation: &DataValidation) -> Result<(), XlsxWriteError> {
    if validation.validation_type() != DataValidationType::AnyValue {
        xml.push_str(&format!(
            " type=\"{}\"",
            validation.validation_type().as_str()
        ));
    }
    if validation.error_style() != DataValidationErrorStyle::Stop {
        xml.push_str(&format!(
            " errorStyle=\"{}\"",
            validation.error_style().as_str()
        ));
    }
    if validation.operator() != DataValidationOperator::Between {
        xml.push_str(&format!(" operator=\"{}\"", validation.operator().as_str()));
    }
    for (name, set) in [
        ("allowBlank", validation.allow_blank()),
        ("showDropDown", !validation.in_cell_dropdown()),
        ("showInputMessage", validation.show_input_message()),
        ("showErrorMessage", validation.show_error_message()),
    ] {
        if set {
            xml.push_str(&format!(" {name}=\"1\""));
        }
    }
    for (name, text) in [
        ("errorTitle", validation.error_title()),
        ("error", validation.error_message()),
        ("promptTitle", validation.prompt_title()),
        ("prompt", validation.prompt_message()),
    ] {
        if let Some(text) = text {
            xml.push_str(&format!(" {name}=\"{}\"", escape_attribute(text)?));
        }
    }
    Ok(())
}

fn sqref_text(ranges: &[CellRange]) -> String {
    ranges
        .iter()
        .map(|range| range_text(*range))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "data_validation_xml_tests.rs"]
mod tests;
