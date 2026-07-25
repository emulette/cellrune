use std::collections::{BTreeMap, BTreeSet};

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::NsReader;

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::CellAddress;
use crate::xlsx::package::PartPath;

const DETAIL_RICH_TEXT_PHONETIC_EDIT: &str =
    "phonetic edits on source rich text require a rich-text authoring model";
const DETAIL_INVALID_SOURCE: &str =
    "source text storage could not be classified for phonetic preservation";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Debug)]
enum TextStorage {
    Shared(usize),
    Inline { rich: bool },
    Other,
}

#[derive(Debug)]
struct TargetCell {
    depth: u64,
    address: CellAddress,
    storage: TextStorage,
    value_depth: Option<u64>,
    shared_index: String,
    inline_depth: Option<u64>,
    phonetic_depth: Option<u64>,
}

pub(super) fn ensure_phonetic_edit_preservation(
    worksheet_bytes: &[u8],
    worksheet_part: &PartPath,
    targets: &BTreeSet<CellAddress>,
    shared_strings: Option<(&[u8], &PartPath)>,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    if targets.is_empty() {
        return Ok(());
    }
    enforce_bytes(worksheet_bytes.len(), worksheet_part, limits)?;
    let mut xml = configured_reader(worksheet_bytes);
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut current = None::<TargetCell>;
    let mut found = BTreeMap::<CellAddress, TextStorage>::new();
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_source(worksheet_part, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, worksheet_part, limits)?;
                if current.is_none() && element.local_name().as_ref() == b"c" {
                    let address = required_address(&element, worksheet_part)?;
                    if targets.contains(&address) {
                        current = Some(TargetCell {
                            depth,
                            address,
                            storage: storage_from_cell(&element, worksheet_part)?,
                            value_depth: None,
                            shared_index: String::new(),
                            inline_depth: None,
                            phonetic_depth: None,
                        });
                    }
                } else if let Some(cell) = &mut current {
                    match element.local_name().as_ref() {
                        b"v" if depth == cell.depth + 1 => cell.value_depth = Some(depth),
                        b"is" if depth == cell.depth + 1 => cell.inline_depth = Some(depth),
                        b"rPh" if cell.inline_depth.is_some() => cell.phonetic_depth = Some(depth),
                        b"r" if cell.inline_depth.is_some() && cell.phonetic_depth.is_none() => {
                            cell.storage = TextStorage::Inline { rich: true };
                        }
                        _ => {}
                    }
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), worksheet_part, limits)?;
                if current.is_none() && element.local_name().as_ref() == b"c" {
                    let address = required_address(&element, worksheet_part)?;
                    if targets.contains(&address) {
                        found.insert(address, TextStorage::Other);
                    }
                } else if let Some(cell) = &mut current
                    && element.local_name().as_ref() == b"r"
                    && cell.inline_depth.is_some()
                    && cell.phonetic_depth.is_none()
                {
                    cell.storage = TextStorage::Inline { rich: true };
                }
            }
            Event::Text(text)
                if current
                    .as_ref()
                    .is_some_and(|cell| cell.value_depth.is_some()) =>
            {
                let cell = current.as_mut().expect("guarded current cell");
                cell.shared_index.push_str(
                    &text
                        .xml_content(XmlVersion::Implicit1_0)
                        .map_err(|error| invalid_source(worksheet_part, error))?,
                );
            }
            Event::End(element) => {
                if let Some(cell) = &mut current {
                    if cell.value_depth == Some(depth) && element.local_name().as_ref() == b"v" {
                        cell.value_depth = None;
                    }
                    if cell.phonetic_depth == Some(depth) && element.local_name().as_ref() == b"rPh"
                    {
                        cell.phonetic_depth = None;
                    }
                    if cell.inline_depth == Some(depth) && element.local_name().as_ref() == b"is" {
                        cell.inline_depth = None;
                    }
                }
                if current.as_ref().is_some_and(|cell| {
                    cell.depth == depth && element.local_name().as_ref() == b"c"
                }) {
                    let cell = current.take().expect("guarded current cell");
                    let address = cell.address;
                    let storage = match cell.storage {
                        TextStorage::Shared(_) if !cell.shared_index.trim().is_empty() => {
                            let index = cell
                                .shared_index
                                .trim()
                                .parse::<usize>()
                                .map_err(|error| invalid_source(worksheet_part, error))?;
                            TextStorage::Shared(index)
                        }
                        TextStorage::Shared(_) => TextStorage::Other,
                        storage => storage,
                    };
                    found.insert(address, storage);
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    if found
        .values()
        .any(|storage| matches!(storage, TextStorage::Inline { rich: true }))
    {
        return Err(unsupported_rich_text());
    }
    let shared_targets = found
        .values()
        .filter_map(|storage| match storage {
            TextStorage::Shared(index) => Some(*index),
            TextStorage::Inline { .. } | TextStorage::Other => None,
        })
        .collect::<BTreeSet<_>>();
    if shared_targets.is_empty() {
        return Ok(());
    }
    let Some((bytes, part)) = shared_strings else {
        return Err(invalid_generated(worksheet_part));
    };
    if shared_items_are_rich(bytes, part, &shared_targets, limits)? {
        return Err(unsupported_rich_text());
    }
    Ok(())
}

fn shared_items_are_rich(
    bytes: &[u8],
    source: &PartPath,
    targets: &BTreeSet<usize>,
    limits: WriteLimits,
) -> Result<bool, XlsxWriteError> {
    enforce_bytes(bytes.len(), source, limits)?;
    let mut xml = configured_reader(bytes);
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut item_index = 0_usize;
    let mut selected_depth = None::<u64>;
    let mut phonetic_depth = None::<u64>;
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_source(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, source, limits)?;
                if element.local_name().as_ref() == b"si" {
                    if targets.contains(&item_index) {
                        selected_depth = Some(depth);
                    }
                } else if selected_depth.is_some() && element.local_name().as_ref() == b"rPh" {
                    phonetic_depth = Some(depth);
                } else if selected_depth.is_some()
                    && phonetic_depth.is_none()
                    && element.local_name().as_ref() == b"r"
                {
                    return Ok(true);
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), source, limits)?;
                if selected_depth.is_some()
                    && phonetic_depth.is_none()
                    && element.local_name().as_ref() == b"r"
                {
                    return Ok(true);
                }
                if element.local_name().as_ref() == b"si" {
                    item_index = item_index.saturating_add(1);
                }
            }
            Event::End(element) => {
                if phonetic_depth == Some(depth) && element.local_name().as_ref() == b"rPh" {
                    phonetic_depth = None;
                }
                if selected_depth == Some(depth) && element.local_name().as_ref() == b"si" {
                    selected_depth = None;
                    item_index = item_index.saturating_add(1);
                } else if element.local_name().as_ref() == b"si" {
                    item_index = item_index.saturating_add(1);
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(false)
}

fn storage_from_cell(
    element: &BytesStart<'_>,
    source: &PartPath,
) -> Result<TextStorage, XlsxWriteError> {
    let mut cell_type = None::<String>;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_source(source, error))?;
        if attribute.key.local_name().as_ref() == b"t" {
            cell_type = Some(
                std::str::from_utf8(attribute.value.as_ref())
                    .map_err(|error| invalid_source(source, error))?
                    .to_owned(),
            );
        }
    }
    Ok(match cell_type.as_deref() {
        Some("s") => TextStorage::Shared(0),
        Some("inlineStr") => TextStorage::Inline { rich: false },
        _ => TextStorage::Other,
    })
}

