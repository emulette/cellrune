//! The content of a source worksheet that only its package carries.

use std::collections::{BTreeMap, BTreeSet};

use quick_xml::events::Event;

use super::{XlsxWriteError, XlsxWriteErrorCode};
use crate::xlsx::package::PartPath;
use crate::xlsx::package::scan_internal_relationships;
use crate::xlsx::xml::{namespace_reader, read_event};
use crate::{SheetId, XlsxDocument};

/// Worksheet elements the workbook model carries, checks itself, or leaves as print and view
/// settings: margins, page setup, headers and footers, page breaks, sheet properties, default
/// row and column sizes, and ignored errors.
const MODELED_ELEMENTS: &[&str] = &[
    "dimension",
    "sheetViews",
    "cols",
    "sheetData",
    "autoFilter",
    "mergeCells",
    "phoneticPr",
    "dataValidations",
    "hyperlinks",
    "tableParts",
    "sheetPr",
    "sheetFormatPr",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "ignoredErrors",
];

/// Worksheet relationships to tables and printer settings, which the model replaces or a copy
/// leaves behind, and to hyperlinks, which the model checks itself.
const MODELED_RELATIONSHIPS: &[&str] = &["table", "printerSettings", "hyperlink"];

impl XlsxDocument {
    /// Returns the elements and relationship types of the worksheet `sheet_id` that only its
    /// package carries, such as `conditionalFormatting`, `drawing`, or `pivotTable`, in name
    /// order. A sheet without a source part carries none.
    pub(crate) fn package_only_sheet_content(
        &self,
        sheet_id: SheetId,
    ) -> Result<Vec<String>, XlsxWriteError> {
        let Some(part) = self.worksheet_part_path(sheet_id) else {
            return Ok(Vec::new());
        };
        let limits = self.read_options().limits();
        let mut reader = self.preserved_package().reader()?;
        let bytes = reader.read_part(part)?;
        let mut content = BTreeSet::new();
        let mut xml = namespace_reader(&bytes, limits.max_xml_depth());
        let mut buffer = Vec::new();
        let mut depth = 0_u32;
        loop {
            let event =
                read_event(&mut xml, &mut buffer).map_err(|error| invalid_source(part, error))?;
            // The worksheet's children are at depth 1.
            let (element, opens) = match &event {
                Event::Start(element) => (Some(element), true),
                Event::Empty(element) => (Some(element), false),
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                    (None, false)
                }
                Event::Eof => break,
                _ => (None, false),
            };
            if let Some(element) = element
                && depth == 1
                && !MODELED_ELEMENTS.contains(&element.local_name().as_ref())
            {
                content.insert(element.local_name().as_ref().to_owned());
            }
            if opens {
                depth += 1;
            }
            buffer.clear();
        }
        let relationships = part
            .relationship_part()
            .map_err(|error| invalid_source(part, error))?;
        if reader.contains(&relationships) {
            let bytes = reader.read_part(&relationships)?;
            for relationship in
                scan_internal_relationships(&bytes, &relationships, Some(part), limits)
                    .map_err(|error| invalid_source(&relationships, error))?
            {
                let kind = relationship.kind.rsplit('/').next().unwrap_or_default();
                if !MODELED_RELATIONSHIPS.contains(&kind) {
                    content.insert(kind.to_owned());
                }
            }
        }
        Ok(content.into_iter().collect())
    }
}

impl XlsxDocument {
    /// Returns the custom number formats the document's styles part declares, by ID.
    pub(crate) fn custom_number_formats(&self) -> Result<BTreeMap<u32, String>, XlsxWriteError> {
        let Some(styles) = self.package_summary().styles_part() else {
            return Ok(BTreeMap::new());
        };
        let part = PartPath::from_archive_name(styles.as_str().as_bytes())
            .map_err(|error| invalid_source(self.workbook_part_path(), error))?;
        let bytes = self.preserved_package().reader()?.read_part(&part)?;
        let mut xml = namespace_reader(&bytes, self.read_options().limits().max_xml_depth());
        let mut buffer = Vec::new();
        let mut formats = BTreeMap::new();
        loop {
            match read_event(&mut xml, &mut buffer).map_err(|error| invalid_source(&part, error))? {
                Event::Start(element) | Event::Empty(element)
                    if element.local_name().as_ref() == "numFmt" =>
                {
                    let mut id = None;
                    let mut code = None;
                    for attribute in element.attributes().flatten() {
                        match attribute.key.as_ref() {
                            "numFmtId" => id = attribute.value.parse::<u32>().ok(),
                            "formatCode" => {
                                code = attribute
                                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .ok()
                                    .map(|code| code.into_owned());
                            }
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(code)) = (id, code) {
                        formats.insert(id, code);
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(formats)
    }
}

fn invalid_source(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
        .at_source(source.source_id())
        .with_cause(cause)
}
