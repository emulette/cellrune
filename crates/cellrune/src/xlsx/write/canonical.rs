use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use super::materialization::{MaterializationAction, MaterializationPlan};
use super::serialization::{
    escape_attribute, escape_text, serialize_cell, serialize_materialized_follower,
    validate_phonetic_limits,
};
use super::{
    RecalculatedWorkbook, RecalculationWriteOptions, WriteLimits, WriteProvenance, WriteReport,
    XlsxWriteError, XlsxWriteErrorCode,
};
use crate::{
    CalculationSnapshot, CellAddress, CellContent, DateSystem, DefinedNameScope,
    MaterializedResultOrigin, NumberFormat, Sheet, SheetVisibility, WorkbookDraft,
    WorkbookSnapshot, XlsxDocumentKind,
};

const CONTENT_TYPES_PART: &str = "[Content_Types].xml";
const ROOT_RELS_PART: &str = "_rels/.rels";
const WORKBOOK_PART: &str = "xl/workbook.xml";
const WORKBOOK_RELS_PART: &str = "xl/_rels/workbook.xml.rels";
const STYLES_PART: &str = "xl/styles.xml";
const METADATA_PART: &str = "xl/metadata.xml";
const DETAIL_CALCULATION_REVISION: &str = "calculation does not belong to the current draft";
const DETAIL_STYLE_CONFLICT: &str =
    "one custom number-format ID is associated with multiple format codes";
const DETAIL_OUTPUT_BYTES: &str = "max_output_archive_bytes";
const DETAIL_ENTRY_BYTES: &str = "max_entry_uncompressed_bytes";
const DETAIL_TOTAL_BYTES: &str = "max_total_uncompressed_bytes";
const DETAIL_ENTRY_COUNT: &str = "max_entries";
const DETAIL_VERIFICATION_BYTES: &str = "max_verification_read_bytes";
const DETAIL_OUTPUT_VERIFICATION: &str =
    "reopened canonical workbook does not match the draft semantic model";

pub(crate) fn write_canonical_draft(
    draft: &WorkbookDraft,
    calculation: &CalculationSnapshot,
    options: RecalculationWriteOptions,
) -> Result<RecalculatedWorkbook, XlsxWriteError> {
    if !calculation.matches_workbook(draft.workbook()) {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::StaleSemanticRevision)
                .with_detail(DETAIL_CALCULATION_REVISION),
        );
    }
    let limits = options.write_options().limits();
    validate_phonetic_limits(draft.presentation(), limits)?;
    let materialization = MaterializationPlan::new(calculation, options.policy(), limits)?;
    let styles = StyleRegistry::for_workbook(draft.workbook())?;
    let request_host_recalculation =
        !materialization.is_complete() || !draft.workbook().diagnostics().is_empty();

    let mut parts = BTreeMap::<String, Vec<u8>>::new();
    parts.insert(
        CONTENT_TYPES_PART.to_owned(),
        content_types_xml(draft.workbook())?.into_bytes(),
    );
    parts.insert(
        ROOT_RELS_PART.to_owned(),
        root_relationships_xml().into_bytes(),
    );
    parts.insert(
        WORKBOOK_PART.to_owned(),
        workbook_xml(
            draft.workbook(),
            draft.presentation(),
            request_host_recalculation,
        )?
        .into_bytes(),
    );
    parts.insert(
        WORKBOOK_RELS_PART.to_owned(),
        workbook_relationships_xml(draft.workbook()).into_bytes(),
    );
    parts.insert(STYLES_PART.to_owned(), styles.to_xml()?.into_bytes());
    if workbook_has_dynamic_arrays(draft.workbook()) {
        parts.insert(
            METADATA_PART.to_owned(),
            dynamic_metadata_xml().into_bytes(),
        );
    }
    for (index, sheet) in draft.workbook().sheets().iter().enumerate() {
        let sheet_materialization = materialization
            .cells()
            .iter()
            .filter(|(id, _)| id.sheet_id() == sheet.id())
            .map(|(id, planned)| (id.address(), planned))
            .collect::<BTreeMap<_, _>>();
        let style_indexes = sheet
            .cells()
            .map(|cell| (cell.address(), styles.index(cell.number_format())))
            .collect::<BTreeMap<_, _>>();
        parts.insert(
            worksheet_part_name(index),
            generated_worksheet_xml(
                sheet,
                &style_indexes,
                &sheet_materialization,
                draft.presentation(),
            )?
            .into_bytes(),
        );
        if let Some(relationships) = super::table::worksheet_relationships_xml(sheet) {
            parts.insert(
                super::table::worksheet_relationships_part_name(index),
                relationships.into_bytes(),
            );
        }
        for table in sheet.tables() {
            parts.insert(
                super::table::table_part_name(table),
                super::table::table_xml(table)?.into_bytes(),
            );
        }
    }
    validate_generated_parts(&parts, limits)?;
    let bytes = write_archive(&parts, limits)?;
    verify_draft_output(
        draft.workbook(),
        draft.presentation(),
        &bytes,
        &materialization,
        crate::XlsxDocumentKind::Xlsx,
        limits,
    )?;

    let changed_parts = parts
        .keys()
        .map(|name| crate::SourceId::new(name.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_cause(error)
        })?;
    let report = WriteReport::new(
        options.policy(),
        materialization.materialized_count(),
        materialization.invalidated_cells().to_vec(),
        super::report::VerifiedOutputReceipt::new(changed_parts, Vec::new(), Vec::new(), &bytes),
        WriteProvenance::new(
            None,
            draft.semantic_revision(),
            draft.presentation_revision(),
            calculation.provenance().provider().clone(),
            calculation.options(),
        ),
    );
    Ok(RecalculatedWorkbook::new(
        bytes,
        report,
        XlsxDocumentKind::Xlsx,
    ))
}

