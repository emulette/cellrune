use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;

use super::shared;

const MESSAGE_ARRAY_FORMULA_RESULT_REF: &str = "array formula is missing its result ref";
const MESSAGE_ABSOLUTE_PATH_PROVENANCE: &str =
    "workbook contains environment-specific absolute-path provenance";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RawRichError {
    pub(super) record_index: u32,
    pub(super) structure_index: u32,
    pub(super) error_type_code: Option<u32>,
    pub(super) resolved_error: Option<String>,
    pub(super) fallback_error: Option<String>,
}

#[derive(Debug)]
pub(super) struct RawFormulaCell {
    pub(super) formula: String,
    pub(super) array_result_ref: Option<String>,
    pub(super) value: Option<String>,
    pub(super) value_type: String,
    pub(super) vm_index: Option<u32>,
    pub(super) rich_error: Option<RawRichError>,
}

#[derive(Debug)]
pub(super) struct RawCachedCell {
    pub(super) value: Option<String>,
    pub(super) value_type: String,
    pub(super) vm_index: Option<u32>,
    pub(super) rich_error: Option<RawRichError>,
}

#[derive(Debug)]
pub(super) struct RawWorkbookProvenance {
    pub(super) application: String,
    pub(super) app_version: String,
    pub(super) modified_at: String,
    pub(super) rup_build: String,
}

#[derive(Debug)]
struct PendingFormulaCell {
    address: String,
    formula: String,
    formula_attributes: String,
    array_result_ref: Option<String>,
    value: Option<String>,
    value_type: String,
    vm_index: Option<u32>,
}

pub(super) fn read_formula_cells(
    workbook_path: &Path,
) -> Result<BTreeMap<String, RawFormulaCell>, String> {
    let file = File::open(workbook_path)
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let workbook_xml = read_member(&mut archive, "xl/workbook.xml")?;
    let relationships_xml = read_member(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let relationships = relationship_targets(&relationships_xml)?;
    let sheets = workbook_sheets(&workbook_xml)?;
    let rich_errors = read_rich_errors(&mut archive)?;
    let shared_strings = read_shared_strings(&mut archive)?;
    let mut formulas = BTreeMap::new();
    for (sheet_name, relationship_id) in sheets {
        let target = relationships
            .get(&relationship_id)
            .ok_or_else(|| format!("missing worksheet relationship {relationship_id}"))?;
        let member = normalize_workbook_target(target)?;
        let xml = read_member(&mut archive, &member)?;
        parse_formula_cells(
            &sheet_name,
            &xml,
            &rich_errors,
            &shared_strings,
            &mut formulas,
        )?;
    }
    Ok(formulas)
}

pub(super) fn read_cached_cells(
    workbook_path: &Path,
) -> Result<BTreeMap<String, RawCachedCell>, String> {
    let file = File::open(workbook_path)
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let workbook_xml = read_member(&mut archive, "xl/workbook.xml")?;
    let relationships_xml = read_member(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let relationships = relationship_targets(&relationships_xml)?;
    let sheets = workbook_sheets(&workbook_xml)?;
    let rich_errors = read_rich_errors(&mut archive)?;
    let shared_strings = read_shared_strings(&mut archive)?;
    let mut cells = BTreeMap::new();
    for (sheet_name, relationship_id) in sheets {
        let target = relationships
            .get(&relationship_id)
            .ok_or_else(|| format!("missing worksheet relationship {relationship_id}"))?;
        let member = normalize_workbook_target(target)?;
        let xml = read_member(&mut archive, &member)?;
        parse_cached_cells(&sheet_name, &xml, &rich_errors, &shared_strings, &mut cells)?;
    }
    Ok(cells)
}

pub(super) fn read_workbook_provenance(
    workbook_path: &Path,
) -> Result<RawWorkbookProvenance, String> {
    let file = File::open(workbook_path)
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let app_xml = read_member(&mut archive, "docProps/app.xml")?;
    let core_xml = read_member(&mut archive, "docProps/core.xml")?;
    let workbook_xml = read_member(&mut archive, "xl/workbook.xml")?;
    let text = |xml: &str, tag: &str| {
        first_element(xml, tag)
            .map(|(_, value)| unescape_xml(value))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "{}: workbook provenance lacks {tag}",
                    workbook_path.display()
                )
            })
    };
    let file_version = opening_tags(&workbook_xml, "fileVersion")
        .into_iter()
        .next()
        .ok_or_else(|| {
            format!(
                "{}: workbook provenance lacks fileVersion",
                workbook_path.display()
            )
        })?;
    let rup_build = attribute(file_version, "rupBuild")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "{}: workbook provenance lacks rupBuild",
                workbook_path.display()
            )
        })?
        .to_owned();
    Ok(RawWorkbookProvenance {
        application: text(&app_xml, "Application")?,
        app_version: text(&app_xml, "AppVersion")?,
        modified_at: text(&core_xml, "dcterms:modified")?,
        rup_build,
    })
}

