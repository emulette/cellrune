use std::collections::{BTreeMap, BTreeSet};

use super::canonical::{generated_worksheet_xml, verify_draft_output};
use super::materialization::{MaterializationAction, MaterializationPlan};
use super::package::PackageWritePlan;
use super::package_additions::{
    NewRelationship, RelationshipIdAllocator, append_content_type_overrides, append_relationships,
};
use super::package_metadata_patch::{
    remove_calculation_chain_relationship, remove_content_type_overrides,
};
use super::phonetic_preservation::ensure_phonetic_edit_preservation;
use super::serialization::validate_phonetic_limits;
use super::styles_patch::{StyleRequest, plan_document_styles};
use super::table_patch::patch_table_xml;
use super::workbook_edit::{WorkbookPatchOptions, patch_workbook_semantics};
use super::worksheet_edit::{
    WorksheetSemanticEdit, patch_worksheet_semantics, read_cell_style_indices,
};
use super::worksheet_patch::{WorksheetCacheAction, WorksheetCellUpdate, patch_worksheet};
use super::worksheet_view_edit::patch_frozen_pane;
use super::{
    RecalculatedWorkbook, RecalculationWriteOptions, WriteProvenance, WriteReport, XlsxWriteError,
    XlsxWriteErrorCode,
};
use crate::draft::DraftCellMutation;
use crate::xlsx::package::PartPath;
use crate::{
    CalculationCellId, CalculationSnapshot, MaterializedResultOrigin, SheetId, WorkbookDraft,
};

const CONTENT_TYPES_PART: &[u8] = b"[Content_Types].xml";
const CONTENT_WORKSHEET: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CONTENT_STYLES: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const REL_WORKSHEET: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
const DETAIL_CALCULATION_IDENTITY: &str = "calculation does not belong to the current draft";
const DETAIL_MISSING_SHEET_PART: &str = "source worksheet part mapping was not found";
const DETAIL_MISSING_TABLE_PART: &str = "source table part mapping was not found";
const DETAIL_EDITED_CELL_COUNT: &str = "max_edited_cells";
const DETAIL_EDITED_SHEET_COUNT: &str = "max_edited_sheets";
const DETAIL_DYNAMIC_FORMULA_METADATA: &str =
    "document-backed dynamic formula edits require source metadata index merging";
const DETAIL_RTL_FROZEN_PANE: &str =
    "right-to-left frozen-pane authoring requires a verified native contract";

struct ExistingSheetSource {
    part: PartPath,
    bytes: Vec<u8>,
    styles: BTreeMap<crate::CellAddress, usize>,
}

