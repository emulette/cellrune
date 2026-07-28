mod cell_value;
mod defined_name;
mod formula_cell;
mod formula_reference;
mod merge;
mod metadata;
mod phonetic;
mod shared_strings;
mod styles;
mod table;
mod workbook_xml;
mod worksheet;
mod worksheet_cell;

#[cfg(test)]
mod formula_tests;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use self::phonetic::PhoneticReadBudget;
use self::shared_strings::SharedStrings;
use self::styles::Styles;
use super::error::{compatibility, detail};
use super::package::{OpenedPackage, PackageSummary, PartPath, WorkbookPackageKind, open_package};
use super::{ReadOptions, XlsxErrorCode, XlsxReadError};
use crate::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, DocumentPresentation, InputHash, Provenance,
    ProviderIdentity, Sheet, SheetId, SourceLocation, TableId, WorkbookSnapshot, WorkbookSource,
    WorkbookSourceKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PresentationCapture {
    None,
    Document,
}

pub(super) struct ReadWorkbook {
    pub(super) workbook: WorkbookSnapshot,
    pub(super) presentation: DocumentPresentation,
    pub(super) package_summary: PackageSummary,
    pub(super) workbook_part: PartPath,
    pub(super) worksheet_parts: BTreeMap<SheetId, PartPath>,
    pub(super) package_kind: WorkbookPackageKind,
}

/// Reads workbook metadata, sparse literal cells, formulas, and saved results from a bounded XLSX
/// stream.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when the package cannot be read into a valid bounded workbook
/// snapshot.
pub fn read_xlsx<R: Read + Seek>(
    reader: R,
    options: ReadOptions,
) -> Result<WorkbookSnapshot, XlsxReadError> {
    read_xlsx_with_identity(
        reader,
        options,
        WorkbookSourceKind::Reader,
        None,
        PresentationCapture::None,
    )
    .map(|result| result.workbook)
}

/// Opens and reads an XLSX workbook from a filesystem path without retaining the host path.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when the file cannot be opened or its contents cannot be read into
/// a valid bounded workbook snapshot.
pub fn read_xlsx_path(
    path: impl AsRef<Path>,
    options: ReadOptions,
) -> Result<WorkbookSnapshot, XlsxReadError> {
    let reader = File::open(path)
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::Io).with_cause(error))?;
    read_xlsx_with_identity(
        reader,
        options,
        WorkbookSourceKind::Path,
        None,
        PresentationCapture::None,
    )
    .map(|result| result.workbook)
}

/// Reads workbook metadata, sparse cells, formulas, and saved results from in-memory XLSX bytes.
///
/// # Errors
///
/// Returns an [`XlsxReadError`] when `bytes` do not contain a valid bounded workbook.
pub fn read_xlsx_bytes(
    bytes: &[u8],
    options: ReadOptions,
) -> Result<WorkbookSnapshot, XlsxReadError> {
    read_xlsx_with_identity(
        Cursor::new(bytes),
        options,
        WorkbookSourceKind::Bytes,
        None,
        PresentationCapture::None,
    )
    .map(|result| result.workbook)
}

