use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

use super::serialization::escape_text;
use super::sheet_deletion::{TabPlacement, patch_workbook_view};
use super::workbook_patch::{
    HostRecalculation, follows_calculation_properties, patch_calculation_properties_with_hints,
};
use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::calculation::sheet_changes::TabStep;
use crate::xlsx::package::PartPath;
use crate::xlsx::xml::{
    DOCUMENT_RELATIONSHIPS_STRICT, DOCUMENT_RELATIONSHIPS_TRANSITIONAL, namespace_reader,
    read_event,
};
use crate::{
    DateSystem, DefinedName, DefinedNameScope, Sheet, SheetId, SheetVisibility, WorkbookSnapshot,
};

const DETAIL_SHEET_COUNT: &str = "workbook sheet metadata does not match the source document";
const DETAIL_MISSING_WORKBOOK: &str = "workbook root was not found";
const DETAIL_MISSING_SHEETS: &str = "workbook sheets container was not found";
const DETAIL_RELATIONSHIP_ATTRIBUTE: &str = "workbook sheet relationship attribute was not found";
const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkbookPatchOptions<'a> {
    /// The recalculation request written to `calcPr`; `None` leaves `calcPr` as it is.
    pub(crate) host_recalculation: Option<HostRecalculation>,
    pub(crate) ensure_book_view: bool,
    /// The draft's sheet deletions and moves in the order it made them.
    pub(crate) tab_steps: &'a [TabStep],
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
    options: WorkbookPatchOptions<'_>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let WorkbookPatchOptions {
        host_recalculation,
        ensure_book_view,
        tab_steps,
    } = options;
    enforce_bytes(bytes.len(), limits, source)?;
    // Deleted and moved tabs move `workbookView` and every sheet-scoped name's `localSheetId`.
    let placement = TabPlacement::between(original, draft, tab_steps);
    let tabs_moved = placement.moved();
    let source_tabs = original
        .tab_order()
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    let names_changed = original.defined_names() != draft.defined_names()
        || draft
            .defined_names()
            .iter()
            .any(|name| moved_local_sheet_id(original, draft, name).is_some());
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
    let mut book_views_depth = None;
    let mut inserted_defined_names = !names_changed;
    let mut source_defined_names = None::<SourceDefinedNames>;
    let mut source_sheets = SourceSheets::default();
    loop {
        let event =
            read_event(&mut xml, &mut buffer).map_err(|error| invalid_xml(source, error))?;
        // A `<sheet>` element's content is captured with it.
        if source_sheets.capturing() {
            match &event {
                Event::Start(_) => depth = depth.saturating_add(1),
                Event::End(_) => depth = depth.saturating_sub(1),
                _ => {}
            }
            source_sheets.capture(event.into_owned(), depth);
            buffer.clear();
            continue;
        }
        let book_view = |element: &BytesStart<'_>, depth: u64| {
            tabs_moved
                && book_views_depth.is_some_and(|parent| depth == parent + 1)
                && element.local_name().as_ref() == "workbookView"
        };
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
                    book_views_depth = Some(depth);
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if book_view(&element, depth) {
                    let patched = patch_workbook_view(&element, &placement, source)?;
                    write_event(&mut writer, Event::Start(patched), source)?;
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
                {
                    let (id, patched) = source_sheet(
                        &element,
                        &xml,
                        SourceSheetContext {
                            original,
                            draft,
                            source_tabs: &source_tabs,
                            tab_position: &mut existing_tab_position,
                            sheet_index: &mut existing_sheet_index,
                            relationship: &mut relationship_attribute,
                        },
                        source,
                    )?;
                    source_sheets.push_sheet(id, patched.map(Event::Start), Some(depth));
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
                {
                    let (id, patched) = source_sheet(
                        &element,
                        &xml,
                        SourceSheetContext {
                            original,
                            draft,
                            source_tabs: &source_tabs,
                            tab_position: &mut existing_tab_position,
                            sheet_index: &mut existing_sheet_index,
                            relationship: &mut relationship_attribute,
                        },
                        source,
                    )?;
                    source_sheets.push_sheet(id, patched.map(Event::Empty), None);
                } else if book_view(&element, depth + 1) {
                    let patched = patch_workbook_view(&element, &placement, source)?;
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
                if book_views_depth == Some(depth) {
                    book_views_depth = None;
                }
                if sheets_depth == Some(depth) && element.local_name().as_ref() == "sheets" {
                    if existing_sheet_index != original.sheets().len() {
                        return Err(invalid_generated(source, DETAIL_SHEET_COUNT));
                    }
                    let relationship = relationship_attribute
                        .as_ref()
                        .ok_or_else(|| invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE))?;
                    let sheets_element = sheets_name.as_deref().unwrap_or("sheets");
                    std::mem::take(&mut source_sheets).write_in_order(
                        &mut writer,
                        draft,
                        source,
                        |writer, sheet| {
                            let relationship_id =
                                added_relationships.get(&sheet.id()).ok_or_else(|| {
                                    invalid_generated(source, DETAIL_RELATIONSHIP_ATTRIBUTE)
                                })?;
                            write_added_sheet(
                                writer,
                                sheets_element,
                                relationship,
                                sheet,
                                relationship_id,
                                source,
                            )
                        },
                    )?;
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
                } else if sheets_depth.is_some_and(|parent| depth == parent) {
                    source_sheets.push_between(other.into_owned());
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

/// The source `<sheets>` entries and the text between them, captured so the draft's tab order
/// decides where each entry is written.
#[derive(Debug, Default)]
struct SourceSheets {
    items: Vec<SheetsItem>,
    /// The depth of the `<sheet>` start tag whose content is being captured.
    capturing: Option<u64>,
}

#[derive(Debug)]
enum SheetsItem {
    Between(Event<'static>),
    /// One source tab's entry, with no events for a deleted worksheet.
    Sheet {
        id: SheetId,
        events: Vec<Event<'static>>,
    },
}

impl SourceSheets {
    const fn capturing(&self) -> bool {
        self.capturing.is_some()
    }

    /// Records a tab's start tag, or none for a deleted worksheet, whose content `depth` marks
    /// as following until its end tag.
    fn push_sheet(&mut self, id: SheetId, start: Option<Event<'static>>, depth: Option<u64>) {
        self.capturing = depth;
        self.items.push(SheetsItem::Sheet {
            id,
            events: start.into_iter().collect(),
        });
    }

    fn push_between(&mut self, event: Event<'static>) {
        self.items.push(SheetsItem::Between(event));
    }

    /// Captures one event inside a `<sheet>` element, `depth` being the depth after it.
    fn capture(&mut self, event: Event<'static>, depth: u64) {
        let closes = matches!(event, Event::End(_)) && self.capturing == Some(depth + 1);
        if let Some(SheetsItem::Sheet { events, .. }) = self.items.last_mut()
            && !events.is_empty()
        {
            events.push(event);
        }
        if closes {
            self.capturing = None;
        }
    }

    /// Writes every draft tab's entry in the draft's tab order: a kept source entry as captured,
    /// and a sheet the draft added or copied through `added`. The text before the first source
    /// entry and after the last one stays around the entries.
    fn write_in_order(
        self,
        writer: &mut Writer<Cursor<Vec<u8>>>,
        draft: &WorkbookSnapshot,
        source: &PartPath,
        mut added: impl FnMut(&mut Writer<Cursor<Vec<u8>>>, &Sheet) -> Result<(), XlsxWriteError>,
    ) -> Result<(), XlsxWriteError> {
        let first_entry = self
            .items
            .iter()
            .position(|item| matches!(item, SheetsItem::Sheet { .. }))
            .unwrap_or(self.items.len());
        let last_entry = self
            .items
            .iter()
            .rposition(|item| matches!(item, SheetsItem::Sheet { .. }))
            .map_or(self.items.len(), |index| index + 1);
        let mut entries = BTreeMap::new();
        let mut prefix = Vec::new();
        let mut suffix = Vec::new();
        for (index, item) in self.items.into_iter().enumerate() {
            match item {
                SheetsItem::Sheet { id, events } => {
                    if !events.is_empty() {
                        entries.insert(id, events);
                    }
                }
                SheetsItem::Between(event) if index < first_entry => prefix.push(event),
                SheetsItem::Between(event) if index >= last_entry => suffix.push(event),
                SheetsItem::Between(_) => {}
            }
        }
        for event in prefix {
            write_event(writer, event, source)?;
        }
        for (id, _) in draft.tab_order() {
            match entries.remove(&id) {
                Some(events) => {
                    for event in events {
                        write_event(writer, event, source)?;
                    }
                }
                None => {
                    let sheet = draft
                        .sheet_by_id(id)
                        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
                    added(writer, sheet)?;
                }
            }
        }
        for event in suffix {
            write_event(writer, event, source)?;
        }
        Ok(())
    }
}

/// What reading one source `<sheet>` element advances.
struct SourceSheetContext<'a> {
    original: &'a WorkbookSnapshot,
    draft: &'a WorkbookSnapshot,
    source_tabs: &'a [SheetId],
    tab_position: &'a mut usize,
    sheet_index: &'a mut usize,
    relationship: &'a mut Option<RelationshipAttribute>,
}

/// Reads the source `<sheet>` element at the next tab position and returns its tab's ID with the
/// start tag to write: a chartsheet, dialogsheet, or macrosheet as it is, a worksheet patched from
/// the draft, and `None` for a deleted worksheet. A worksheet's relationship attribute is read
/// either way, so a workbook whose source worksheets were all deleted still declares the sheets
/// it added.
fn source_sheet(
    element: &BytesStart<'_>,
    reader: &NsReader<&[u8]>,
    context: SourceSheetContext<'_>,
    source: &PartPath,
) -> Result<(SheetId, Option<BytesStart<'static>>), XlsxWriteError> {
    let position = *context.tab_position;
    *context.tab_position += 1;
    let id = *context
        .source_tabs
        .get(position)
        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
    if context
        .original
        .non_worksheet_tabs()
        .contains_position(position)
    {
        return Ok((id, Some(element.to_owned())));
    }
    let source_sheet = context
        .original
        .sheets()
        .get(*context.sheet_index)
        .ok_or_else(|| invalid_generated(source, DETAIL_SHEET_COUNT))?;
    *context.sheet_index += 1;
    let sheet = context.draft.sheet_by_id(source_sheet.id());
    let (patched, relationship) =
        patch_sheet(element, reader, sheet.unwrap_or(source_sheet), source)?;
    context.relationship.get_or_insert(relationship);
    Ok((source_sheet.id(), sheet.map(|_| patched)))
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

/// Returns the draft tab position of a retained sheet-scoped name's sheet when a deletion or move
/// changed it, so its `localSheetId` is renumbered.
fn moved_local_sheet_id(
    original: &WorkbookSnapshot,
    draft: &WorkbookSnapshot,
    name: &DefinedName,
) -> Option<usize> {
    let DefinedNameScope::Sheet(sheet_id) = name.scope() else {
        return None;
    };
    let position = draft.sheet_tab_position(sheet_id)?;
    original
        .sheet_tab_position(sheet_id)
        .is_some_and(|source| source != position)
        .then_some(position)
}

fn patch_local_sheet_id(
    element: &BytesStart<'_>,
    position: usize,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let mut patched = BytesStart::new(element.name().as_ref().to_owned());
    let position = position.to_string();
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.as_ref() == "localSheetId" {
            patched.push_attribute(("localSheetId", position.as_str()));
        } else {
            patched.push_attribute(attribute);
        }
    }
    Ok(patched.into_owned())
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
        let moved = moved_local_sheet_id(original, draft, defined_name);
        match retained {
            Some((source_name, element)) if source_name == defined_name => {
                let start = match moved {
                    Some(position) => patch_local_sheet_id(&element.start, position, source)?,
                    None => element.start.borrow(),
                };
                if element.content.is_empty() {
                    write_event(writer, Event::Empty(start), source)?;
                } else {
                    write_event(writer, Event::Start(start), source)?;
                    for event in &element.content {
                        write_event(writer, event.borrow(), source)?;
                    }
                }
            }
            Some((source_name, element)) => {
                let mut patched =
                    patch_defined_name(&element.start, source_name, defined_name, source)?;
                if let Some(position) = moved {
                    patched = patch_local_sheet_id(&patched, position, source)?;
                }
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
