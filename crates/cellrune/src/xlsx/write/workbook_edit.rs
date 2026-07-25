use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::NsReader;

use super::serialization::escape_text;
use super::workbook_patch::patch_calculation_properties_with_hints;
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::{DateSystem, DefinedNameScope, Sheet, SheetId, SheetVisibility, WorkbookSnapshot};

const DETAIL_SHEET_COUNT: &str = "workbook sheet metadata does not match the source document";
const DETAIL_SHEET_ORDER: &str = "draft changed the stable order of existing sheets";
const DETAIL_MISSING_WORKBOOK: &str = "workbook root was not found";
const DETAIL_MISSING_SHEETS: &str = "workbook sheets container was not found";
const DETAIL_RELATIONSHIP_ATTRIBUTE: &str = "workbook sheet relationship attribute was not found";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkbookPatchOptions {
    pub(crate) request_host_recalculation: bool,
    pub(crate) ensure_book_view: bool,
}

pub(crate) fn patch_workbook_semantics(
    bytes: &[u8],
    source: &PartPath,
    original: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
    added_relationships: &BTreeMap<SheetId, String>,
    options: WorkbookPatchOptions,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let WorkbookPatchOptions {
        request_host_recalculation,
        ensure_book_view,
    } = options;
    enforce_bytes(bytes.len(), limits, source)?;
    validate_existing_sheet_order(original, draft, source)?;
    let names_changed = original.defined_names() != draft.defined_names();
    let mut xml = configured_reader(bytes);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut workbook_depth = None;
    let mut workbook_name = None::<Vec<u8>>;
    let mut sheets_depth = None;
    let mut sheets_name = None::<Vec<u8>>;
    let mut relationship_attribute_name = None::<Vec<u8>>;
    let mut existing_sheet_index = 0_usize;
    let mut saw_workbook_properties = false;
    let mut saw_book_views = false;
    let mut inserted_defined_names = !names_changed;
    let mut skip_defined_names_depth = None::<u64>;
    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if skip_defined_names_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if depth == 1 && element.local_name().as_ref() == b"workbook" {
                    workbook_depth = Some(depth);
                    workbook_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"workbookPr"
                {
                    saw_workbook_properties = true;
                    let patched = patch_date_system(&element, draft.date_system(), source)?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"bookViews"
                {
                    saw_book_views = true;
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"sheets"
                {
                    if !saw_workbook_properties && draft.date_system() == DateSystem::Excel1904 {
                        write_workbook_properties(
                            &mut writer,
                            workbook_name.as_deref().unwrap_or(b"workbook"),
                            draft.date_system(),
                            source,
                        )?;
                        saw_workbook_properties = true;
                    }
                    if ensure_book_view && !saw_book_views {
                        write_book_views(
                            &mut writer,
                            workbook_name.as_deref().unwrap_or(b"workbook"),
                            source,
                        )?;
                        saw_book_views = true;
                    }
                    sheets_depth = Some(depth);
                    sheets_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if sheets_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"sheet"
                {
                    let sheet = original
                        .sheets()
                        .get(existing_sheet_index)
                        .and_then(|source_sheet| draft.sheet_by_id(source_sheet.id()))
                        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
                    let (patched, relationship_name) = patch_sheet(&element, sheet, source)?;
                    relationship_attribute_name.get_or_insert(relationship_name);
                    existing_sheet_index = existing_sheet_index.saturating_add(1);
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"definedNames"
                    && names_changed
                {
                    write_defined_names(&mut writer, element.name().as_ref(), draft, source)?;
                    inserted_defined_names = true;
                    skip_defined_names_depth = Some(depth);
                } else {
                    maybe_insert_defined_names(
                        &mut writer,
                        workbook_depth,
                        depth,
                        element.local_name().as_ref(),
                        workbook_name.as_deref(),
                        draft,
                        names_changed,
                        &mut inserted_defined_names,
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if skip_defined_names_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"workbookPr"
                {
                    saw_workbook_properties = true;
                    let patched = patch_date_system(&element, draft.date_system(), source)?;
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"bookViews"
                {
                    saw_book_views = true;
                    if ensure_book_view {
                        let name = element.name().as_ref().to_vec();
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                        let view_name = qualified_sibling_name(&name, b"workbookView");
                        write_event(
                            &mut writer,
                            Event::Empty(BytesStart::new(decode_name(&view_name, source)?)),
                            source,
                        )?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(decode_name(&name, source)?)),
                            source,
                        )?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"sheets"
                {
                    return Err(invalid_generated(source, DETAIL_MISSING_SHEETS));
                } else if sheets_depth.is_some_and(|parent| depth + 1 == parent + 1)
                    && element.local_name().as_ref() == b"sheet"
                {
                    let sheet = original
                        .sheets()
                        .get(existing_sheet_index)
                        .and_then(|source_sheet| draft.sheet_by_id(source_sheet.id()))
                        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
                    let (patched, relationship_name) = patch_sheet(&element, sheet, source)?;
                    relationship_attribute_name.get_or_insert(relationship_name);
                    existing_sheet_index = existing_sheet_index.saturating_add(1);
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"definedNames"
                    && names_changed
                {
                    write_defined_names(&mut writer, element.name().as_ref(), draft, source)?;
                    inserted_defined_names = true;
                } else {
                    maybe_insert_defined_names(
                        &mut writer,
                        workbook_depth,
                        depth + 1,
                        element.local_name().as_ref(),
                        workbook_name.as_deref(),
                        draft,
                        names_changed,
                        &mut inserted_defined_names,
                        source,
                    )?;
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                }
            }
            Event::End(element) => {
                if let Some(skipped) = skip_defined_names_depth {
                    if depth == skipped {
                        skip_defined_names_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                    buffer.clear();
                    continue;
                }
                if sheets_depth == Some(depth) && element.local_name().as_ref() == b"sheets" {
                    if existing_sheet_index != original.sheets().len() {
                        return Err(invalid_generated(source, DETAIL_SHEET_COUNT));
                    }
                    let relationship_name = relationship_attribute_name
                        .as_deref()
                        .ok_or_else(|| invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE))?;
                    for sheet in draft
                        .sheets()
                        .iter()
                        .filter(|sheet| original.sheet_by_id(sheet.id()).is_none())
                    {
                        let relationship =
                            added_relationships.get(&sheet.id()).ok_or_else(|| {
                                invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE)
                            })?;
                        write_added_sheet(
                            &mut writer,
                            sheets_name.as_deref().unwrap_or(b"sheets"),
                            relationship_name,
                            sheet,
                            relationship,
                            source,
                        )?;
                    }
                }
                if workbook_depth == Some(depth)
                    && element.local_name().as_ref() == b"workbook"
                    && names_changed
                    && !inserted_defined_names
                {
                    write_defined_names(
                        &mut writer,
                        &qualified_sibling_name(
                            workbook_name.as_deref().unwrap_or(b"workbook"),
                            b"definedNames",
                        ),
                        draft,
                        source,
                    )?;
                    inserted_defined_names = true;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => {
                if skip_defined_names_depth.is_none() {
                    write_event(&mut writer, other.into_owned(), source)?;
                }
            }
        }
        buffer.clear();
    }
    if workbook_depth.is_none() || sheets_depth.is_none() || !inserted_defined_names {
        return Err(invalid_generated(source, DETAIL_MISSING_WORKBOOK));
    }
    let semantics = writer.into_inner().into_inner();
    enforce_bytes(semantics.len(), limits, source)?;
    patch_calculation_properties_with_hints(
        &semantics,
        source,
        request_host_recalculation,
        Some(draft.calculation_hints()),
        limits,
    )
}

fn write_book_views(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    workbook_name: &[u8],
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let views_name = qualified_sibling_name(workbook_name, b"bookViews");
    write_event(
        writer,
        Event::Start(BytesStart::new(decode_name(&views_name, source)?)),
        source,
    )?;
    let view_name = qualified_sibling_name(&views_name, b"workbookView");
    write_event(
        writer,
        Event::Empty(BytesStart::new(decode_name(&view_name, source)?)),
        source,
    )?;
    write_event(
        writer,
        Event::End(BytesEnd::new(decode_name(&views_name, source)?)),
        source,
    )
}

fn validate_existing_sheet_order(
    original: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let existing = draft
        .sheets()
        .iter()
        .filter(|sheet| original.sheet_by_id(sheet.id()).is_some())
        .map(Sheet::id)
        .collect::<Vec<_>>();
    let original_ids = original.sheets().iter().map(Sheet::id).collect::<Vec<_>>();
    if existing == original_ids {
        Ok(())
    } else {
        Err(invalid_generated(source, DETAIL_SHEET_ORDER))
    }
}

fn patch_date_system(
    element: &BytesStart<'_>,
    date_system: DateSystem,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != b"date1904" {
            patched.push_attribute(attribute);
        }
    }
    if date_system == DateSystem::Excel1904 {
        patched.push_attribute(("date1904", "1"));
    }
    Ok(patched.into_owned())
}

fn write_workbook_properties(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    workbook_name: &[u8],
    date_system: DateSystem,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(workbook_name, b"workbookPr");
    let mut properties = BytesStart::new(decode_name(&name, source)?);
    if date_system == DateSystem::Excel1904 {
        properties.push_attribute(("date1904", "1"));
    }
    write_event(writer, Event::Empty(properties), source)
}

fn patch_sheet(
    element: &BytesStart<'_>,
    sheet: &Sheet,
    source: &PartPath,
) -> Result<(BytesStart<'static>, Vec<u8>), XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut patched = BytesStart::new(name);
    let mut relationship_name = None;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.local_name().as_ref() == b"id" && attribute.key.as_ref() != b"sheetId" {
            relationship_name = Some(attribute.key.as_ref().to_vec());
            patched.push_attribute(attribute);
        } else if !matches!(attribute.key.as_ref(), b"name" | b"sheetId" | b"state") {
            patched.push_attribute(attribute);
        }
    }
    let sheet_id = sheet.id().get().to_string();
    patched.push_attribute(("name", sheet.name().as_str()));
    patched.push_attribute(("sheetId", sheet_id.as_str()));
    match sheet.visibility() {
        SheetVisibility::Visible => {}
        SheetVisibility::Hidden => patched.push_attribute(("state", "hidden")),
        SheetVisibility::VeryHidden => patched.push_attribute(("state", "veryHidden")),
    }
    Ok((
        patched.into_owned(),
        relationship_name
            .ok_or_else(|| invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE))?,
    ))
}

fn write_added_sheet(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sheets_name: &[u8],
    relationship_name: &[u8],
    sheet: &Sheet,
    relationship_id: &str,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(sheets_name, b"sheet");
    let mut element = BytesStart::new(decode_name(&name, source)?);
    let sheet_id = sheet.id().get().to_string();
    element.push_attribute(("name", sheet.name().as_str()));
    element.push_attribute(("sheetId", sheet_id.as_str()));
    match sheet.visibility() {
        SheetVisibility::Visible => {}
        SheetVisibility::Hidden => element.push_attribute(("state", "hidden")),
        SheetVisibility::VeryHidden => element.push_attribute(("state", "veryHidden")),
    }
    element.push_attribute((decode_name(relationship_name, source)?, relationship_id));
    write_event(writer, Event::Empty(element), source)
}

#[allow(clippy::too_many_arguments)]
fn maybe_insert_defined_names(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    workbook_depth: Option<u64>,
    depth: u64,
    local_name: &[u8],
    workbook_name: Option<&[u8]>,
    draft: &WorkbookSnapshot,
    names_changed: bool,
    inserted: &mut bool,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if names_changed
        && !*inserted
        && workbook_depth.is_some_and(|root| depth == root + 1)
        && matches!(
            local_name,
            b"calcPr"
                | b"oleSize"
                | b"customWorkbookViews"
                | b"pivotCaches"
                | b"smartTagPr"
                | b"smartTagTypes"
                | b"webPublishing"
                | b"fileRecoveryPr"
                | b"webPublishObjects"
                | b"extLst"
        )
    {
        let name = qualified_sibling_name(workbook_name.unwrap_or(b"workbook"), b"definedNames");
        write_defined_names(writer, &name, draft, source)?;
        *inserted = true;
    }
    Ok(())
}

fn write_defined_names(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    container_name: &[u8],
    workbook: &WorkbookSnapshot,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if workbook.defined_names().is_empty() {
        return Ok(());
    }
    write_event(
        writer,
        Event::Start(BytesStart::new(decode_name(container_name, source)?)),
        source,
    )?;
    let name = qualified_sibling_name(container_name, b"definedName");
    for defined_name in workbook.defined_names() {
        let mut element = BytesStart::new(decode_name(&name, source)?);
        element.push_attribute(("name", defined_name.name()));
        if let DefinedNameScope::Sheet(sheet_id) = defined_name.scope() {
            let index = workbook
                .sheets()
                .iter()
                .position(|sheet| sheet.id() == sheet_id)
                .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
            let index = index.to_string();
            element.push_attribute(("localSheetId", index.as_str()));
        }
        if defined_name.hidden() {
            element.push_attribute(("hidden", "1"));
        }
        write_event(writer, Event::Start(element), source)?;
        write_event(
            writer,
            Event::Text(BytesText::from_escaped(escape_text(
                defined_name.formula().as_str(),
            )?)),
            source,
        )?;
        write_event(
            writer,
            Event::End(BytesEnd::new(decode_name(&name, source)?)),
            source,
        )?;
    }
    write_event(
        writer,
        Event::End(BytesEnd::new(decode_name(container_name, source)?)),
        source,
    )
}

fn configured_reader(bytes: &[u8]) -> NsReader<&[u8]> {
    let mut reader = NsReader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
}

fn qualified_sibling_name(template: &[u8], local_name: &[u8]) -> Vec<u8> {
    let mut output = template
        .iter()
        .rposition(|byte| *byte == b':')
        .map_or_else(Vec::new, |index| template[..=index].to_vec());
    output.extend_from_slice(local_name);
    output
}

fn enforce_depth(depth: u64, limits: WriteLimits, source: &PartPath) -> Result<(), XlsxWriteError> {
    if depth > limits.max_xml_depth() {
        return Err(resource_error(
            source,
            DETAIL_XML_DEPTH,
            depth,
            limits.max_xml_depth(),
        ));
    }
    Ok(())
}

fn enforce_bytes(
    actual: usize,
    limits: WriteLimits,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if actual as u64 > limits.max_rewritten_xml_bytes() {
        return Err(resource_error(
            source,
            DETAIL_XML_BYTES,
            actual as u64,
            limits.max_rewritten_xml_bytes(),
        ));
    }
    Ok(())
}

fn write_event<'a>(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    event: Event<'a>,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    writer
        .write_event(event)
        .map_err(|error| invalid_xml(source, error))
}

fn decode_name<'a>(name: &'a [u8], source: &PartPath) -> Result<&'a str, XlsxWriteError> {
    std::str::from_utf8(name).map_err(|error| invalid_xml(source, error))
}

fn invalid_xml(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .at_source(source.source_id())
        .with_cause(cause)
}

fn invalid_generated(source: &PartPath, detail: &'static str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
        .with_detail(detail)
        .at_source(source.source_id())
}

fn resource_error(
    source: &PartPath,
    name: &'static str,
    actual: u64,
    maximum: u64,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!("{name}: {actual} > {maximum}"))
        .at_source(source.source_id())
}