pub(super) struct StyleRegistry {
    formats: Vec<NumberFormat>,
    custom_formats: BTreeMap<u32, Box<str>>,
}

impl StyleRegistry {
    fn for_workbook(workbook: &WorkbookSnapshot) -> Result<Self, XlsxWriteError> {
        Self::for_formats(
            workbook
                .sheets()
                .iter()
                .flat_map(|sheet| sheet.cells())
                .map(|cell| cell.number_format()),
        )
    }

    pub(super) fn for_formats<'a>(
        formats_to_register: impl IntoIterator<Item = &'a NumberFormat>,
    ) -> Result<Self, XlsxWriteError> {
        let mut formats = vec![NumberFormat::default()];
        let mut custom_formats = BTreeMap::<u32, Box<str>>::new();
        for format in formats_to_register {
            if let Some(code) = format.code()
                && format.id() >= 164
            {
                if custom_formats
                    .get(&format.id())
                    .is_some_and(|existing| existing.as_ref() != code)
                {
                    return Err(
                        XlsxWriteError::new(XlsxWriteErrorCode::ConflictingPartOperation)
                            .with_detail(DETAIL_STYLE_CONFLICT),
                    );
                }
                custom_formats.insert(format.id(), Box::from(code));
            }
            if !formats.iter().any(|existing| existing == format) {
                formats.push(format.clone());
            }
        }
        Ok(Self {
            formats,
            custom_formats,
        })
    }

    pub(super) fn index(&self, format: &NumberFormat) -> usize {
        self.formats
            .iter()
            .position(|candidate| candidate == format)
            .unwrap_or(0)
    }

    pub(super) fn to_xml(&self) -> Result<String, XlsxWriteError> {
        let mut xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
        );
        if !self.custom_formats.is_empty() {
            xml.push_str("<numFmts count=\"");
            xml.push_str(&self.custom_formats.len().to_string());
            xml.push_str("\">");
            for (id, code) in &self.custom_formats {
                xml.push_str("<numFmt numFmtId=\"");
                xml.push_str(&id.to_string());
                xml.push_str("\" formatCode=\"");
                xml.push_str(&escape_attribute(code)?);
                xml.push_str("\"/>");
            }
            xml.push_str("</numFmts>");
        }
        xml.push_str(
            r#"<fonts count="1"><font><sz val="11"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#,
        );
        xml.push_str("<cellXfs count=\"");
        xml.push_str(&self.formats.len().to_string());
        xml.push_str("\">");
        for format in &self.formats {
            xml.push_str("<xf numFmtId=\"");
            xml.push_str(&format.id().to_string());
            xml.push_str("\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"");
            if format.id() != 0 {
                xml.push_str(" applyNumberFormat=\"1\"");
            }
            xml.push_str("/>");
        }
        xml.push_str(
            r#"</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="0"/><tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/></styleSheet>"#,
        );
        Ok(xml)
    }
}