pub(crate) fn write_document_draft(
    draft: &WorkbookDraft,
    calculation: &CalculationSnapshot,
    options: RecalculationWriteOptions,
) -> Result<RecalculatedWorkbook, XlsxWriteError> {
    let document = draft
        .source_document()
        .ok_or_else(|| invalid_plan(DETAIL_CALCULATION_IDENTITY))?;
    if !calculation.matches_workbook(draft.workbook()) {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::StaleSemanticRevision)
                .with_detail(DETAIL_CALCULATION_IDENTITY),
        );
    }
    validate_dynamic_formula_edits(draft, document)?;
    if !draft.workbook_changed()
        && draft.cell_mutations().is_empty()
        && draft.presentation_cell_mutations().is_empty()
        && draft.presentation_sheet_mutations().is_empty()
        && draft.changed_table_ids().is_empty()
    {
        return super::recalculation::write_recalculated_xlsx_bytes(document, calculation, options);
    }
    let limits = options.write_options().limits();
    validate_phonetic_limits(draft.presentation(), limits)?;
    enforce_count(
        DETAIL_EDITED_CELL_COUNT,
        draft
            .cell_mutations()
            .keys()
            .chain(draft.presentation_cell_mutations().iter())
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        limits.max_edited_cells(),
    )?;
    let materialization = MaterializationPlan::new(calculation, options.policy(), limits)?;
    let source = document.preserved_package();
    let shared_strings_source = document
        .package_summary()
        .shared_strings_part()
        .map(|source_id| PartPath::from_archive_name(source_id.as_str().as_bytes()))
        .transpose()
        .map_err(|error| invalid_plan_with_cause(document.workbook_part_path(), error))?;
    let shared_strings_bytes = shared_strings_source
        .as_ref()
        .map(|part| source.read_part(part))
        .transpose()?;
    let mut replacements = BTreeMap::<PartPath, Vec<u8>>::new();
    let mut additions = BTreeMap::<PartPath, Vec<u8>>::new();
    let mut new_content_types = BTreeMap::<PartPath, &'static str>::new();
    let mut new_relationships = Vec::<NewRelationship>::new();

    for table_id in draft.changed_table_ids() {
        let part = document
            .table_part_path(*table_id)
            .cloned()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_TABLE_PART))?;
        let table = draft
            .workbook()
            .table_by_id(*table_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_TABLE_PART))?;
        let bytes = source.read_part(&part)?;
        replacements.insert(part.clone(), patch_table_xml(&bytes, &part, table, limits)?);
    }

    let workbook_part = document.workbook_part_path();
    let relationship_part = workbook_part
        .relationship_part()
        .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
    let relationship_bytes = source.read_part(&relationship_part)?;
    let mut relationship_ids =
        RelationshipIdAllocator::from_xml(&relationship_bytes, &relationship_part, limits)?;
    let mut added_relationship_ids = BTreeMap::<SheetId, String>::new();
    let mut added_parts = BTreeMap::<SheetId, PartPath>::new();
    for sheet_id in draft.added_sheets() {
        let relationship_id =
            relationship_ids.allocate(&format!("rIdCellRuneSheet{}", sheet_id.get()));
        let target = format!("worksheets/cellrune-sheet-{}.xml", sheet_id.get());
        let part = PartPath::resolve_relationship(Some(workbook_part), &target)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        added_relationship_ids.insert(*sheet_id, relationship_id.clone());
        added_parts.insert(*sheet_id, part.clone());
        new_relationships.push(NewRelationship {
            id: relationship_id,
            kind: REL_WORKSHEET,
            target,
        });
        new_content_types.insert(part, CONTENT_WORKSHEET);
    }

    let relevant_existing_sheets = draft
        .cell_mutations()
        .keys()
        .map(|id| id.sheet_id())
        .chain(
            draft
                .presentation_cell_mutations()
                .iter()
                .map(|id| id.sheet_id()),
        )
        .chain(materialization.cells().keys().map(|id| id.sheet_id()))
        .chain(draft.presentation_sheet_mutations().iter().copied())
        .filter(|sheet_id| !draft.added_sheets().contains(sheet_id))
        .collect::<BTreeSet<_>>();
    enforce_count(
        DETAIL_EDITED_SHEET_COUNT,
        relevant_existing_sheets
            .len()
            .saturating_add(draft.added_sheets().len()),
        limits.max_edited_sheets(),
    )?;
    let mut sheet_sources = BTreeMap::<SheetId, ExistingSheetSource>::new();
    for sheet_id in relevant_existing_sheets {
        let part = document
            .worksheet_part_path(sheet_id)
            .cloned()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let bytes = source.read_part(&part)?;
        let targets = draft
            .cell_mutations()
            .keys()
            .chain(draft.presentation_cell_mutations().iter())
            .filter(|id| id.sheet_id() == sheet_id)
            .map(|id| id.address())
            .collect::<BTreeSet<_>>();
        let styles = read_cell_style_indices(&bytes, &part, &targets, limits)?;
        sheet_sources.insert(
            sheet_id,
            ExistingSheetSource {
                part,
                bytes,
                styles,
            },
        );
    }

    let styles_part = document
        .package_summary()
        .styles_part()
        .map(|source_id| PartPath::from_archive_name(source_id.as_str().as_bytes()))
        .transpose()
        .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
    let generated_styles_part = PartPath::resolve_relationship(Some(workbook_part), "styles.xml")
        .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
    let target_styles_part = styles_part
        .as_ref()
        .unwrap_or(&generated_styles_part)
        .clone();
    let mut style_requests = BTreeMap::<CalculationCellId, StyleRequest>::new();
    for (id, mutation) in draft.cell_mutations().iter() {
        let DraftCellMutation::Upsert {
            number_format_changed: true,
        } = mutation
        else {
            continue;
        };
        let cell = draft
            .workbook()
            .sheet_by_id(id.sheet_id())
            .and_then(|sheet| sheet.cell(id.address()))
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let base_index = if draft.added_sheets().contains(&id.sheet_id()) {
            0
        } else {
            sheet_sources
                .get(&id.sheet_id())
                .and_then(|sheet| sheet.styles.get(&id.address()))
                .copied()
                .unwrap_or(0)
        };
        style_requests.insert(
            *id,
            StyleRequest {
                base_index,
                format: cell.number_format().clone(),
            },
        );
    }
    let existing_style_bytes = styles_part
        .as_ref()
        .map(|part| source.read_part(part))
        .transpose()?;
    let style_plan = plan_document_styles(
        existing_style_bytes.as_deref(),
        &target_styles_part,
        &style_requests,
        limits,
    )?;
    let style_indexes = style_plan
        .as_ref()
        .map_or_else(BTreeMap::new, |plan| plan.indexes.clone());
    if let Some(plan) = style_plan {
        if styles_part.is_some() {
            replacements.insert(target_styles_part.clone(), plan.bytes);
        } else {
            additions.insert(target_styles_part.clone(), plan.bytes);
            new_content_types.insert(target_styles_part.clone(), CONTENT_STYLES);
            new_relationships.push(NewRelationship {
                id: relationship_ids.allocate("rIdCellRuneStyles"),
                kind: REL_STYLES,
                target: "styles.xml".to_owned(),
            });
        }
    }

    for (sheet_id, sheet_source) in sheet_sources {
        let draft_sheet = draft
            .workbook()
            .sheet_by_id(sheet_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let mut semantic_edits = BTreeMap::new();
        let phonetic_targets = draft
            .presentation_cell_mutations()
            .iter()
            .filter(|id| id.sheet_id() == sheet_id)
            .map(|id| id.address())
            .collect::<BTreeSet<_>>();
        ensure_phonetic_edit_preservation(
            &sheet_source.bytes,
            &sheet_source.part,
            &phonetic_targets,
            shared_strings_bytes
                .as_deref()
                .zip(shared_strings_source.as_ref()),
            limits,
        )?;
        let edit_addresses = draft
            .cell_mutations()
            .keys()
            .chain(draft.presentation_cell_mutations().iter())
            .filter(|id| id.sheet_id() == sheet_id)
            .map(|id| id.address())
            .collect::<BTreeSet<_>>();
        for address in edit_addresses {
            let id = CalculationCellId::new(sheet_id, address);
            let mutation = draft.cell_mutations().get(&id);
            let edit = match mutation {
                Some(DraftCellMutation::Remove) => WorksheetSemanticEdit::Remove,
                Some(DraftCellMutation::Upsert { .. }) | None => {
                    let cell = draft_sheet
                        .cell(address)
                        .cloned()
                        .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
                    let original_cell = document
                        .workbook()
                        .sheet_by_id(sheet_id)
                        .and_then(|sheet| sheet.cell(address));
                    let content_changed = original_cell.map(crate::Cell::content)
                        != Some(cell.content())
                        || draft.presentation_cell_mutations().contains(&id);
                    let style_index = style_indexes
                        .get(&id)
                        .copied()
                        .or_else(|| sheet_source.styles.get(&address).copied())
                        .unwrap_or(0);
                    WorksheetSemanticEdit::Upsert {
                        cell,
                        style_index,
                        content_changed,
                        presentation: draft
                            .presentation()
                            .cell_presentation(sheet_id, address)
                            .cloned(),
                    }
                }
            };
            semantic_edits.insert(address, edit);
        }
        let mut bytes = if semantic_edits.is_empty() {
            sheet_source.bytes
        } else {
            patch_worksheet_semantics(
                &sheet_source.bytes,
                &sheet_source.part,
                &semantic_edits,
                draft_sheet.used_range(),
                limits,
            )?
        };
        let cache_updates = sheet_cache_updates(sheet_id, &materialization)?;
        if !cache_updates.is_empty() {
            bytes = patch_worksheet(&bytes, &sheet_source.part, &cache_updates, limits)?;
        }
        if draft.presentation_sheet_mutations().contains(&sheet_id) {
            if document.presentation().right_to_left(sheet_id) {
                return Err(
                    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                        .with_detail(DETAIL_RTL_FROZEN_PANE),
                );
            }
            bytes = patch_frozen_pane(
                &bytes,
                &sheet_source.part,
                draft.presentation().frozen_pane(sheet_id),
                limits,
            )?;
        }
        replacements.insert(sheet_source.part, bytes);
    }

    for (sheet_id, part) in &added_parts {
        let sheet = draft
            .workbook()
            .sheet_by_id(*sheet_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let sheet_materialization = materialization
            .cells()
            .iter()
            .filter(|(id, _)| id.sheet_id() == *sheet_id)
            .map(|(id, planned)| (id.address(), planned))
            .collect::<BTreeMap<_, _>>();
        let sheet_styles = sheet
            .cells()
            .map(|cell| {
                let id = CalculationCellId::new(*sheet_id, cell.address());
                (cell.address(), style_indexes.get(&id).copied().unwrap_or(0))
            })
            .collect::<BTreeMap<_, _>>();
        additions.insert(
            part.clone(),
            generated_worksheet_xml(
                sheet,
                &sheet_styles,
                &sheet_materialization,
                draft.presentation(),
            )?
            .into_bytes(),
        );
    }

    let request_host_recalculation =
        !materialization.is_complete() || !draft.workbook().diagnostics().is_empty();
    let workbook_bytes = source.read_part(workbook_part)?;
    replacements.insert(
        workbook_part.clone(),
        patch_workbook_semantics(
            &workbook_bytes,
            workbook_part,
            document.workbook(),
            draft.workbook(),
            &added_relationship_ids,
            WorkbookPatchOptions {
                request_host_recalculation,
                ensure_book_view: draft
                    .workbook()
                    .sheets()
                    .iter()
                    .any(|sheet| draft.presentation().frozen_pane(sheet.id()).is_some()),
            },
            limits,
        )?,
    );

    let chain = remove_calculation_chain_relationship(
        &relationship_bytes,
        &relationship_part,
        workbook_part,
        limits,
    )?;
    let removals = chain.removed_parts;
    let relationship_without_chain = chain.relationship_bytes.unwrap_or(relationship_bytes);
    replacements.insert(
        relationship_part.clone(),
        append_relationships(
            &relationship_without_chain,
            &relationship_part,
            &new_relationships,
            limits,
        )?,
    );

    if !removals.is_empty() || !new_content_types.is_empty() {
        let content_types_part = PartPath::from_archive_name(CONTENT_TYPES_PART)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        let content_types = source.read_part(&content_types_part)?;
        let without_chain = if removals.is_empty() {
            content_types
        } else {
            remove_content_type_overrides(&content_types, &content_types_part, &removals, limits)?
        };
        replacements.insert(
            content_types_part.clone(),
            append_content_type_overrides(
                &without_chain,
                &content_types_part,
                &new_content_types,
                limits,
            )?,
        );
    }

    let changed_parts = replacements
        .keys()
        .chain(additions.keys())
        .map(PartPath::source_id)
        .collect::<Vec<_>>();
    let removed_parts = removals.iter().map(PartPath::source_id).collect::<Vec<_>>();
    let plan = PackageWritePlan::modified_with_additions(
        source,
        replacements,
        additions,
        &removals,
        limits,
    )?;
    let bytes = plan.write_to_vec(source)?;
    verify_draft_output(
        draft.workbook(),
        draft.presentation(),
        &bytes,
        &materialization,
        document.kind(),
        limits,
    )?;
    let report = WriteReport::new(
        options.policy(),
        materialization.materialized_count(),
        materialization.invalidated_cells().to_vec(),
        changed_parts,
        removed_parts,
        Vec::new(),
        WriteProvenance::new(
            Some(document.input_hash()),
            draft.semantic_revision(),
            draft.presentation_revision(),
            calculation.provenance().provider().clone(),
            calculation.options(),
        ),
    );
    Ok(RecalculatedWorkbook::new(bytes, report, document.kind()))
}

fn validate_dynamic_formula_edits(
    draft: &WorkbookDraft,
    document: &crate::XlsxDocument,
) -> Result<(), XlsxWriteError> {
    for (id, mutation) in draft.cell_mutations().iter() {
        if !matches!(mutation, DraftCellMutation::Upsert { .. }) {
            continue;
        }
        let current = draft
            .workbook()
            .sheet_by_id(id.sheet_id())
            .and_then(|sheet| sheet.cell(id.address()));
        let Some(current) = current else {
            continue;
        };
        let current_formula = match current.content() {
            crate::CellContent::Formula(formula)
                if matches!(
                    formula.metadata(),
                    crate::FormulaMetadata::DynamicArray { .. }
                ) =>
            {
                Some(formula)
            }
            _ => None,
        };
        let source_content = document
            .workbook()
            .sheet_by_id(id.sheet_id())
            .and_then(|sheet| sheet.cell(id.address()))
            .map(crate::Cell::content);
        let preserves_dynamic_metadata = current_formula.is_none_or(|current| {
            matches!(
                source_content,
                Some(crate::CellContent::Formula(source))
                    if source.metadata() == current.metadata()
            )
        });
        if !preserves_dynamic_metadata {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                    .with_detail(DETAIL_DYNAMIC_FORMULA_METADATA),
            );
        }
    }
    Ok(())
}