fn required_address(
    element: &BytesStart<'_>,
    source: &PartPath,
) -> Result<CellAddress, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_source(source, error))?;
        if attribute.key.local_name().as_ref() == b"r" {
            let value = std::str::from_utf8(attribute.value.as_ref())
                .map_err(|error| invalid_source(source, error))?;
            return CellAddress::from_a1(value).map_err(|error| invalid_source(source, error));
        }
    }
    Err(invalid_generated(source))
}

fn configured_reader(bytes: &[u8]) -> NsReader<&[u8]> {
    let mut reader = NsReader::from_reader(bytes);
    let config = reader.config_mut();
    config.check_end_names = true;
    config.allow_unmatched_ends = false;
    config.expand_empty_elements = false;
    config.trim_text(false);
    reader
}

fn enforce_depth(depth: u64, source: &PartPath, limits: WriteLimits) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .at_source(source.source_id())
                .with_detail(DETAIL_XML_DEPTH),
        );
    }
    Ok(())
}

fn enforce_bytes(
    size: usize,
    source: &PartPath,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    if size as u64 > limits.max_rewritten_xml_bytes() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .at_source(source.source_id())
                .with_detail(DETAIL_XML_BYTES),
        );
    }
    Ok(())
}

fn unsupported_rich_text() -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(DETAIL_RICH_TEXT_PHONETIC_EDIT)
}

fn invalid_generated(source: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_detail(DETAIL_INVALID_SOURCE)
}