pub(super) fn generated_worksheet_xml(
    sheet: &Sheet,
    style_indexes: &BTreeMap<CellAddress, usize>,
    materialization: &BTreeMap<CellAddress, &super::materialization::PlannedMaterialization>,
    presentation: &crate::DocumentPresentation,
) -> Result<String, XlsxWriteError> {
    super::table::validate_table_headers(sheet)?;
    let mut cells = BTreeMap::<CellAddress, String>::new();
    for cell in sheet.cells() {
        let calculation = materialization
            .get(&cell.address())
            .map(|planned| &planned.action);
        cells.insert(
            cell.address(),
            serialize_cell(
                cell,
                style_indexes.get(&cell.address()).copied().unwrap_or(0),
                calculation,
                presentation.cell_presentation(sheet.id(), cell.address()),
            )?,
        );
    }
    for (address, planned) in materialization {
        if cells.contains_key(address) {
            continue;
        }
        if matches!(
            planned.origin,
            MaterializedResultOrigin::LegacyArray { .. }
                | MaterializedResultOrigin::DynamicSpill { .. }
        ) && let Some(cell) = serialize_materialized_follower(*address, &planned.action)?
        {
            cells.insert(*address, cell);
        }
    }
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main""#,
    );
    if !sheet.tables().is_empty() {
        xml.push_str(
            r#" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#,
        );
    }
    xml.push('>');
    if !cells.is_empty() {
        let row_start = cells
            .keys()
            .map(|address| address.row().get())
            .min()
            .expect("non-empty cells have a minimum row");
        let row_end = cells
            .keys()
            .map(|address| address.row().get())
            .max()
            .expect("non-empty cells have a maximum row");
        let column_start = cells
            .keys()
            .map(|address| address.column().get())
            .min()
            .expect("non-empty cells have a minimum column");
        let column_end = cells
            .keys()
            .map(|address| address.column().get())
            .max()
            .expect("non-empty cells have a maximum column");
        let first = CellAddress::from_indices(row_start, column_start)
            .expect("cell bounds produce a valid first address");
        let last = CellAddress::from_indices(row_end, column_end)
            .expect("cell bounds produce a valid last address");
        xml.push_str("<dimension ref=\"");
        if first == last {
            xml.push_str(&first.to_string());
        } else {
            xml.push_str(&first.to_string());
            xml.push(':');
            xml.push_str(&last.to_string());
        }
        xml.push_str("\"/>");
    }
    push_sheet_views(&mut xml, presentation.frozen_pane(sheet.id()));
    xml.push_str("<sheetData>");
    let rows = cells
        .keys()
        .map(|address| address.row().get())
        .collect::<BTreeSet<_>>();
    for row in rows {
        xml.push_str("<row r=\"");
        xml.push_str(&row.to_string());
        xml.push_str("\">");
        for (address, cell) in cells
            .iter()
            .filter(|(address, _)| address.row().get() == row)
        {
            let _ = address;
            xml.push_str(cell);
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData>");
    super::table::push_table_parts(&mut xml, sheet);
    xml.push_str("</worksheet>");
    Ok(xml)
}

fn workbook_xml(
    workbook: &WorkbookSnapshot,
    presentation: &crate::DocumentPresentation,
    request_host_recalculation: bool,
) -> Result<String, XlsxWriteError> {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
    );
    if workbook.date_system() == DateSystem::Excel1904 {
        xml.push_str("<workbookPr date1904=\"1\"/>");
    }
    if workbook
        .sheets()
        .iter()
        .any(|sheet| presentation.frozen_pane(sheet.id()).is_some())
    {
        xml.push_str("<bookViews><workbookView/></bookViews>");
    }
    xml.push_str("<sheets>");
    for (index, sheet) in workbook.sheets().iter().enumerate() {
        xml.push_str("<sheet name=\"");
        xml.push_str(&escape_attribute(sheet.name().as_str())?);
        xml.push_str("\" sheetId=\"");
        xml.push_str(&sheet.id().get().to_string());
        xml.push('"');
        match sheet.visibility() {
            SheetVisibility::Visible => {}
            SheetVisibility::Hidden => xml.push_str(" state=\"hidden\""),
            SheetVisibility::VeryHidden => xml.push_str(" state=\"veryHidden\""),
        }
        xml.push_str(" r:id=\"rId");
        xml.push_str(&(index + 1).to_string());
        xml.push_str("\"/>");
    }
    xml.push_str("</sheets>");
    if !workbook.defined_names().is_empty() {
        xml.push_str("<definedNames>");
        for name in workbook.defined_names() {
            xml.push_str("<definedName name=\"");
            xml.push_str(&escape_attribute(name.name())?);
            xml.push('"');
            if let DefinedNameScope::Sheet(sheet_id) = name.scope() {
                let local_index = workbook
                    .sheets()
                    .iter()
                    .position(|sheet| sheet.id() == sheet_id)
                    .ok_or_else(|| {
                        XlsxWriteError::new(XlsxWriteErrorCode::InvalidGeneratedXml)
                            .with_detail(DETAIL_OUTPUT_VERIFICATION)
                    })?;
                xml.push_str(" localSheetId=\"");
                xml.push_str(&local_index.to_string());
                xml.push('"');
            }
            if name.hidden() {
                xml.push_str(" hidden=\"1\"");
            }
            xml.push('>');
            xml.push_str(&escape_text(name.formula().as_str())?);
            xml.push_str("</definedName>");
        }
        xml.push_str("</definedNames>");
    }
    let hints = workbook.calculation_hints();
    if hints.mode().is_some()
        || hints.calculation_id().is_some()
        || hints.full_calculation_on_load().is_some()
        || hints.force_full_calculation().is_some()
        || hints.iterative_calculation().is_some()
        || request_host_recalculation
    {
        xml.push_str("<calcPr");
        if let Some(mode) = hints.mode() {
            xml.push_str(" calcMode=\"");
            xml.push_str(match mode {
                crate::CalculationMode::Automatic => "auto",
                crate::CalculationMode::AutomaticExceptDataTables => "autoNoTable",
                crate::CalculationMode::Manual => "manual",
            });
            xml.push('"');
        }
        if let Some(id) = hints.calculation_id() {
            xml.push_str(" calcId=\"");
            xml.push_str(&id.to_string());
            xml.push('"');
        }
        if let Some(iterative) = hints.iterative_calculation() {
            xml.push_str(if iterative {
                " iterate=\"1\""
            } else {
                " iterate=\"0\""
            });
        }
        let full = request_host_recalculation || hints.full_calculation_on_load().unwrap_or(false);
        let force = request_host_recalculation || hints.force_full_calculation().unwrap_or(false);
        xml.push_str(if full {
            " fullCalcOnLoad=\"1\""
        } else {
            " fullCalcOnLoad=\"0\""
        });
        xml.push_str(if force {
            " forceFullCalc=\"1\""
        } else {
            " forceFullCalc=\"0\""
        });
        xml.push_str("/>");
    }
    xml.push_str("</workbook>");
    Ok(xml)
}

