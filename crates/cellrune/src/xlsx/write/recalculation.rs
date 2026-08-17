use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};
use std::path::Path;
use zip::ZipArchive;

use super::materialization::{MaterializationAction, MaterializationPlan};
use super::package::PackageWritePlan;
use super::package_metadata_patch::{
    remove_calculation_chain_relationship, remove_content_type_overrides,
};
use super::workbook_patch::patch_calculation_properties;
use super::worksheet_patch::{WorksheetCacheAction, WorksheetCellUpdate, patch_worksheet};
use super::{
    RecalculatedWorkbook, RecalculationWriteOptions, WriteProvenance, WriteReport, XlsxWriteError,
    XlsxWriteErrorCode,
};
use crate::xlsx::document::{OpenOptions, XlsxDocument, open_xlsx_document_bytes};
use crate::xlsx::package::PartPath;
use crate::{
    CalculationCellId, CalculationSnapshot, CellContent, Diagnostic, DiagnosticCode,
    DiagnosticSeverity, InputHash, MaterializedResultOrigin, ReadOptions, SavedResult,
    SourceLocation, WorkbookSnapshot,
};

const CONTENT_TYPES_PART: &[u8] = b"[Content_Types].xml";
const DIAGNOSTIC_INVALIDATED_CODE: &str = "xlsx.write.invalidated_result";
const DIAGNOSTIC_INVALIDATED_MESSAGE: &str =
    "an unavailable formula cache was removed and host recalculation was requested";
const DETAIL_CALCULATION_INPUT_MISMATCH: &str =
    "calculation input hash does not match the document archive";
const DETAIL_CALCULATION_FINGERPRINT_MISMATCH: &str =
    "calculation semantic fingerprint does not match the document workbook";
const DETAIL_CALCULATION_REVISION_MISMATCH: &str =
    "calculation semantic revision does not match the document revision";
const DETAIL_EDITED_CELL_COUNT: &str = "max_edited_cells";
const DETAIL_EDITED_SHEET_COUNT: &str = "max_edited_sheets";
const DETAIL_VERIFICATION_BYTES: &str = "max_verification_read_bytes";
const DETAIL_SEMANTIC_VERIFICATION: &str =
    "reopened workbook does not preserve the declared semantic contract";