pub(super) fn verify_package_invariants(workbook_path: &Path) -> Result<(), String> {
    let file = File::open(workbook_path)
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let mut external_parts = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
        let name = entry.name();
        if name.starts_with("xl/externalLinks/") || name == "xl/externalLinks.xml" {
            external_parts.push(name.to_owned());
        }
    }
    if !external_parts.is_empty() {
        return Err(format!(
            "{}: workbook contains external-link parts {:?}",
            workbook_path.display(),
            external_parts
        ));
    }
    let workbook_xml = read_member(&mut archive, "xl/workbook.xml")?;
    if opening_tags(&workbook_xml, "calcPr").iter().any(|tag| {
        attribute(tag, "iterate") == Some("1") || attribute(tag, "forceFullCalc") == Some("1")
    }) {
        return Err(format!(
            "{}: workbook enables iterative or forced-full calculation",
            workbook_path.display()
        ));
    }
    if workbook_xml.contains("<externalReferences") {
        return Err(format!(
            "{}: workbook declares external references",
            workbook_path.display()
        ));
    }
    Ok(())
}

pub(super) fn verify_no_absolute_path_provenance(workbook_path: &Path) -> Result<(), String> {
    let file = File::open(workbook_path)
        .map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("{}: {error}", workbook_path.display()))?;
    let workbook_xml = read_member(&mut archive, "xl/workbook.xml")?;
    if workbook_xml.contains(":absPath") {
        return Err(format!(
            "{}: {MESSAGE_ABSOLUTE_PATH_PROVENANCE}",
            workbook_path.display()
        ));
    }
    Ok(())
}

fn read_member(archive: &mut ZipArchive<File>, member: &str) -> Result<String, String> {
    let mut entry = archive
        .by_name(member)
        .map_err(|error| format!("{member}: {error}"))?;
    let mut value = String::new();
    entry
        .read_to_string(&mut value)
        .map_err(|error| format!("{member}: {error}"))?;
    Ok(value)
}

fn read_optional_member(
    archive: &mut ZipArchive<File>,
    member: &str,
) -> Result<Option<String>, String> {
    match archive.by_name(member) {
        Ok(mut entry) => {
            let mut value = String::new();
            entry
                .read_to_string(&mut value)
                .map_err(|error| format!("{member}: {error}"))?;
            Ok(Some(value))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(error) => Err(format!("{member}: {error}")),
    }
}

fn relationship_targets(xml: &str) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    for tag in opening_tags(xml, "Relationship") {
        let Some(id) = attribute(tag, "Id") else {
            continue;
        };
        let Some(target) = attribute(tag, "Target") else {
            continue;
        };
        if attribute(tag, "Type").is_some_and(|value| value.ends_with("/worksheet")) {
            if attribute(tag, "TargetMode") == Some("External") {
                return Err(format!("worksheet relationship {id} is external"));
            }
            if result.insert(id.to_owned(), target.to_owned()).is_some() {
                return Err(format!("duplicate worksheet relationship id {id}"));
            }
        }
    }
    Ok(result)
}

fn workbook_sheets(xml: &str) -> Result<Vec<(String, String)>, String> {
    let mut result = Vec::new();
    for tag in opening_tags(xml, "sheet") {
        let Some(name) = attribute(tag, "name") else {
            continue;
        };
        let Some(relationship_id) = attribute(tag, "r:id") else {
            continue;
        };
        result.push((unescape_xml(name), relationship_id.to_owned()));
    }
    if result.is_empty() {
        Err("workbook contains no resolvable worksheets".to_owned())
    } else {
        Ok(result)
    }
}