fn push_sheet_views(output: &mut String, pane: Option<crate::FrozenPane>) {
    let Some(pane) = pane else {
        return;
    };
    output.push_str("<sheetViews><sheetView workbookViewId=\"0\"><pane");
    if pane.frozen_columns() > 0 {
        output.push_str(" xSplit=\"");
        output.push_str(&pane.frozen_columns().to_string());
        output.push('"');
    }
    if pane.frozen_rows() > 0 {
        output.push_str(" ySplit=\"");
        output.push_str(&pane.frozen_rows().to_string());
        output.push('"');
    }
    output.push_str(" topLeftCell=\"");
    let top_left = CellAddress::from_indices(pane.frozen_rows() + 1, pane.frozen_columns() + 1)
        .expect("validated frozen pane has a valid top-left cell");
    output.push_str(&top_left.to_string());
    output.push_str("\" activePane=\"");
    output.push_str(match (pane.frozen_rows() > 0, pane.frozen_columns() > 0) {
        (true, true) => "bottomRight",
        (true, false) => "bottomLeft",
        (false, true) => "topRight",
        (false, false) => unreachable!("clear panes are not retained"),
    });
    output.push_str("\" state=\"frozen\"/></sheetView></sheetViews>");
}

fn workbook_relationships_xml(workbook: &WorkbookSnapshot) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for index in 0..workbook.sheets().len() {
        xml.push_str("<Relationship Id=\"rId");
        xml.push_str(&(index + 1).to_string());
        xml.push_str("\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet");
        xml.push_str(&(index + 1).to_string());
        xml.push_str(".xml\"/>");
    }
    xml.push_str("<Relationship Id=\"rId");
    xml.push_str(&(workbook.sheets().len() + 1).to_string());
    xml.push_str("\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>");
    if workbook_has_dynamic_arrays(workbook) {
        xml.push_str("<Relationship Id=\"rId");
        xml.push_str(&(workbook.sheets().len() + 2).to_string());
        xml.push_str("\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata\" Target=\"metadata.xml\"/>");
    }
    xml.push_str("</Relationships>");
    xml
}