/// Materializes a calculation, verifies the completed package, and returns owned archive bytes.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when identity, completeness, resource, preservation, package, or
/// output-verification requirements are not satisfied.
pub fn write_recalculated_xlsx_bytes(
    document: &XlsxDocument,
    calculation: &CalculationSnapshot,
    options: RecalculationWriteOptions,
) -> Result<RecalculatedWorkbook, XlsxWriteError> {
    validate_calculation_identity(document, calculation)?;
    let limits = options.write_options().limits();
    let materialization = MaterializationPlan::new(calculation, options.policy(), limits)?;

    let mut updates_by_sheet =
        BTreeMap::<crate::SheetId, BTreeMap<crate::CellAddress, WorksheetCellUpdate>>::new();
    let mut expected = BTreeMap::<CalculationCellId, WorksheetCacheAction>::new();
    for (id, planned) in materialization.cells() {
        let action = match &planned.action {
            MaterializationAction::Set(value) => WorksheetCacheAction::Set(value.clone()),
            MaterializationAction::Invalidate => WorksheetCacheAction::Invalidate,
        };
        let requires_formula = match planned.origin {
            MaterializedResultOrigin::DirectFormula => true,
            MaterializedResultOrigin::LegacyArray { anchor, .. }
            | MaterializedResultOrigin::DynamicSpill { anchor, .. } => *id == anchor,
        };
        let replaced = updates_by_sheet.entry(id.sheet_id()).or_default().insert(
            id.address(),
            WorksheetCellUpdate {
                action: action.clone(),
                requires_formula,
            },
        );
        if replaced.is_some() || expected.insert(*id, action).is_some() {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::ConflictingPartOperation)
                    .with_detail(DETAIL_SEMANTIC_VERIFICATION),
            );
        }
    }
    enforce_count(
        DETAIL_EDITED_CELL_COUNT,
        expected.len(),
        limits.max_edited_cells(),
    )?;
    enforce_count(
        DETAIL_EDITED_SHEET_COUNT,
        updates_by_sheet.len(),
        limits.max_edited_sheets(),
    )?;

    let source = document.preserved_package();
    let mut replacements = BTreeMap::<PartPath, Vec<u8>>::new();
    for (sheet_id, updates) in &updates_by_sheet {
        let part = document.worksheet_part_path(*sheet_id).ok_or_else(|| {
            XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
                .with_detail(DETAIL_SEMANTIC_VERIFICATION)
        })?;
        let original = source.read_part(part)?;
        let rewritten = patch_worksheet(&original, part, updates, limits)?;
        replacements.insert(part.clone(), rewritten);
    }

    let request_host_recalculation =
        !materialization.is_complete() || !document.workbook().diagnostics().is_empty();
    let workbook_part = document.workbook_part_path();
    let original_workbook = source.read_part(workbook_part)?;
    replacements.insert(
        workbook_part.clone(),
        patch_calculation_properties(
            &original_workbook,
            workbook_part,
            request_host_recalculation,
            limits,
        )?,
    );

    let relationship_part = workbook_part
        .relationship_part()
        .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
    let relationship_bytes = source.read_part(&relationship_part)?;
    let chain = remove_calculation_chain_relationship(
        &relationship_bytes,
        &relationship_part,
        workbook_part,
        limits,
    )?;
    if let Some(bytes) = chain.relationship_bytes {
        replacements.insert(relationship_part, bytes);
    }
    let removals = chain.removed_parts;
    if !removals.is_empty() {
        let content_types_part = PartPath::from_archive_name(CONTENT_TYPES_PART)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        let content_types = source.read_part(&content_types_part)?;
        replacements.insert(
            content_types_part.clone(),
            remove_content_type_overrides(&content_types, &content_types_part, &removals, limits)?,
        );
    }

    let changed_parts = replacements
        .keys()
        .map(PartPath::source_id)
        .collect::<Vec<_>>();
    let removed_parts = removals.iter().map(PartPath::source_id).collect::<Vec<_>>();
    let plan = PackageWritePlan::modified(source, replacements, &removals, limits)?;
    let bytes = plan.write_to_vec(source)?;
    if bytes.len() as u64 > limits.max_verification_read_bytes() {
        return Err(resource_error(
            DETAIL_VERIFICATION_BYTES,
            bytes.len() as u64,
            limits.max_verification_read_bytes(),
        ));
    }
    verify_output(document, &bytes, &expected, &removals, limits)?;

    let diagnostics = invalidation_diagnostics(document, materialization.invalidated_cells())?;
    let provenance = WriteProvenance::new(
        Some(document.input_hash()),
        document.semantic_revision(),
        document.presentation_revision(),
        calculation.provenance().provider().clone(),
        calculation.options(),
    );
    let report = WriteReport::new(
        options.policy(),
        materialization.materialized_count(),
        materialization.invalidated_cells().to_vec(),
        changed_parts,
        removed_parts,
        diagnostics,
        provenance,
    );
    Ok(RecalculatedWorkbook::new(bytes, report, document.kind()))
}

/// Writes a fully prepared and verified recalculated package to an output.
///
/// The output is prepared before the supplied writer is touched.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] for preparation failures or when the output cannot be written or
/// flushed.
pub fn write_recalculated_xlsx<W: Write>(
    document: &XlsxDocument,
    calculation: &CalculationSnapshot,
    writer: &mut W,
    options: RecalculationWriteOptions,
) -> Result<WriteReport, XlsxWriteError> {
    let output = write_recalculated_xlsx_bytes(document, calculation, options)?;
    writer.write_all(output.bytes()).map_err(io_write_error)?;
    writer.flush().map_err(io_write_error)?;
    let (_, report) = output.into_parts();
    Ok(report)
}

/// Saves a verified recalculated package to a new path or explicitly replaces the destination.
///
/// # Errors
///
/// Returns an [`XlsxWriteError`] when the destination kind is wrong, already exists without
/// replacement permission, cannot be written atomically, or package preparation fails.
pub fn write_recalculated_xlsx_path(
    document: &XlsxDocument,
    calculation: &CalculationSnapshot,
    path: impl AsRef<Path>,
    options: RecalculationWriteOptions,
) -> Result<WriteReport, XlsxWriteError> {
    let path = path.as_ref();
    let output = write_recalculated_xlsx_bytes(document, calculation, options)?;
    output.save_path(path, options.write_options())?;
    let (_, report) = output.into_parts();
    Ok(report)
}

