use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

use super::serialization::escape_text;
use super::workbook_patch::{
    HostRecalculation, follows_calculation_properties, patch_calculation_properties_with_hints,
};
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, namespace_reader,
    read_event,
};
use crate::{
    DateSystem, DefinedName, DefinedNameScope, Sheet, SheetId, SheetVisibility, WorkbookSnapshot,
};

const DETAIL_SHEET_COUNT: &str = "workbook sheet metadata does not match the source document";
const DETAIL_SHEET_ORDER: &str = "draft changed the stable order of existing sheets";
const DETAIL_MISSING_WORKBOOK: &str = "workbook root was not found";
const DETAIL_MISSING_SHEETS: &str = "workbook sheets container was not found";
const DETAIL_RELATIONSHIP_ATTRIBUTE: &str = "workbook sheet relationship attribute was not found";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkbookPatchOptions {
    /// The recalculation request written to `calcPr`; `None` leaves `calcPr` as it is.
    pub(crate) host_recalculation: Option<HostRecalculation>,
    pub(crate) ensure_book_view: bool,
}

#[derive(Debug)]
struct RelationshipAttribute {
    qualified_name: String,
    prefix: String,
    namespace: String,
}

/// The source `<definedNames>` container, captured so a changed name set is written in draft
/// order while each retained name keeps its source element and producer attributes.
#[derive(Debug)]
struct SourceDefinedNames {
    depth: u64,
    container: BytesStart<'static>,
    elements: Vec<SourceDefinedName>,
    current: Option<SourceDefinedName>,
}

/// One source `<definedName>` element. `content` holds every event after the start tag through
/// the end tag and is empty for a self-closing element.
#[derive(Debug)]
struct SourceDefinedName {
    start: BytesStart<'static>,
    content: Vec<Event<'static>>,
}

impl SourceDefinedNames {
    fn new(container: BytesStart<'static>, depth: u64) -> Self {
        Self {
            depth,
            container,
            elements: Vec::new(),
            current: None,
        }
    }

    fn capture_start(&mut self, element: BytesStart<'_>, depth: u64, empty: bool) {
        if let Some(current) = &mut self.current {
            let element = element.into_owned();
            current.content.push(if empty {
                Event::Empty(element)
            } else {
                Event::Start(element)
            });
        } else if depth == self.depth + 1 && element.local_name().as_ref() == "definedName" {
            let defined_name = SourceDefinedName {
                start: element.into_owned(),
                content: Vec::new(),
            };
            if empty {
                self.elements.push(defined_name);
            } else {
                self.current = Some(defined_name);
            }
        }
    }

    fn capture_end(&mut self, element: BytesEnd<'_>, depth: u64) {
        if let Some(current) = &mut self.current {
            current.content.push(Event::End(element.into_owned()));
            if depth == self.depth + 1 {
                self.elements.extend(self.current.take());
            }
        }
    }