fn root_relationships_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_owned()
}

fn content_types_xml(workbook: &WorkbookSnapshot) -> Result<String, XlsxWriteError> {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    for index in 0..workbook.sheets().len() {
        xml.push_str("<Override PartName=\"/xl/worksheets/sheet");
        xml.push_str(&(index + 1).to_string());
        xml.push_str(".xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>");
    }
    for sheet in workbook.sheets() {
        for table in sheet.tables() {
            xml.push_str("<Override PartName=\"/xl/tables/table");
            xml.push_str(&table.id().get().to_string());
            xml.push_str(".xml\" ContentType=\"");
            xml.push_str(super::table::TABLE_CONTENT_TYPE);
            xml.push_str("\"/>");
        }
    }
    if workbook_has_dynamic_arrays(workbook) {
        xml.push_str("<Override PartName=\"/xl/metadata.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml\"/>");
    }
    xml.push_str("</Types>");
    Ok(xml)
}

fn workbook_has_dynamic_arrays(workbook: &WorkbookSnapshot) -> bool {
    workbook.sheets().iter().any(|sheet| {
        sheet.cells().any(|cell| {
            matches!(
                cell.content(),
                CellContent::Formula(formula)
                    if matches!(formula.metadata(), crate::FormulaMetadata::DynamicArray { .. })
            )
        })
    })
}

fn dynamic_metadata_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><metadata xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:xda="http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray"><metadataTypes count="1"><metadataType name="XLDAPR"/></metadataTypes><futureMetadata name="XLDAPR" count="1"><bk><extLst><ext uri="{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}"><xda:dynamicArrayProperties fDynamic="1" fCollapsed="0"/></ext></extLst></bk></futureMetadata><cellMetadata count="1"><bk><rc t="1" v="0"/></bk></cellMetadata></metadata>"#.to_owned()
}

fn worksheet_part_name(index: usize) -> String {
    format!("xl/worksheets/sheet{}.xml", index + 1)
}