fn sheet_cache_updates(
    sheet_id: SheetId,
    materialization: &MaterializationPlan,
) -> Result<BTreeMap<crate::CellAddress, WorksheetCellUpdate>, XlsxWriteError> {
    let mut updates = BTreeMap::new();
    for (id, planned) in materialization
        .cells()
        .iter()
        .filter(|(id, _)| id.sheet_id() == sheet_id)
    {
        let action = match &planned.action {
            MaterializationAction::Set(value) => WorksheetCacheAction::Set(value.clone()),
            MaterializationAction::Invalidate => WorksheetCacheAction::Invalidate,
        };
        let requires_formula = match planned.origin {
            MaterializedResultOrigin::DirectFormula => true,
            MaterializedResultOrigin::LegacyArray { anchor, .. }
            | MaterializedResultOrigin::DynamicSpill { anchor, .. } => *id == anchor,
        };
        if updates
            .insert(
                id.address(),
                WorksheetCellUpdate {
                    action,
                    requires_formula,
                },
            )
            .is_some()
        {
            return Err(
                XlsxWriteError::new(XlsxWriteErrorCode::ConflictingPartOperation)
                    .with_detail(DETAIL_MISSING_SHEET_PART),
            );
        }
    }
    Ok(updates)
}

fn enforce_count(name: &'static str, actual: usize, maximum: u64) -> Result<(), XlsxWriteError> {
    if actual as u64 > maximum {
        Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .with_detail(format!("{name}: {actual} > {maximum}")),
        )
    } else {
        Ok(())
    }
}

fn invalid_plan(detail: &'static str) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan).with_detail(detail)
}

fn invalid_plan_with_cause(
    source: &PartPath,
    cause: impl std::error::Error + Send + Sync + 'static,
) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
        .at_source(source.source_id())
        .with_cause(cause)
}