fn normalize_workbook_target(target: &str) -> Result<String, String> {
    let target = target.trim_start_matches('/');
    let target = target.strip_prefix("xl/").unwrap_or(target);
    if target.is_empty() || target.split('/').any(|segment| segment == "..") {
        return Err(format!("worksheet target escapes xl/: {target}"));
    }
    Ok(format!("xl/{target}"))
}

fn parse_formula_cells(
    sheet_name: &str,
    xml: &str,
    rich_errors: &BTreeMap<u32, RawRichError>,
    shared_strings: &[String],
    output: &mut BTreeMap<String, RawFormulaCell>,
) -> Result<(), String> {
    let mut pending = Vec::new();
    for (opening, body) in element_blocks(xml, "c") {
        let Some(address) = attribute(opening, "r") else {
            continue;
        };
        let Some((formula_tag, formula_value)) = first_element(body, "f") else {
            continue;
        };
        let key = format!("{sheet_name}!{address}");
        let (value, value_type) = cached_cell_value(opening, body, shared_strings, &key)?;
        let vm_index = attribute(opening, "vm")
            .map(str::parse::<u32>)
            .transpose()
            .map_err(|error| format!("{key}: invalid vm index: {error}"))?;
        let array_result_ref = if attribute(formula_tag, "t") == Some("array") {
            Some(
                attribute(formula_tag, "ref")
                    .ok_or_else(|| format!("{key}: {MESSAGE_ARRAY_FORMULA_RESULT_REF}"))?
                    .to_owned(),
            )
        } else {
            None
        };
        pending.push(PendingFormulaCell {
            address: address.to_owned(),
            formula: unescape_xml(formula_value),
            formula_attributes: formula_tag.to_owned(),
            array_result_ref,
            value,
            value_type,
            vm_index,
        });
    }
    let mut anchors = BTreeMap::new();
    for cell in &pending {
        if let Some(shared_index) = attribute(&cell.formula_attributes, "si")
            && !cell.formula.is_empty()
            && anchors
                .insert(
                    shared_index.to_owned(),
                    (cell.address.clone(), cell.formula.clone()),
                )
                .is_some()
        {
            return Err(format!(
                "{sheet_name}!{}: duplicate shared-formula anchor {shared_index}",
                cell.address
            ));
        }
    }
    for cell in pending {
        let formula = if let Some(shared_index) = attribute(&cell.formula_attributes, "si")
            && cell.formula.is_empty()
        {
            let (anchor_address, anchor_formula) = anchors.get(shared_index).ok_or_else(|| {
                format!(
                    "{sheet_name}!{}: shared-formula follower {shared_index} has no anchor",
                    cell.address
                )
            })?;
            shared::translate(anchor_formula, anchor_address, &cell.address)?
        } else {
            cell.formula
        };
        let key = format!("{sheet_name}!{}", cell.address);
        let rich_error =
            cell.vm_index
                .map(|index| {
                    rich_errors.get(&index).cloned().ok_or_else(|| {
                        format!("{key}: vm={index} does not resolve to rich metadata")
                    })
                })
                .transpose()?;
        if rich_error.is_some() && cell.value_type != "e" {
            return Err(format!(
                "{key}: rich metadata is attached to a non-error cell"
            ));
        }
        let raw = RawFormulaCell {
            formula,
            array_result_ref: cell.array_result_ref,
            value: cell.value,
            value_type: cell.value_type,
            vm_index: cell.vm_index,
            rich_error,
        };
        if output.insert(key.clone(), raw).is_some() {
            return Err(format!("duplicate formula cell {key}"));
        }
    }
    Ok(())
}