    fn capture(&mut self, event: Event<'_>) {
        if let Some(current) = &mut self.current {
            current.content.push(event.into_owned());
        }
    }
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
        host_recalculation,
        ensure_book_view,
    } = options;
    enforce_bytes(bytes.len(), limits, source)?;
    validate_existing_sheet_order(original, draft, source)?;
    let names_changed = original.defined_names() != draft.defined_names();
    let mut xml = configured_reader(bytes, limits);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut workbook_depth = None;
    let mut workbook_name = None::<String>;
    let mut sheets_depth = None;
    let mut sheets_name = None::<String>;
    let mut relationship_attribute = None::<RelationshipAttribute>;
    let mut existing_sheet_index = 0_usize;
    let mut existing_tab_position = 0_usize;
    let mut saw_workbook_properties = false;
    let mut saw_book_views = false;
    let mut inserted_defined_names = !names_changed;
    let mut source_defined_names = None::<SourceDefinedNames>;
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if let Some(names) = &mut source_defined_names {
                    names.capture_start(element, depth, false);
                    buffer.clear();
                    continue;
                }
                if depth == 1 && element.local_name().as_ref() == "workbook" {
                    workbook_depth = Some(depth);
                    workbook_name = Some(element.name().as_ref().to_owned());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == "workbookPr"
                {
                    saw_workbook_properties = true;
                    let patched = patch_date_system(&element, draft.date_system(), source)?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == "bookViews"
                {
                    saw_book_views = true;
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == "sheets"
                {
                    if !saw_workbook_properties && draft.date_system() == DateSystem::Excel1904 {
                        write_workbook_properties(
                            &mut writer,
                            workbook_name.as_deref().unwrap_or("workbook"),
                            draft.date_system(),
                            source,
                        )?;
                        saw_workbook_properties = true;
                    }
                    if ensure_book_view && !saw_book_views {
                        write_book_views(
                            &mut writer,
                            workbook_name.as_deref().unwrap_or("workbook"),
                            source,
                        )?;
                        saw_book_views = true;
                    }
                    sheets_depth = Some(depth);
                    sheets_name = Some(element.name().as_ref().to_owned());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if sheets_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == "sheet"
                    && is_non_worksheet_tab(original, &mut existing_tab_position)
                {
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if sheets_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == "sheet"
                {
                    let sheet = original
                        .sheets()
                        .get(existing_sheet_index)
                        .and_then(|source_sheet| draft.sheet_by_id(source_sheet.id()))
                        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
                    let (patched, relationship) = patch_sheet(&element, &xml, sheet, source)?;
                    relationship_attribute.get_or_insert(relationship);
                    existing_sheet_index = existing_sheet_index.saturating_add(1);
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == "definedNames"
                    && names_changed
                {
                    source_defined_names =
                        Some(SourceDefinedNames::new(element.into_owned(), depth));
                    inserted_defined_names = true;
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
                if let Some(names) = &mut source_defined_names {
                    names.capture_start(element, depth + 1, true);
                    buffer.clear();
                    continue;
                }
                if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == "workbookPr"
                {
                    saw_workbook_properties = true;
                    let patched = patch_date_system(&element, draft.date_system(), source)?;
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == "bookViews"
                {
                    saw_book_views = true;
                    if ensure_book_view {
                        let name = element.name().as_ref().to_owned();
                        write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                        let view_name = qualified_sibling_name(&name, "workbookView");
                        write_event(
                            &mut writer,
                            Event::Empty(BytesStart::new(view_name)),
                            source,
                        )?;
                        write_event(&mut writer, Event::End(BytesEnd::new(name)), source)?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == "sheets"
                {
                    return Err(invalid_generated(source, DETAIL_MISSING_SHEETS));
                } else if sheets_depth.is_some_and(|parent| depth + 1 == parent + 1)
                    && element.local_name().as_ref() == "sheet"
                    && is_non_worksheet_tab(original, &mut existing_tab_position)
                {
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                } else if sheets_depth.is_some_and(|parent| depth + 1 == parent + 1)
                    && element.local_name().as_ref() == "sheet"
                {
                    let sheet = original
                        .sheets()
                        .get(existing_sheet_index)
                        .and_then(|source_sheet| draft.sheet_by_id(source_sheet.id()))
                        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
                    let (patched, relationship) = patch_sheet(&element, &xml, sheet, source)?;
                    relationship_attribute.get_or_insert(relationship);
                    existing_sheet_index = existing_sheet_index.saturating_add(1);
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else if workbook_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == "definedNames"
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
                if let Some(mut names) = source_defined_names.take() {
                    if depth == names.depth {
                        write_source_defined_names(
                            &mut writer,
                            names,
                            element,
                            original,
                            draft,
                            source,
                        )?;
                    } else {
                        names.capture_end(element, depth);
                        source_defined_names = Some(names);
                    }
                    depth = depth.saturating_sub(1);
                    buffer.clear();
                    continue;
                }
                if sheets_depth == Some(depth) && element.local_name().as_ref() == "sheets" {
                    if existing_sheet_index != original.sheets().len() {
                        return Err(invalid_generated(source, DETAIL_SHEET_COUNT));
                    }
                    let relationship = relationship_attribute
                        .as_ref()
                        .ok_or_else(|| invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE))?;
                    for sheet in draft
                        .sheets()
                        .iter()
                        .filter(|sheet| original.sheet_by_id(sheet.id()).is_none())
                    {
                        let relationship_id =
                            added_relationships.get(&sheet.id()).ok_or_else(|| {
                                invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE)
                            })?;
                        write_added_sheet(
                            &mut writer,
                            sheets_name.as_deref().unwrap_or("sheets"),
                            relationship,
                            sheet,
                            relationship_id,
                            source,
                        )?;
                    }
                }
                if workbook_depth == Some(depth)
                    && element.local_name().as_ref() == "workbook"
                    && names_changed
                    && !inserted_defined_names
                {
                    write_defined_names(
                        &mut writer,
                        &qualified_sibling_name(
                            workbook_name.as_deref().unwrap_or("workbook"),
                            "definedNames",
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
                if let Some(names) = &mut source_defined_names {
                    names.capture(other);
                } else {
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
    match host_recalculation {
        Some(host_recalculation) => patch_calculation_properties_with_hints(
            &semantics,
            source,
            host_recalculation,
            Some(draft.calculation_hints()),
            limits,
        ),
        None => Ok(semantics),
    }
}

fn write_book_views(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    workbook_name: &str,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let views_name = qualified_sibling_name(workbook_name, "bookViews");
    write_event(
        writer,
        Event::Start(BytesStart::new(views_name.as_str())),
        source,
    )?;
    let view_name = qualified_sibling_name(&views_name, "workbookView");
    write_event(writer, Event::Empty(BytesStart::new(view_name)), source)?;
    write_event(writer, Event::End(BytesEnd::new(views_name)), source)
}

/// Advances past one source `<sheet>` element and reports whether it is a chartsheet,
/// dialogsheet, or macrosheet tab, which is copied through unchanged.
fn is_non_worksheet_tab(original: &WorkbookSnapshot, tab_position: &mut usize) -> bool {
    let position = *tab_position;
    *tab_position += 1;
    original.non_worksheet_tabs().contains_position(position)
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
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() != "date1904" {
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
    workbook_name: &str,
    date_system: DateSystem,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(workbook_name, "workbookPr");
    let mut properties = BytesStart::new(name);
    if date_system == DateSystem::Excel1904 {
        properties.push_attribute(("date1904", "1"));
    }
    write_event(writer, Event::Empty(properties), source)
}

fn patch_sheet(
    element: &BytesStart<'_>,
    reader: &NsReader<&[u8]>,
    sheet: &Sheet,
    source: &PartPath,
) -> Result<(BytesStart<'static>, RelationshipAttribute), XlsxWriteError> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    let mut relationship = None;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        let (namespace, local_name) = reader.resolver().resolve_attribute(attribute.key);
        let namespace = match namespace {
            ResolveResult::Bound(namespace)
                if matches!(
                    namespace.as_ref(),
                    DOCUMENT_RELATIONSHIPS_TRANSITIONAL | DOCUMENT_RELATIONSHIPS_STRICT
                ) =>
            {
                Some(namespace.as_ref().to_owned())
            }
            _ => None,
        };
        if local_name.as_ref() == "id"
            && let Some(namespace) = namespace
        {
            if relationship.is_none() {
                let qualified_name = attribute.key.as_ref().to_owned();
                let prefix_end = qualified_name
                    .find(':')
                    .ok_or_else(|| invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE))?;
                relationship = Some(RelationshipAttribute {
                    prefix: qualified_name[..prefix_end].to_owned(),
                    qualified_name,
                    namespace,
                });
            }
            patched.push_attribute(attribute);
        } else if !matches!(attribute.key.as_ref(), "name" | "sheetId" | "state") {
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
        relationship.ok_or_else(|| invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE))?,
    ))
}

fn write_added_sheet(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sheets_name: &str,
    relationship: &RelationshipAttribute,
    sheet: &Sheet,
    relationship_id: &str,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(sheets_name, "sheet");
    let mut element = BytesStart::new(name);
    let sheet_id = sheet.id().get().to_string();
    element.push_attribute(("name", sheet.name().as_str()));
    element.push_attribute(("sheetId", sheet_id.as_str()));
    match sheet.visibility() {
        SheetVisibility::Visible => {}
        SheetVisibility::Hidden => element.push_attribute(("state", "hidden")),
        SheetVisibility::VeryHidden => element.push_attribute(("state", "veryHidden")),
    }
    let namespace_declaration = format!("xmlns:{}", relationship.prefix);
    element.push_attribute((
        namespace_declaration.as_str(),
        relationship.namespace.as_str(),
    ));
    element.push_attribute((relationship.qualified_name.as_str(), relationship_id));
    write_event(writer, Event::Empty(element), source)
}

#[allow(clippy::too_many_arguments)]
fn maybe_insert_defined_names(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    workbook_depth: Option<u64>,
    depth: u64,
    local_name: &str,
    workbook_name: Option<&str>,
    draft: &WorkbookSnapshot,
    names_changed: bool,
    inserted: &mut bool,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if names_changed
        && !*inserted
        && workbook_depth.is_some_and(|root| depth == root + 1)
        && (local_name == "calcPr" || follows_calculation_properties(local_name))
    {
        let name = qualified_sibling_name(workbook_name.unwrap_or("workbook"), "definedNames");
        write_defined_names(writer, &name, draft, source)?;
        *inserted = true;
    }
    Ok(())
}

fn write_defined_names(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    container_name: &str,
    workbook: &WorkbookSnapshot,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if workbook.defined_names().is_empty() {
        return Ok(());
    }
    write_event(
        writer,
        Event::Start(BytesStart::new(container_name)),
        source,
    )?;
    let name = qualified_sibling_name(container_name, "definedName");
    for defined_name in workbook.defined_names() {
        let element = new_defined_name(&name, workbook, defined_name, source)?;
        write_defined_name(writer, element, defined_name, source)?;
    }
    write_event(writer, Event::End(BytesEnd::new(container_name)), source)
}

/// Writes the draft names into the source container in draft order. A name whose identity
/// matches a source element keeps that element: verbatim when unchanged, otherwise with only its
/// changed name spelling, `hidden` flag, and formula text rewritten.
fn write_source_defined_names(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    names: SourceDefinedNames,
    end: BytesEnd<'_>,
    original: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if draft.defined_names().is_empty() {
        return Ok(());
    }
    let name = qualified_sibling_name(names.container.name().as_ref(), "definedName");
    write_event(writer, Event::Start(names.container), source)?;
    for defined_name in draft.defined_names() {
        let retained = original
            .defined_name(defined_name.scope(), defined_name.name())
            .and_then(|(index, source_name)| Some((source_name, names.elements.get(index)?)));
        match retained {
            Some((source_name, element)) if source_name == defined_name => {
                if element.content.is_empty() {
                    write_event(writer, Event::Empty(element.start.borrow()), source)?;
                } else {
                    write_event(writer, Event::Start(element.start.borrow()), source)?;
                    for event in &element.content {
                        write_event(writer, event.borrow(), source)?;
                    }
                }
            }
            Some((source_name, element)) => {
                let patched =
                    patch_defined_name(&element.start, source_name, defined_name, source)?;
                write_defined_name(writer, patched, defined_name, source)?;
            }
            None => {
                let element = new_defined_name(&name, draft, defined_name, source)?;
                write_defined_name(writer, element, defined_name, source)?;
            }
        }
    }
    write_event(writer, Event::End(end), source)
}

fn new_defined_name<'a>(
    element_name: &'a str,
    workbook: &WorkbookSnapshot,
    defined_name: &DefinedName,
    source: &PartPath,
) -> Result<BytesStart<'a>, XlsxWriteError> {
    let mut element = BytesStart::new(element_name);
    element.push_attribute(("name", defined_name.name()));
    if let DefinedNameScope::Sheet(sheet_id) = defined_name.scope() {
        let index = workbook
            .sheet_tab_position(sheet_id)
            .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
        let index = index.to_string();
        element.push_attribute(("localSheetId", index.as_str()));
    }
    if defined_name.hidden() {
        element.push_attribute(("hidden", "1"));
    }
    Ok(element)
}

fn patch_defined_name(
    element: &BytesStart<'_>,
    source_name: &DefinedName,
    defined_name: &DefinedName,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        match attribute.key.as_ref() {
            "name" if source_name.name() != defined_name.name() => {
                patched.push_attribute(("name", defined_name.name()));
            }
            "hidden" if source_name.hidden() != defined_name.hidden() => {}
            _ => patched.push_attribute(attribute),
        }
    }
    if defined_name.hidden() && !source_name.hidden() {
        patched.push_attribute(("hidden", "1"));
    }
    Ok(patched.into_owned())
}

fn write_defined_name(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    element: BytesStart<'_>,
    defined_name: &DefinedName,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let end = element.to_end().into_owned();
    write_event(writer, Event::Start(element), source)?;
    write_event(
        writer,
        Event::Text(BytesText::from_escaped(escape_text(
            defined_name.formula().as_str(),
        )?)),
        source,
    )?;
    write_event(writer, Event::End(end), source)
}

fn configured_reader(bytes: &[u8], limits: WriteLimits) -> NsReader<&[u8]> {
    let mut reader = namespace_reader(bytes, limits.max_xml_depth());
    reader.config_mut().check_end_names = true;
    reader.config_mut().allow_unmatched_ends = false;
    reader.config_mut().expand_empty_elements = false;
    reader.config_mut().trim_text(false);
    reader
}

fn qualified_sibling_name(template: &str, local_name: &str) -> String {
    let prefix = template.rfind(':').map_or("", |index| &template[..=index]);
    format!("{prefix}{local_name}")
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