fn validate_generated_parts(
    parts: &BTreeMap<String, Vec<u8>>,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    enforce_limit(DETAIL_ENTRY_COUNT, parts.len() as u64, limits.max_entries())?;
    let mut total = 0_u128;
    for bytes in parts.values() {
        enforce_limit(
            DETAIL_ENTRY_BYTES,
            bytes.len() as u64,
            limits.max_entry_uncompressed_bytes(),
        )?;
        total = total.saturating_add(bytes.len() as u128);
    }
    if total > u128::from(limits.max_total_uncompressed_bytes()) {
        return Err(resource_error(
            DETAIL_TOTAL_BYTES,
            total,
            u128::from(limits.max_total_uncompressed_bytes()),
        ));
    }
    Ok(())
}

fn write_archive(
    parts: &BTreeMap<String, Vec<u8>>,
    limits: WriteLimits,
) -> Result<Vec<u8>, XlsxWriteError> {
    let output = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(output);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, bytes) in parts {
        writer.start_file(name, options).map_err(zip_error)?;
        writer.write_all(bytes).map_err(io_error)?;
    }
    let bytes = writer.finish().map_err(zip_error)?.into_inner();
    enforce_limit(
        DETAIL_OUTPUT_BYTES,
        bytes.len() as u64,
        limits.max_output_archive_bytes(),
    )?;
    enforce_limit(
        "max_temporary_storage_bytes",
        bytes.len() as u64,
        limits.max_temporary_storage_bytes(),
    )?;
    Ok(bytes)
}

pub(super) fn verify_draft_output(
    expected: &WorkbookSnapshot,
    expected_presentation: &crate::DocumentPresentation,
    bytes: &[u8],
    materialization: &MaterializationPlan,
    expected_kind: crate::XlsxDocumentKind,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
    enforce_limit(
        DETAIL_VERIFICATION_BYTES,
        bytes.len() as u64,
        limits.max_verification_read_bytes(),
    )?;
    let reopened =
        crate::open_xlsx_document_bytes(bytes, crate::OpenOptions::default()).map_err(|error| {
            XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed).with_cause(error)
        })?;
    if reopened.kind() != expected_kind
        || !draft_semantics_match(expected, reopened.workbook(), materialization)
        || !expected_presentation.semantics_match(reopened.presentation())
    {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed)
                .with_detail(DETAIL_OUTPUT_VERIFICATION),
        );
    }
    for (id, planned) in materialization.cells() {
        let cell = reopened
            .workbook()
            .sheet_by_id(id.sheet_id())
            .and_then(|sheet| sheet.cell(id.address()));
        match (&planned.origin, &planned.action) {
            (_, MaterializationAction::Set(value)) => {
                let valid = cell.is_some_and(|cell| match cell.content() {
                    crate::CellContent::Formula(formula) => {
                        formula.saved_result() == &crate::SavedResult::Present(value.clone())
                    }
                    crate::CellContent::Literal(actual) => actual == value,
                });
                if !valid {
                    return Err(
                        XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed)
                            .with_detail(DETAIL_OUTPUT_VERIFICATION),
                    );
                }
            }
            (MaterializedResultOrigin::DirectFormula, MaterializationAction::Invalidate) => {
                if !cell.is_some_and(|cell| {
                    matches!(
                        cell.content(),
                        crate::CellContent::Formula(formula)
                            if formula.saved_result() == &crate::SavedResult::Missing
                    )
                }) {
                    return Err(
                        XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed)
                            .with_detail(DETAIL_OUTPUT_VERIFICATION),
                    );
                }
            }
            (
                MaterializedResultOrigin::LegacyArray { anchor, .. }
                | MaterializedResultOrigin::DynamicSpill { anchor, .. },
                MaterializationAction::Invalidate,
            ) if *id != *anchor && cell.is_some() => {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed)
                        .with_detail(DETAIL_OUTPUT_VERIFICATION),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn draft_semantics_match(
    expected: &WorkbookSnapshot,
    actual: &WorkbookSnapshot,
    materialization: &MaterializationPlan,
) -> bool {
    if expected.date_system() != actual.date_system()
        || !calculation_hints_match(
            expected.calculation_hints(),
            actual.calculation_hints(),
            !materialization.is_complete() || !expected.diagnostics().is_empty(),
        )
        || expected.defined_names() != actual.defined_names()
        || expected.sheets().len() != actual.sheets().len()
    {
        return false;
    }
    for expected_sheet in expected.sheets() {
        let Some(actual_sheet) = actual.sheet_by_id(expected_sheet.id()) else {
            return false;
        };
        if expected_sheet.name() != actual_sheet.name()
            || expected_sheet.visibility() != actual_sheet.visibility()
            || !tables_semantically_match(expected_sheet.tables(), actual_sheet.tables())
        {
            return false;
        }
        for expected_cell in expected_sheet.cells() {
            let Some(actual_cell) = actual_sheet.cell(expected_cell.address()) else {
                return false;
            };
            if !number_formats_match(expected_cell.number_format(), actual_cell.number_format())
                || !cell_content_matches(expected_cell.content(), actual_cell.content())
            {
                return false;
            }
        }
        for actual_cell in actual_sheet.cells() {
            if expected_sheet.cell(actual_cell.address()).is_none() {
                let id = crate::CalculationCellId::new(expected_sheet.id(), actual_cell.address());
                let allowed_follower = materialization.cells().get(&id).is_some_and(|planned| {
                    matches!(
                        planned.origin,
                        MaterializedResultOrigin::LegacyArray { anchor, .. }
                            | MaterializedResultOrigin::DynamicSpill { anchor, .. }
                            if anchor != id
                    ) && matches!(planned.action, MaterializationAction::Set(_))
                });
                if !allowed_follower {
                    return false;
                }
            }
        }
    }
    true
}