pub(super) fn read_xlsx_with_identity<R: Read + Seek>(
    reader: R,
    options: ReadOptions,
    source_kind: WorkbookSourceKind,
    input_hash: Option<InputHash>,
    capture: PresentationCapture,
) -> Result<ReadWorkbook, XlsxReadError> {
    let mut package = open_package(reader, options)?;
    let limits = package.limits();

    let workbook_part = package.workbook_part().clone();
    let workbook_bytes = package.read_part(&workbook_part)?;
    let workbook = workbook_xml::parse(&workbook_bytes, &workbook_part, limits)?;

    let styles = read_styles(&mut package)?;
    let mut phonetic_budget = PhoneticReadBudget::default();
    let shared_strings = read_shared_strings(
        &mut package,
        capture,
        styles.font_count(),
        &mut phonetic_budget,
    )?;
    let cell_metadata = read_cell_metadata(&mut package)?;
    let mut diagnostics = compatibility_diagnostics(&package, &workbook_part)?;
    let mut presentation = DocumentPresentation::default();
    let mut sheets = Vec::with_capacity(workbook.sheets.len());
    let mut worksheet_parts = BTreeMap::new();
    let mut used_relationships = BTreeSet::new();
    let mut total_cells = 0_u64;
    let mut total_merged_ranges = 0_u64;
    let mut total_tables = 0_u64;
    let defined_name_keys = workbook
        .defined_names
        .iter()
        .map(|name| Box::<str>::from(name.lookup_key()))
        .collect::<BTreeSet<_>>();
    let mut seen_table_ids = BTreeSet::<TableId>::new();
    let mut seen_table_display_names = BTreeSet::<Box<str>>::new();
    let mut total_formula_bytes = workbook
        .defined_names
        .iter()
        .map(|name| name.formula().as_str().len() as u64)
        .fold(0_u64, u64::saturating_add);
    if total_formula_bytes > limits.max_total_formula_bytes() {
        return Err(XlsxReadError::new(XlsxErrorCode::TotalFormulaBytesTooLarge)
            .at_source(workbook_part.source_id()));
    }
    for metadata in workbook.sheets {
        let worksheet_part = package
            .worksheet_part(&metadata.relationship_id)
            .cloned()
            .ok_or_else(|| {
                XlsxReadError::new(XlsxErrorCode::MissingSheetRelationship)
                    .with_detail(detail::UNKNOWN_SHEET_RELATIONSHIP)
                    .at_source(workbook_part.source_id())
            })?;
        worksheet_parts.insert(metadata.id, worksheet_part.clone());
        used_relationships.insert(metadata.relationship_id);
        let worksheet_bytes = package.read_part(&worksheet_part)?;
        let mut sheet = Sheet::new(metadata.id, metadata.name, metadata.visibility);
        let mut table_relationship_ids = Vec::new();
        worksheet::parse(
            &worksheet_bytes,
            &worksheet_part,
            limits,
            worksheet::WorksheetResources {
                shared_strings: shared_strings.as_ref(),
                styles: &styles,
                cell_metadata: cell_metadata.as_ref(),
            },
            capture,
            worksheet::WorksheetOutput {
                sheet: &mut sheet,
                total_cells: &mut total_cells,
                total_formula_bytes: &mut total_formula_bytes,
                total_merged_ranges: &mut total_merged_ranges,
                total_tables: &mut total_tables,
                presentation: &mut presentation,
                phonetic_budget: &mut phonetic_budget,
                diagnostics: &mut diagnostics,
                table_relationship_ids: &mut table_relationship_ids,
            },
        )?;
        read_sheet_tables(SheetTableContext {
            package: &mut package,
            worksheet_part: &worksheet_part,
            table_relationship_ids: &table_relationship_ids,
            sheet: &mut sheet,
            defined_name_keys: &defined_name_keys,
            seen_table_ids: &mut seen_table_ids,
            seen_table_display_names: &mut seen_table_display_names,
            diagnostics: &mut diagnostics,
        })?;
        sheets.push(sheet);
    }
    if used_relationships.len() != package.worksheet_count() {
        return Err(
            XlsxReadError::new(XlsxErrorCode::InvalidWorkbook).at_source(workbook_part.source_id())
        );
    }

    let provider = ProviderIdentity::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::InvalidWorkbook).with_cause(error))?;
    let package_summary = package.summary();
    let package_kind = package.workbook_kind();
    let workbook = WorkbookSnapshot::new_with_metadata(
        sheets,
        workbook.defined_names,
        diagnostics,
        workbook.date_system,
        workbook.calculation_hints,
        WorkbookSource::new(source_kind, Some(package.archive_bytes())),
        Provenance::new(provider, input_hash),
    )
    .map_err(|error| {
        XlsxReadError::new(XlsxErrorCode::InvalidWorkbook)
            .at_source(workbook_part.source_id())
            .with_cause(error)
    })?;
    Ok(ReadWorkbook {
        workbook,
        presentation,
        package_summary,
        workbook_part,
        worksheet_parts,
        package_kind,
    })
}

struct SheetTableContext<'a, R: Read + Seek> {
    package: &'a mut OpenedPackage<R>,
    worksheet_part: &'a PartPath,
    table_relationship_ids: &'a [Box<str>],
    sheet: &'a mut Sheet,
    defined_name_keys: &'a BTreeSet<Box<str>>,
    seen_table_ids: &'a mut BTreeSet<TableId>,
    seen_table_display_names: &'a mut BTreeSet<Box<str>>,
    diagnostics: &'a mut Vec<Diagnostic>,
}

