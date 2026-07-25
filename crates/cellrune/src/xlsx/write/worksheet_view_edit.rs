use std::io::Cursor;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::reader::NsReader;

use super::{WriteLimits, XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::{CellAddress, FrozenPane};

const DETAIL_XML_DEPTH: &str = "max_xml_depth";
const DETAIL_XML_BYTES: &str = "max_rewritten_xml_bytes";
const DETAIL_WORKSHEET_ROOT: &str = "worksheet root was not found while patching frozen panes";

pub(crate) fn patch_frozen_pane(
    bytes: &[u8],
    source: &PartPath,
    pane: Option<FrozenPane>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    enforce_bytes(bytes.len(), limits, source)?;
    let mut xml = configured_reader(bytes);
    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buffer = Vec::new();
    let mut depth = 0_u64;
    let mut worksheet_depth = None;
    let mut worksheet_name = None::<Vec<u8>>;
    let mut sheet_views_depth = None;
    let mut sheet_views_name = None::<Vec<u8>>;
    let mut default_view_depth = None;
    let mut saw_sheet_views = false;
    let mut saw_default_view = false;
    let mut inserted_sheet_views = false;
    let mut skip_pane_depth = None;

    loop {
        let event = xml
            .read_event_into(&mut buffer)
            .map_err(|error| invalid_xml(source, error))?;
        match event {
            Event::Start(element) => {
                depth = depth.saturating_add(1);
                enforce_depth(depth, limits, source)?;
                if skip_pane_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if depth == 1 && element.local_name().as_ref() == b"worksheet" {
                    worksheet_depth = Some(depth);
                    worksheet_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if worksheet_depth.is_some_and(|root| depth == root + 1)
                    && element.local_name().as_ref() == b"sheetViews"
                {
                    saw_sheet_views = true;
                    sheet_views_depth = Some(depth);
                    sheet_views_name = Some(element.name().as_ref().to_vec());
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                } else if sheet_views_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"sheetView"
                    && workbook_view_id(&element, source)? == 0
                {
                    if std::mem::replace(&mut saw_default_view, true) {
                        return Err(invalid_generated(source, DETAIL_WORKSHEET_ROOT));
                    }
                    default_view_depth = Some(depth);
                    let default_view_name = element.name().as_ref().to_vec();
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                    if let Some(pane) = pane {
                        write_pane(&mut writer, &default_view_name, pane, source)?;
                    }
                } else if default_view_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"pane"
                {
                    skip_pane_depth = Some(depth);
                } else if default_view_depth.is_some_and(|parent| depth == parent + 1)
                    && element.local_name().as_ref() == b"selection"
                    && pane.is_none()
                {
                    let patched = without_pane_attribute(&element, source)?;
                    write_event(&mut writer, Event::Start(patched), source)?;
                } else {
                    maybe_insert_sheet_views(
                        &mut writer,
                        worksheet_depth,
                        depth,
                        element.local_name().as_ref(),
                        worksheet_name.as_deref(),
                        pane,
                        saw_sheet_views,
                        &mut inserted_sheet_views,
                        source,
                    )?;
                    write_event(&mut writer, Event::Start(element.into_owned()), source)?;
                }
            }
            Event::Empty(element) => {
                enforce_depth(depth.saturating_add(1), limits, source)?;
                if skip_pane_depth.is_some() {
                    buffer.clear();
                    continue;
                }
                if worksheet_depth.is_some_and(|root| depth + 1 == root + 1)
                    && element.local_name().as_ref() == b"sheetViews"
                {
                    saw_sheet_views = true;
                    if let Some(pane) = pane {
                        write_sheet_views(
                            &mut writer,
                            worksheet_name.as_deref().unwrap_or(b"worksheet"),
                            pane,
                            source,
                        )?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else if sheet_views_depth == Some(depth)
                    && element.local_name().as_ref() == b"sheetView"
                    && workbook_view_id(&element, source)? == 0
                {
                    if std::mem::replace(&mut saw_default_view, true) {
                        return Err(invalid_generated(source, DETAIL_WORKSHEET_ROOT));
                    }
                    if let Some(pane) = pane {
                        let name = element.name().as_ref().to_vec();
                        let start = element.into_owned();
                        write_event(&mut writer, Event::Start(start), source)?;
                        write_pane(&mut writer, &name, pane, source)?;
                        write_event(
                            &mut writer,
                            Event::End(BytesEnd::new(decode_name(&name, source)?)),
                            source,
                        )?;
                    } else {
                        write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                    }
                } else if default_view_depth == Some(depth)
                    && element.local_name().as_ref() == b"pane"
                {
                } else if default_view_depth == Some(depth)
                    && element.local_name().as_ref() == b"selection"
                    && pane.is_none()
                {
                    let patched = without_pane_attribute(&element, source)?;
                    write_event(&mut writer, Event::Empty(patched), source)?;
                } else {
                    maybe_insert_sheet_views(
                        &mut writer,
                        worksheet_depth,
                        depth + 1,
                        element.local_name().as_ref(),
                        worksheet_name.as_deref(),
                        pane,
                        saw_sheet_views,
                        &mut inserted_sheet_views,
                        source,
                    )?;
                    write_event(&mut writer, Event::Empty(element.into_owned()), source)?;
                }
            }
            Event::End(element) => {
                if let Some(skipped) = skip_pane_depth {
                    if depth == skipped {
                        skip_pane_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                    buffer.clear();
                    continue;
                }
                if default_view_depth == Some(depth)
                    && element.local_name().as_ref() == b"sheetView"
                {
                    default_view_depth = None;
                }
                if sheet_views_depth == Some(depth)
                    && element.local_name().as_ref() == b"sheetViews"
                {
                    if !saw_default_view && let Some(pane) = pane {
                        write_sheet_view(
                            &mut writer,
                            sheet_views_name.as_deref().unwrap_or(b"sheetViews"),
                            pane,
                            source,
                        )?;
                    }
                    sheet_views_depth = None;
                }
                write_event(&mut writer, Event::End(element.into_owned()), source)?;
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            other => {
                if skip_pane_depth.is_none() {
                    write_event(&mut writer, other.into_owned(), source)?;
                }
            }
        }
        buffer.clear();
    }
    if worksheet_depth.is_none() || (pane.is_some() && !saw_sheet_views && !inserted_sheet_views) {
        return Err(invalid_generated(source, DETAIL_WORKSHEET_ROOT));
    }
    let output = writer.into_inner().into_inner();
    enforce_bytes(output.len(), limits, source)?;
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn maybe_insert_sheet_views(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    worksheet_depth: Option<u64>,
    depth: u64,
    local_name: &[u8],
    worksheet_name: Option<&[u8]>,
    pane: Option<FrozenPane>,
    saw_sheet_views: bool,
    inserted: &mut bool,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    if *inserted || saw_sheet_views || pane.is_none() {
        return Ok(());
    }
    if worksheet_depth.is_some_and(|root| depth == root + 1)
        && !matches!(local_name, b"sheetPr" | b"dimension" | b"sheetViews")
    {
        write_sheet_views(
            writer,
            worksheet_name.unwrap_or(b"worksheet"),
            pane.expect("pane checked"),
            source,
        )?;
        *inserted = true;
    }
    Ok(())
}

fn write_sheet_views(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    worksheet_name: &[u8],
    pane: FrozenPane,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(worksheet_name, b"sheetViews");
    write_event(
        writer,
        Event::Start(BytesStart::new(decode_name(&name, source)?)),
        source,
    )?;
    write_sheet_view(writer, &name, pane, source)?;
    write_event(
        writer,
        Event::End(BytesEnd::new(decode_name(&name, source)?)),
        source,
    )
}

fn write_sheet_view(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sheet_views_name: &[u8],
    pane: FrozenPane,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(sheet_views_name, b"sheetView");
    let mut start = BytesStart::new(decode_name(&name, source)?);
    start.push_attribute(("workbookViewId", "0"));
    write_event(writer, Event::Start(start), source)?;
    write_pane(writer, &name, pane, source)?;
    write_event(
        writer,
        Event::End(BytesEnd::new(decode_name(&name, source)?)),
        source,
    )
}

fn write_pane(
    writer: &mut Writer<Cursor<Vec<u8>>>,
    sheet_view_name: &[u8],
    pane: FrozenPane,
    source: &PartPath,
) -> Result<(), XlsxWriteError> {
    let name = qualified_sibling_name(sheet_view_name, b"pane");
    let mut element = BytesStart::new(decode_name(&name, source)?);
    let columns = pane.frozen_columns().to_string();
    let rows = pane.frozen_rows().to_string();
    if pane.frozen_columns() > 0 {
        element.push_attribute(("xSplit", columns.as_str()));
    }
    if pane.frozen_rows() > 0 {
        element.push_attribute(("ySplit", rows.as_str()));
    }
    let top_left = CellAddress::from_indices(pane.frozen_rows() + 1, pane.frozen_columns() + 1)
        .expect("validated frozen pane has a valid top-left cell")
        .to_string();
    element.push_attribute(("topLeftCell", top_left.as_str()));
    element.push_attribute((
        "activePane",
        match (pane.frozen_rows() > 0, pane.frozen_columns() > 0) {
            (true, true) => "bottomRight",
            (true, false) => "bottomLeft",
            (false, true) => "topRight",
            (false, false) => unreachable!("clear panes are not retained"),
        },
    ));
    element.push_attribute(("state", "frozen"));
    write_event(writer, Event::Empty(element), source)
}

fn without_pane_attribute(
    element: &BytesStart<'_>,
    source: &PartPath,
) -> Result<BytesStart<'static>, XlsxWriteError> {
    let name = decode_name(element.name().as_ref(), source)?.to_owned();
    let mut output = BytesStart::new(name);
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.local_name().as_ref() != b"pane" {
            output.push_attribute(attribute);
        }
    }
    Ok(output.into_owned())
}

fn workbook_view_id(element: &BytesStart<'_>, source: &PartPath) -> Result<u32, XlsxWriteError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|error| invalid_xml(source, error))?;
        if attribute.key.local_name().as_ref() == b"workbookViewId" {
            return std::str::from_utf8(attribute.value.as_ref())
                .map_err(|error| invalid_xml(source, error))?
                .parse::<u32>()
                .map_err(|error| invalid_xml(source, error));
        }
    }
    Err(invalid_generated(source, DETAIL_WORKSHEET_ROOT))
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
    let mut output = qualified_prefix(template).as_bytes().to_vec();
    output.extend_from_slice(local_name);
    output
}

fn qualified_prefix(template: &[u8]) -> &str {
    template
        .iter()
        .rposition(|byte| *byte == b':')
        .and_then(|index| std::str::from_utf8(&template[..=index]).ok())
        .unwrap_or("")
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

#[cfg(test)]
mod tests {
    use super::patch_frozen_pane;
    use crate::xlsx::package::PartPath;
    use crate::{FrozenPane, WriteLimits, XlsxWriteErrorCode};

    fn source() -> PartPath {
        PartPath::from_archive_name(b"xl/worksheets/sheet1.xml").expect("part")
    }

    fn patch(input: &[u8], pane: Option<FrozenPane>) -> String {
        String::from_utf8(
            patch_frozen_pane(input, &source(), pane, WriteLimits::default()).expect("patch"),
        )
        .expect("UTF-8")
    }

    #[test]
    fn pane_patch_inserts_replaces_and_clears_the_default_view() {
        let input = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1"/><sheetData/></worksheet>"#;
        let pane = FrozenPane::new(1, 3).expect("pane");
        let inserted = patch(input, Some(pane));
        assert!(inserted.contains(
            r#"<pane xSplit="3" ySplit="1" topLeftCell="D2" activePane="bottomRight" state="frozen"/>"#
        ));
        assert!(
            inserted.find("<dimension").expect("dimension")
                < inserted.find("<sheetViews").expect("sheet views")
        );
        assert!(
            inserted.find("<sheetViews").expect("sheet views")
                < inserted.find("<sheetData").expect("sheet data")
        );

        let cleared = patch(inserted.as_bytes(), None);
        assert!(!cleared.contains("<pane"), "{cleared}");
    }

    #[test]
    fn pane_patch_changes_only_the_default_view_and_preserves_other_children() {
        let input = br#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetPr><x:tabColor rgb="FF00FF00"/></x:sheetPr><x:dimension ref="A1"/><x:sheetViews><x:sheetView workbookViewId="1"><x:pane xSplit="4" topLeftCell="E1" activePane="topRight" state="split"/><x:selection pane="topRight" activeCell="E1" marker="other"/></x:sheetView><x:sheetView workbookViewId="0" showGridLines="0"><x:pane xSplit="3" ySplit="3" topLeftCell="D4" activePane="bottomRight" state="frozen"><x:ext/></x:pane><x:selection pane="bottomRight" activeCell="C3"/><x:extLst><x:ext uri="preserved"/></x:extLst></x:sheetView></x:sheetViews><x:sheetData/></x:worksheet>"#;

        let replaced = patch(input, Some(FrozenPane::new(2, 0).expect("pane")));
        assert!(replaced.contains(
            r#"<x:pane ySplit="2" topLeftCell="A3" activePane="bottomLeft" state="frozen"/>"#
        ));
        assert!(!replaced.contains("topLeftCell=\"D4\""), "{replaced}");
        assert!(
            replaced.contains(
                r#"<x:pane xSplit="4" topLeftCell="E1" activePane="topRight" state="split"/>"#
            ),
            "{replaced}"
        );
        assert!(
            replaced.contains(r#"<x:selection pane="bottomRight" activeCell="C3"/>"#),
            "{replaced}"
        );
        assert!(replaced.contains(r#"<x:ext uri="preserved"/>"#));
        assert_eq!(replaced.matches("workbookViewId=\"0\"").count(), 1);

        let cleared = patch(input, None);
        assert!(!cleared.contains("state=\"frozen\""), "{cleared}");
        assert!(cleared.contains("state=\"split\""), "{cleared}");
        assert!(
            cleared.contains(r#"<x:selection activeCell="C3"/>"#),
            "{cleared}"
        );
        assert!(
            cleared.contains(r#"<x:selection pane="topRight" activeCell="E1" marker="other"/>"#),
            "{cleared}"
        );
        assert!(cleared.contains(r#"<x:ext uri="preserved"/>"#));
    }

    #[test]
    fn pane_patch_populates_empty_or_nondefault_sheet_views() {
        let pane = Some(FrozenPane::new(1, 1).expect("pane"));
        let empty_parent = br#"<worksheet><sheetViews/><sheetData/></worksheet>"#;
        let from_empty_parent = patch(empty_parent, pane);
        assert_eq!(from_empty_parent.matches("<sheetViews").count(), 1);
        assert!(from_empty_parent.contains(r#"<sheetView workbookViewId="0">"#));
        assert!(from_empty_parent.contains("state=\"frozen\""));

        let empty_default = br#"<worksheet><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let from_empty_default = patch(empty_default, pane);
        assert_eq!(
            from_empty_default.matches("workbookViewId=\"0\"").count(),
            1
        );
        assert!(from_empty_default.contains("state=\"frozen\""));

        let nondefault_only = br#"<worksheet><sheetViews><sheetView workbookViewId="1"/></sheetViews><sheetData/></worksheet>"#;
        let with_default = patch(nondefault_only, pane);
        assert!(with_default.contains(r#"<sheetView workbookViewId="1"/>"#));
        assert!(with_default.contains(r#"<sheetView workbookViewId="0">"#));
    }

    #[test]
    fn pane_clear_patches_nonempty_selections_without_dropping_children() {
        let input = br#"<worksheet><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft" activeCell="A2"><ext marker="preserved"/></selection></sheetView></sheetViews><sheetData/></worksheet>"#;
        let cleared = patch(input, None);
        assert!(!cleared.contains("state=\"frozen\""), "{cleared}");
        assert!(
            cleared.contains(r#"<selection activeCell="A2">"#),
            "{cleared}"
        );
        assert!(
            cleared.contains(r#"<ext marker="preserved"/>"#),
            "{cleared}"
        );
    }

    #[test]
    fn view_state_ends_at_the_matching_container_boundary() {
        let input = br#"<worksheet><sheetViews><sheetView workbookViewId="0"></sheetView><sheetView workbookViewId="1"><pane xSplit="4" topLeftCell="E1" activePane="topRight" state="split"/></sheetView></sheetViews><wrapper><sheetView workbookViewId="2"><pane xSplit="7" state="split"/></sheetView></wrapper><sheetData/></worksheet>"#;
        let replaced = patch(input, Some(FrozenPane::new(1, 0).expect("pane")));
        assert!(
            replaced.contains(
                r#"<sheetView workbookViewId="1"><pane xSplit="4" topLeftCell="E1" activePane="topRight" state="split"/></sheetView>"#
            ),
            "{replaced}"
        );
        assert!(
            replaced.contains(
                r#"<wrapper><sheetView workbookViewId="2"><pane xSplit="7" state="split"/></sheetView></wrapper>"#
            ),
            "{replaced}"
        );
    }

    #[test]
    fn duplicate_default_views_are_rejected() {
        let input = br#"<worksheet><sheetViews><sheetView workbookViewId="0"/><sheetView workbookViewId="0"/></sheetViews><sheetData/></worksheet>"#;
        let error = patch_frozen_pane(
            input,
            &source(),
            Some(FrozenPane::new(1, 0).expect("pane")),
            WriteLimits::default(),
        )
        .expect_err("duplicate default views are ambiguous");
        assert_eq!(error.code(), XlsxWriteErrorCode::InvalidGeneratedXml);
    }

    #[test]
    fn nonworksheet_and_incomplete_roots_are_rejected() {
        for input in [
            br#"<notWorksheet><sheetData/></notWorksheet>"#.as_slice(),
            br#"<notWorksheet><worksheet><sheetData/></worksheet></notWorksheet>"#.as_slice(),
            br#"<worksheet></worksheet>"#.as_slice(),
        ] {
            let error = patch_frozen_pane(
                input,
                &source(),
                Some(FrozenPane::new(1, 0).expect("pane")),
                WriteLimits::default(),
            )
            .expect_err("invalid worksheet root must fail closed");
            assert_eq!(error.code(), XlsxWriteErrorCode::InvalidGeneratedXml);
        }
    }
}