fn tables_semantically_match(expected: &[crate::Table], actual: &[crate::Table]) -> bool {
    expected.len() == actual.len()
        && expected
            .iter()
            .zip(actual)
            .all(|(expected, actual)| expected.semantic_eq(actual))
}

fn calculation_hints_match(
    expected: crate::CalculationHints,
    actual: crate::CalculationHints,
    request_host_recalculation: bool,
) -> bool {
    expected.mode() == actual.mode()
        && expected.calculation_id() == actual.calculation_id()
        && optional_false_matches(
            expected.iterative_calculation(),
            actual.iterative_calculation(),
        )
        && if request_host_recalculation {
            actual.full_calculation_on_load() == Some(true)
                && actual.force_full_calculation() == Some(true)
        } else {
            optional_false_matches(
                expected.full_calculation_on_load(),
                actual.full_calculation_on_load(),
            ) && optional_false_matches(
                expected.force_full_calculation(),
                actual.force_full_calculation(),
            )
        }
}

fn optional_false_matches(expected: Option<bool>, actual: Option<bool>) -> bool {
    expected.map_or_else(
        || !actual.unwrap_or(false),
        |expected| actual == Some(expected),
    )
}

fn number_formats_match(expected: &NumberFormat, actual: &NumberFormat) -> bool {
    expected.id() == actual.id()
        && expected.kind() == actual.kind()
        && (expected.id() < 164 || expected.code() == actual.code())
}

fn cell_content_matches(expected: &CellContent, actual: &CellContent) -> bool {
    match (expected, actual) {
        (CellContent::Literal(expected), CellContent::Literal(actual)) => expected == actual,
        (CellContent::Formula(expected), CellContent::Formula(actual)) => {
            expected.text() == actual.text()
                && expected.metadata() == actual.metadata()
                && expected.recalculate_always() == actual.recalculate_always()
        }
        _ => false,
    }
}

fn enforce_limit(name: &'static str, actual: u64, maximum: u64) -> Result<(), XlsxWriteError> {
    if actual > maximum {
        Err(resource_error(
            name,
            u128::from(actual),
            u128::from(maximum),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "canonical_tests.rs"]
mod tests;

fn resource_error(name: &'static str, actual: u128, maximum: u128) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!("{name}: {actual} > {maximum}"))
}

fn zip_error(error: zip::result::ZipError) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
}

fn io_error(error: std::io::Error) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
}