fn validate_calculation_identity(
    document: &XlsxDocument,
    calculation: &CalculationSnapshot,
) -> Result<(), XlsxWriteError> {
    validate_calculation_identity_parts(
        Some(document.input_hash()),
        document.workbook(),
        calculation,
    )
}

fn validate_calculation_identity_parts(
    input_hash: Option<InputHash>,
    workbook: &WorkbookSnapshot,
    calculation: &CalculationSnapshot,
) -> Result<(), XlsxWriteError> {
    if calculation.provenance().input_hash() != input_hash {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::SourceIdentityMismatch)
                .with_detail(DETAIL_CALCULATION_INPUT_MISMATCH),
        );
    }
    if calculation.source_revision() != workbook.semantic_revision() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::StaleSemanticRevision)
                .with_detail(DETAIL_CALCULATION_REVISION_MISMATCH),
        );
    }
    if calculation.source_fingerprint() != workbook.fingerprint() {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::StaleSemanticRevision)
                .with_detail(DETAIL_CALCULATION_FINGERPRINT_MISMATCH),
        );
    }
    Ok(())
}

fn verify_output(
    source: &XlsxDocument,
    bytes: &[u8],
    expected: &BTreeMap<CalculationCellId, WorksheetCacheAction>,
    removals: &BTreeSet<PartPath>,
    limits: super::WriteLimits,
) -> Result<(), XlsxWriteError> {
    let read_limits = source
        .read_options()
        .limits()
        .with_max_archive_bytes(limits.max_verification_read_bytes())
        .map_err(|error| {
            XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed).with_cause(error)
        })?;
    let reopened = open_xlsx_document_bytes(bytes, OpenOptions::new(ReadOptions::new(read_limits)))
        .map_err(|error| {
            XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed).with_cause(error)
        })?;
    if reopened.kind() != source.kind()
        || reopened.workbook().sheets().len() != source.workbook().sheets().len()
        || reopened.workbook().defined_names() != source.workbook().defined_names()
        || reopened.workbook().date_system() != source.workbook().date_system()
    {
        return Err(verification_error());
    }
    for original_sheet in source.workbook().sheets() {
        let reopened_sheet = reopened
            .workbook()
            .sheet_by_id(original_sheet.id())
            .ok_or_else(verification_error)?;
        if reopened_sheet.name() != original_sheet.name()
            || reopened_sheet.visibility() != original_sheet.visibility()
            || reopened_sheet.tables() != original_sheet.tables()
        {
            return Err(verification_error());
        }
        for original_cell in original_sheet.cells() {
            let id = CalculationCellId::new(original_sheet.id(), original_cell.address());
            let reopened_cell = reopened_sheet.cell(original_cell.address());
            if let Some(action) = expected.get(&id) {
                verify_target_cell(original_cell.content(), reopened_cell, action)?;
            } else if reopened_cell != Some(original_cell) {
                return Err(verification_error());
            }
        }
    }
    for (id, action) in expected {
        let sheet = reopened
            .workbook()
            .sheet_by_id(id.sheet_id())
            .ok_or_else(verification_error)?;
        if source
            .workbook()
            .sheet_by_id(id.sheet_id())
            .and_then(|sheet| sheet.cell(id.address()))
            .is_none()
        {
            verify_inserted_cell(sheet.cell(id.address()), action)?;
        }
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| {
        XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed).with_cause(error)
    })?;
    for removed in removals {
        if (0..archive.len()).any(|index| {
            archive.by_index(index).is_ok_and(|file| {
                PartPath::from_archive_name(file.name_raw()).is_ok_and(|part| &part == removed)
            })
        }) {
            return Err(verification_error());
        }
    }
    Ok(())
}