/// Resolves and parses the table parts one worksheet references through `<tableParts>`.
///
/// A relationship id that resolves to no table relationship, an invalid table definition,
/// and a table that violates workbook identity constraints each drop only that table with a
/// warning diagnostic; the read itself keeps going. The worksheet parser has already charged
/// every `<tablePart>` declaration against the workbook-wide table budget.
fn read_sheet_tables<R: Read + Seek>(
    context: SheetTableContext<'_, R>,
) -> Result<(), XlsxReadError> {
    let SheetTableContext {
        package,
        worksheet_part,
        table_relationship_ids,
        sheet,
        defined_name_keys,
        seen_table_ids,
        seen_table_display_names,
        diagnostics,
    } = context;
    if table_relationship_ids.is_empty() {
        return Ok(());
    }
    let limits = package.limits();
    let table_parts = package.worksheet_table_parts(worksheet_part)?;
    let budget = crate::xlsx::xml::XmlBudget::new(
        limits,
        worksheet_part.source_id(),
        XlsxErrorCode::InvalidWorksheet,
    );
    let mut tables = Vec::new();
    let mut programmatic_names = BTreeSet::<Box<str>>::new();
    for relationship_id in table_relationship_ids {
        let Some(part) = table_parts.get(relationship_id) else {
            table::push_invalid_diagnostic(
                diagnostics,
                compatibility::TABLE_UNRESOLVED_RELATIONSHIP,
                sheet.id(),
                &budget,
            )?;
            continue;
        };
        let bytes = package.read_part(part)?;
        let Some(parsed) = table::parse(&bytes, part, limits, sheet.id(), diagnostics)? else {
            continue;
        };
        if seen_table_ids.contains(&parsed.id()) {
            table::push_table_diagnostic(
                diagnostics,
                compatibility::TABLE_DUPLICATE_ID_CODE,
                compatibility::TABLE_DUPLICATE_ID_MESSAGE,
                &parsed.id().get().to_string(),
                sheet.id(),
                &budget,
            )?;
            continue;
        }
        let display_key = parsed.display_name().lookup_key();
        if defined_name_keys.contains(display_key) {
            table::push_table_diagnostic(
                diagnostics,
                compatibility::TABLE_DEFINED_NAME_CONFLICT_CODE,
                compatibility::TABLE_DEFINED_NAME_CONFLICT_MESSAGE,
                parsed.display_name().as_str(),
                sheet.id(),
                &budget,
            )?;
            continue;
        }
        if seen_table_display_names.contains(display_key) {
            table::push_table_diagnostic(
                diagnostics,
                compatibility::TABLE_DUPLICATE_DISPLAY_NAME_CODE,
                compatibility::TABLE_DUPLICATE_DISPLAY_NAME_MESSAGE,
                parsed.display_name().as_str(),
                sheet.id(),
                &budget,
            )?;
            continue;
        }
        let programmatic_key = parsed.name().lookup_key();
        if programmatic_names.contains(programmatic_key) {
            table::push_table_diagnostic(
                diagnostics,
                compatibility::TABLE_DUPLICATE_PROGRAMMATIC_NAME_CODE,
                compatibility::TABLE_DUPLICATE_PROGRAMMATIC_NAME_MESSAGE,
                parsed.name().as_str(),
                sheet.id(),
                &budget,
            )?;
            continue;
        }
        seen_table_ids.insert(parsed.id());
        seen_table_display_names.insert(Box::from(display_key));
        programmatic_names.insert(Box::from(programmatic_key));
        tables.push(parsed);
    }
    sheet.set_tables(tables);
    Ok(())
}

fn read_cell_metadata<R: Read + Seek>(
    package: &mut OpenedPackage<R>,
) -> Result<Option<metadata::CellMetadata>, XlsxReadError> {
    let Some(part) = package.metadata_part().cloned() else {
        return Ok(None);
    };
    let bytes = package.read_part(&part)?;
    metadata::parse(&bytes, &part, package.limits()).map(Some)
}

fn compatibility_diagnostics<R: Read + Seek>(
    package: &OpenedPackage<R>,
    workbook_part: &super::package::PartPath,
) -> Result<Vec<Diagnostic>, XlsxReadError> {
    let mut diagnostics = Vec::new();
    if package.has_external_links() {
        diagnostics.push(compatibility_diagnostic(
            compatibility::EXTERNAL_LINK_CODE,
            compatibility::EXTERNAL_LINK_MESSAGE,
            workbook_part,
        )?);
    }
    if package.has_macros() {
        diagnostics.push(compatibility_diagnostic(
            compatibility::MACRO_CODE,
            compatibility::MACRO_MESSAGE,
            workbook_part,
        )?);
    }
    Ok(diagnostics)
}

fn compatibility_diagnostic(
    code: &'static str,
    message: &'static str,
    workbook_part: &super::package::PartPath,
) -> Result<Diagnostic, XlsxReadError> {
    let code = DiagnosticCode::new(code)
        .map_err(|error| XlsxReadError::new(XlsxErrorCode::InvalidWorkbook).with_cause(error))?;
    Diagnostic::new(
        code,
        DiagnosticSeverity::Warning,
        message,
        Some(SourceLocation::source(workbook_part.source_id())),
    )
    .map_err(|error| XlsxReadError::new(XlsxErrorCode::InvalidWorkbook).with_cause(error))
}

fn read_styles<R: Read + Seek>(package: &mut OpenedPackage<R>) -> Result<Styles, XlsxReadError> {
    let Some(part) = package.styles_part().cloned() else {
        return Ok(Styles::default());
    };
    let bytes = package.read_part(&part)?;
    styles::parse(&bytes, &part, package.limits())
}

fn read_shared_strings<R: Read + Seek>(
    package: &mut OpenedPackage<R>,
    capture: PresentationCapture,
    font_count: u32,
    phonetic_budget: &mut PhoneticReadBudget,
) -> Result<Option<SharedStrings>, XlsxReadError> {
    let Some(part) = package.shared_strings_part().cloned() else {
        return Ok(None);
    };
    let bytes = package.read_part(&part)?;
    shared_strings::parse(
        &bytes,
        &part,
        package.limits(),
        capture,
        font_count,
        phonetic_budget,
    )
    .map(Some)
}