fn invalid_source(
    source: &PartPath,
    error: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_detail(DETAIL_INVALID_SOURCE)
        .with_cause(error)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::ensure_phonetic_edit_preservation;
    use crate::xlsx::package::PartPath;
    use crate::xlsx::write::WriteLimits;
    use crate::{CellAddress, XlsxWriteError, XlsxWriteErrorCode};

    fn check(worksheet: &[u8], shared_strings: Option<&[u8]>) -> Result<(), XlsxWriteError> {
        let worksheet_part =
            PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("worksheet part");
        let shared_strings_part =
            PartPath::from_archive_name(b"xl/sharedStrings.xml").expect("shared strings part");
        let targets = BTreeSet::from([CellAddress::from_a1("A1").expect("cell address")]);
        ensure_phonetic_edit_preservation(
            worksheet,
            &worksheet_part,
            &targets,
            shared_strings.map(|bytes| (bytes, &shared_strings_part)),
            WriteLimits::default(),
        )
    }

    fn assert_rich_text_rejected(worksheet: &[u8], shared_strings: Option<&[u8]>) {
        let error = check(worksheet, shared_strings).expect_err("rich text must be rejected");
        assert_eq!(error.code(), XlsxWriteErrorCode::UnsupportedPreservation);
    }

    #[test]
    fn empty_shared_string_cells_do_not_charge_shared_item_zero() {
        let worksheet = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"/></row></sheetData></worksheet>"#;
        let shared_strings = br#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><t>rich</t></r></si></sst>"#;
        check(worksheet, Some(shared_strings))
            .expect("empty cell is not a shared-string reference");
    }

    #[test]
    fn inline_plain_text_and_phonetic_run_children_are_not_rich_text() {
        let plain = br#"<worksheet><sheetData><row><c r="A1" t="inlineStr"><is><t>base</t><rPh sb="0" eb="1"><t>guide</t><r/></rPh></is></c></row></sheetData></worksheet>"#;
        check(plain, None).expect("phonetic run content is not source rich text");

        let absent = br#"<worksheet><sheetData><row><c r="B1" t="inlineStr"><is><r><t>unrelated rich text</t></r></is></c></row></sheetData></worksheet>"#;
        check(absent, None).expect("an absent target does not need preservation classification");
    }

    #[test]
    fn inline_rich_text_start_and_empty_runs_are_rejected() {
        let start = br#"<worksheet><sheetData><row><c r="A1" t="inlineStr"><is><r><t>rich</t></r></is></c></row></sheetData></worksheet>"#;
        assert_rich_text_rejected(start, None);

        let empty = br#"<worksheet><sheetData><row><c r="A1" t="inlineStr"><is><r/></is></c></row></sheetData></worksheet>"#;
        assert_rich_text_rejected(empty, None);
    }

    #[test]
    fn shared_string_selection_uses_the_referenced_item_index() {
        let worksheet = br#"<worksheet><sheetData><row><c r="A1" t="s"><v>1</v></c></row></sheetData></worksheet>"#;
        let shared_strings =
            br#"<sst><si><r><t>unreferenced rich</t></r></si><si><t>plain</t></si></sst>"#;
        check(worksheet, Some(shared_strings)).expect("only the referenced item is classified");
    }

    #[test]
    fn shared_phonetic_runs_are_not_rich_text_but_base_rich_runs_are() {
        let worksheet = br#"<worksheet><sheetData><row><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#;
        let phonetic =
            br#"<sst><si><t>base</t><rPh sb="0" eb="1"><t>guide</t><r/></rPh></si></sst>"#;
        check(worksheet, Some(phonetic)).expect("rPh children do not make the base text rich");

        let rich = br#"<sst><si><r><t>rich</t></r></si></sst>"#;
        assert_rich_text_rejected(worksheet, Some(rich));

        let empty_rich = br#"<sst><si><r/></si></sst>"#;
        assert_rich_text_rejected(worksheet, Some(empty_rich));
    }

    #[test]
    fn nested_cell_content_lookalikes_do_not_change_storage_classification() {
        let inline = br#"<worksheet><sheetData><row><c r="A1" t="inlineStr"><wrapper><v>9</v><is><r/></is><r/><rPh><r/></rPh></wrapper><is><t>plain</t></is></c></row></sheetData></worksheet>"#;
        check(inline, None).expect("only direct cell content controls inline storage");

        let shared = br#"<worksheet><sheetData><row><c r="A1" t="s"><wrapper><v>9</v></wrapper><v>1</v></c></row></sheetData></worksheet>"#;
        let shared_strings =
            br#"<sst><si><t>plain</t></si><si><r><t>referenced rich</t></r></si></sst>"#;
        assert_rich_text_rejected(shared, Some(shared_strings));
    }

    #[test]
    fn nonempty_shared_cells_without_a_value_are_not_shared_string_zero() {
        let worksheet =
            br#"<worksheet><sheetData><row><c r="A1" t="s"></c></row></sheetData></worksheet>"#;
        check(worksheet, None).expect("a missing shared-string index is other storage");
    }

    #[test]
    fn shared_item_state_closes_at_phonetic_and_item_boundaries() {
        let worksheet = br#"<worksheet><sheetData><row><c r="A1" t="s"><v>0</v></c></row></sheetData></worksheet>"#;
        let rich_after_phonetic =
            br#"<sst><si><t>base</t><rPh sb="0" eb="1"><t>guide</t></rPh><r/></si></sst>"#;
        assert_rich_text_rejected(worksheet, Some(rich_after_phonetic));

        let second_item_worksheet = br#"<worksheet><sheetData><row><c r="A1" t="s"><v>1</v></c></row></sheetData></worksheet>"#;
        let second_item_rich =
            br#"<sst><si><t>first</t></si><si><r><t>second rich</t></r></si></sst>"#;
        assert_rich_text_rejected(second_item_worksheet, Some(second_item_rich));

        let after_empty_item = br#"<sst><si/><si><r/></si></sst>"#;
        assert_rich_text_rejected(second_item_worksheet, Some(after_empty_item));
    }
}