fn parse_cached_cells(
    sheet_name: &str,
    xml: &str,
    rich_errors: &BTreeMap<u32, RawRichError>,
    shared_strings: &[String],
    output: &mut BTreeMap<String, RawCachedCell>,
) -> Result<(), String> {
    for (opening, body) in element_blocks(xml, "c") {
        let Some(address) = attribute(opening, "r") else {
            continue;
        };
        let key = format!("{sheet_name}!{address}");
        let (value, value_type) = cached_cell_value(opening, body, shared_strings, &key)?;
        let vm_index = attribute(opening, "vm")
            .map(str::parse::<u32>)
            .transpose()
            .map_err(|error| format!("{key}: invalid vm index: {error}"))?;
        let rich_error = vm_index
            .map(|index| {
                rich_errors
                    .get(&index)
                    .cloned()
                    .ok_or_else(|| format!("{key}: vm={index} does not resolve to rich metadata"))
            })
            .transpose()?;
        if rich_error.is_some() && value_type != "e" {
            return Err(format!(
                "{key}: rich metadata is attached to a non-error cell"
            ));
        }
        let raw = RawCachedCell {
            value,
            value_type,
            vm_index,
            rich_error,
        };
        if output.insert(key.clone(), raw).is_some() {
            return Err(format!("duplicate cached cell {key}"));
        }
    }
    Ok(())
}

fn read_shared_strings(archive: &mut ZipArchive<File>) -> Result<Vec<String>, String> {
    let Some(xml) = read_optional_member(archive, "xl/sharedStrings.xml")? else {
        return Ok(Vec::new());
    };
    Ok(element_blocks(&xml, "si")
        .into_iter()
        .map(|(_, body)| {
            element_blocks(body, "t")
                .into_iter()
                .map(|(_, text)| unescape_xml(text))
                .collect::<String>()
        })
        .collect())
}

fn cached_cell_value(
    opening: &str,
    body: &str,
    shared_strings: &[String],
    context: &str,
) -> Result<(Option<String>, String), String> {
    let value_type = attribute(opening, "t").unwrap_or("n");
    if value_type == "inlineStr" {
        let value = body.contains("<is").then(|| {
            element_blocks(body, "t")
                .into_iter()
                .map(|(_, text)| unescape_xml(text))
                .collect::<String>()
        });
        return Ok((value, "str".to_owned()));
    }
    let value = if body.contains("<v/>") || body.contains("<v />") {
        Some(String::new())
    } else {
        first_element(body, "v").map(|(_, value)| unescape_xml(value))
    };
    if value_type != "s" || value.is_none() {
        return Ok((value, value_type.to_owned()));
    }
    let raw = value.expect("shared-string value presence checked");
    let index = raw
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("{context}: invalid shared-string index: {error}"))?;
    let value = shared_strings
        .get(index)
        .cloned()
        .ok_or_else(|| format!("{context}: shared-string index {index} is out of bounds"))?;
    Ok((Some(value), "str".to_owned()))
}