fn verify_target_cell(
    original: &CellContent,
    reopened: Option<&crate::Cell>,
    action: &WorksheetCacheAction,
) -> Result<(), XlsxWriteError> {
    match original {
        CellContent::Formula(original_formula) => {
            let reopened = reopened.ok_or_else(verification_error)?;
            let CellContent::Formula(reopened_formula) = reopened.content() else {
                return Err(verification_error());
            };
            if original_formula.text() != reopened_formula.text()
                || original_formula.metadata() != reopened_formula.metadata()
                || original_formula.recalculate_always() != reopened_formula.recalculate_always()
            {
                return Err(verification_error());
            }
            let valid = match action {
                WorksheetCacheAction::Set(value) => {
                    reopened_formula.saved_result() == &SavedResult::Present(value.clone())
                }
                WorksheetCacheAction::Invalidate => {
                    reopened_formula.saved_result() == &SavedResult::Missing
                }
            };
            if !valid {
                return Err(verification_error());
            }
        }
        CellContent::Literal(_) => verify_inserted_cell(reopened, action)?,
    }
    Ok(())
}

fn verify_inserted_cell(
    cell: Option<&crate::Cell>,
    action: &WorksheetCacheAction,
) -> Result<(), XlsxWriteError> {
    match action {
        WorksheetCacheAction::Set(value)
            if cell.is_some_and(|cell| cell.content() == &CellContent::Literal(value.clone())) =>
        {
            Ok(())
        }
        WorksheetCacheAction::Invalidate if cell.is_none() => Ok(()),
        WorksheetCacheAction::Invalidate => Err(verification_error()),
        WorksheetCacheAction::Set(_) => Err(verification_error()),
    }
}

fn invalidation_diagnostics(
    document: &XlsxDocument,
    invalidated: &[CalculationCellId],
) -> Result<Vec<Diagnostic>, XlsxWriteError> {
    let code = DiagnosticCode::new(DIAGNOSTIC_INVALIDATED_CODE).map_err(|error| {
        XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_cause(error)
    })?;
    invalidated
        .iter()
        .map(|cell| {
            let source = document
                .worksheet_part_path(cell.sheet_id())
                .ok_or_else(verification_error)?
                .source_id();
            Diagnostic::new(
                code.clone(),
                DiagnosticSeverity::Warning,
                DIAGNOSTIC_INVALIDATED_MESSAGE,
                Some(SourceLocation::cell(
                    source,
                    cell.sheet_id(),
                    cell.address(),
                )),
            )
            .map_err(|error| {
                XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_cause(error)
            })
        })
        .collect()
}

fn enforce_count(name: &'static str, actual: usize, maximum: u64) -> Result<(), XlsxWriteError> {
    if actual as u64 > maximum {
        return Err(resource_error(name, actual as u64, maximum));
    }
    Ok(())
}

fn resource_error(name: &'static str, actual: u64, maximum: u64) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
        .with_detail(format!("{name}: {actual} > {maximum}"))
}

fn invalid_plan_with_cause(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
        .at_source(source.source_id())
        .with_cause(cause)
}

fn verification_error() -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::OutputVerificationFailed)
        .with_detail(DETAIL_SEMANTIC_VERIFICATION)
}

fn io_write_error(error: std::io::Error) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::Io).with_cause(error)
}

#[cfg(test)]
mod identity_tests {
    use super::validate_calculation_identity_parts;
    use crate::{
        CalculationOptions, CellAddress, CellValue, FiniteNumber, WorkbookDraft,
        XlsxWriteErrorCode, calculate_workbook,
    };

    #[test]
    fn fingerprint_only_mismatch_is_stale() {
        let base = WorkbookDraft::new();
        let calculation = calculate_workbook(base.workbook(), CalculationOptions::default());
        let mut changed = base.clone();
        let sheet_id = changed.workbook().sheets()[0].id();
        changed
            .set_cell_value(
                sheet_id,
                CellAddress::from_a1("A1").expect("constant address"),
                CellValue::Number(FiniteNumber::new(1.0).expect("finite value")),
            )
            .expect("test edit");
        let same_revision = changed.workbook().clone().with_semantic_revision(0);

        let error = validate_calculation_identity_parts(None, &same_revision, &calculation)
            .expect_err("semantic fingerprint mismatch must be stale");
        assert_eq!(error.code(), XlsxWriteErrorCode::StaleSemanticRevision);
    }
}