fn read_rich_errors(archive: &mut ZipArchive<File>) -> Result<BTreeMap<u32, RawRichError>, String> {
    let Some(metadata) = read_optional_member(archive, "xl/metadata.xml")? else {
        return Ok(BTreeMap::new());
    };
    let metadata_types = opening_tags(&metadata, "metadataType");
    let Some(rich_type_index) = metadata_types
        .iter()
        .position(|tag| attribute(tag, "name") == Some("XLRICHVALUE"))
        .map(|index| index as u32 + 1)
    else {
        return Ok(BTreeMap::new());
    };
    let future = element_blocks(&metadata, "futureMetadata")
        .into_iter()
        .find(|(tag, _)| attribute(tag, "name") == Some("XLRICHVALUE"))
        .ok_or_else(|| "XLRICHVALUE futureMetadata is missing".to_owned())?;
    let value_metadata = element_blocks(&metadata, "valueMetadata")
        .into_iter()
        .next()
        .ok_or_else(|| "XLRICHVALUE valueMetadata is missing".to_owned())?;
    let future_to_record = element_blocks(future.1, "bk")
        .into_iter()
        .enumerate()
        .map(|(index, (_, body))| {
            let tag = opening_tags_with_suffix(body, "rvb")
                .into_iter()
                .next()
                .ok_or_else(|| format!("rich future metadata {index} lacks rvb"))?;
            attribute(tag, "i")
                .ok_or_else(|| format!("rich future metadata {index} lacks rvb index"))?
                .parse::<u32>()
                .map_err(|error| format!("rich future metadata {index}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let vm_to_future = element_blocks(value_metadata.1, "bk")
        .into_iter()
        .enumerate()
        .map(|(index, (_, body))| {
            let tag = opening_tags(body, "rc")
                .into_iter()
                .next()
                .ok_or_else(|| format!("valueMetadata {index} lacks rc"))?;
            let record_type = attribute(tag, "t")
                .ok_or_else(|| format!("valueMetadata {index} lacks type"))?
                .parse::<u32>()
                .map_err(|error| format!("valueMetadata {index}: {error}"))?;
            let value = attribute(tag, "v")
                .ok_or_else(|| format!("valueMetadata {index} lacks value"))?
                .parse::<usize>()
                .map_err(|error| format!("valueMetadata {index}: {error}"))?;
            if record_type != rich_type_index || value >= future_to_record.len() {
                return Err(format!("valueMetadata {index} has an invalid rich mapping"));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let structures_xml = read_member(archive, "xl/richData/rdrichvaluestructure.xml")?;
    let structures = element_blocks(&structures_xml, "s")
        .into_iter()
        .enumerate()
        .map(|(index, (tag, body))| {
            let structure_type = attribute(tag, "t").unwrap_or("").to_owned();
            let keys = opening_tags(body, "k")
                .into_iter()
                .map(|key| {
                    (
                        attribute(key, "n").unwrap_or("").to_owned(),
                        attribute(key, "t").unwrap_or("").to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            (index as u32, structure_type, keys)
        })
        .collect::<Vec<_>>();
    let values_xml = read_member(archive, "xl/richData/rdrichvalue.xml")?;
    let records = element_blocks(&values_xml, "rv")
        .into_iter()
        .enumerate()
        .map(|(record_index, (tag, body))| {
            let structure_index = attribute(tag, "s")
                .ok_or_else(|| format!("rich record {record_index} lacks structure"))?
                .parse::<usize>()
                .map_err(|error| format!("rich record {record_index}: {error}"))?;
            let Some((_, structure_type, keys)) = structures.get(structure_index) else {
                return Err(format!("rich record {record_index} has invalid structure"));
            };
            if structure_type != "_error" {
                return Err(format!("rich record {record_index} is not an error"));
            }
            let values = element_blocks(body, "v")
                .into_iter()
                .map(|(_, value)| unescape_xml(value))
                .collect::<Vec<_>>();
            if values.len() != keys.len() {
                return Err(format!("rich record {record_index} field count mismatch"));
            }
            let error_type_code = keys
                .iter()
                .position(|(name, _)| name == "errorType")
                .map(|index| values[index].parse::<u32>())
                .transpose()
                .map_err(|error| format!("rich record {record_index}: {error}"))?;
            let fallback_error = first_element(body, "fb")
                .map(|(opening, value)| {
                    if attribute(opening, "t") != Some("e") {
                        return Err(format!(
                            "rich record {record_index} fallback is not an error"
                        ));
                    }
                    Ok(unescape_xml(value))
                })
                .transpose()?;
            let resolved_error = match error_type_code {
                Some(4) => Some("#NAME?".to_owned()),
                Some(8) => Some("#SPILL!".to_owned()),
                _ => None,
            };
            Ok(RawRichError {
                record_index: record_index as u32,
                structure_index: structure_index as u32,
                error_type_code,
                resolved_error,
                fallback_error,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut result = BTreeMap::new();
    for (vm_offset, future_index) in vm_to_future.into_iter().enumerate() {
        let record_index = future_to_record[future_index] as usize;
        let record = records
            .get(record_index)
            .ok_or_else(|| format!("vm={} points outside rich values", vm_offset + 1))?
            .clone();
        result.insert(vm_offset as u32 + 1, record);
    }
    Ok(result)
}

fn opening_tags<'xml>(xml: &'xml str, name: &str) -> Vec<&'xml str> {
    let marker = format!("<{name}");
    let mut result = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = xml[cursor..].find(&marker) {
        let start = cursor + relative_start;
        let boundary = xml.as_bytes().get(start + marker.len());
        if !boundary
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'/' || *byte == b'>')
        {
            cursor = start + marker.len();
            continue;
        }
        let Some(relative_end) = xml[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        result.push(&xml[start..end]);
        cursor = end;
    }
    result
}

fn opening_tags_with_suffix<'xml>(xml: &'xml str, suffix: &str) -> Vec<&'xml str> {
    opening_tags(xml, suffix)
        .into_iter()
        .chain(
            xml.match_indices(&format!(":{suffix}"))
                .filter_map(|(suffix_at, _)| {
                    let start = xml[..suffix_at].rfind('<')?;
                    let end = suffix_at + xml[suffix_at..].find('>')? + 1;
                    Some(&xml[start..end])
                }),
        )
        .collect()
}

fn element_blocks<'xml>(xml: &'xml str, name: &str) -> Vec<(&'xml str, &'xml str)> {
    let mut result = Vec::new();
    let mut cursor = 0;
    let closing = format!("</{name}>");
    for opening in opening_tags(xml, name) {
        let Some(start) = xml[cursor..].find(opening).map(|offset| cursor + offset) else {
            continue;
        };
        let opening_end = start + opening.len();
        if opening.ends_with("/>") {
            result.push((opening, ""));
            cursor = opening_end;
            continue;
        }
        let Some(body_end) = xml[opening_end..]
            .find(&closing)
            .map(|offset| opening_end + offset)
        else {
            continue;
        };
        result.push((opening, &xml[opening_end..body_end]));
        cursor = body_end + closing.len();
    }
    result
}

fn first_element<'xml>(xml: &'xml str, name: &str) -> Option<(&'xml str, &'xml str)> {
    element_blocks(xml, name).into_iter().next()
}

fn attribute<'tag>(tag: &'tag str, name: &str) -> Option<&'tag str> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = start + tag[start..].find('"')?;
    Some(&tag[start..end])
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        MESSAGE_ARRAY_FORMULA_RESULT_REF, attribute, cached_cell_value, first_element,
        parse_formula_cells, unescape_xml,
    };

    #[test]
    fn xml_helpers_preserve_formula_text() {
        let cell = r#"<c r="F5" t="e" vm="3"><f>IF(A1&lt;2,&quot;x&quot;,0)</f><v>#VALUE!</v></c>"#;
        assert_eq!(attribute(cell, "r"), Some("F5"));
        assert_eq!(attribute(cell, "vm"), Some("3"));
        assert_eq!(
            first_element(cell, "f").map(|(_, value)| unescape_xml(value)),
            Some("IF(A1<2,\"x\",0)".to_owned())
        );
    }

    #[test]
    fn cached_strings_match_the_v2_producer_normalization() {
        let shared = vec!["zero".to_owned(), "shared value".to_owned()];
        assert_eq!(
            cached_cell_value(r#"<c r="A1" t="s">"#, "<v>1</v>", &shared, "Sheet1!A1",),
            Ok((Some("shared value".to_owned()), "str".to_owned()))
        );
        assert_eq!(
            cached_cell_value(
                r#"<c r="A2" t="inlineStr">"#,
                "<is><r><t>inline</t></r><r><t> value</t></r></is>",
                &shared,
                "Sheet1!A2",
            ),
            Ok((Some("inline value".to_owned()), "str".to_owned()))
        );
    }

    #[test]
    fn raw_array_formula_retains_its_declared_result_range() {
        let xml = r#"<worksheet><sheetData><row r="1"><c r="F1"><f t="array" ref="F1:G2">SEQUENCE(2,2)</f><v>1</v></c></row></sheetData></worksheet>"#;
        let mut formulas = BTreeMap::new();
        parse_formula_cells("Sheet1", xml, &BTreeMap::new(), &[], &mut formulas)
            .expect("valid array formula");
        assert_eq!(
            formulas
                .get("Sheet1!F1")
                .and_then(|cell| cell.array_result_ref.as_deref()),
            Some("F1:G2")
        );
    }

    #[test]
    fn raw_array_formula_requires_its_declared_result_range() {
        let xml = r#"<worksheet><sheetData><row r="1"><c r="F1"><f t="array">SEQUENCE(2,2)</f><v>1</v></c></row></sheetData></worksheet>"#;
        let error = parse_formula_cells("Sheet1", xml, &BTreeMap::new(), &[], &mut BTreeMap::new())
            .expect_err("array formula without ref");
        assert!(error.contains(MESSAGE_ARRAY_FORMULA_RESULT_REF), "{error}");
    }
}
