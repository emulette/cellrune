use std::collections::{BTreeMap, BTreeSet};

use super::auto_filter_patch::patch_sheet_auto_filter;
use super::canonical::{ExpectedDraftOutput, generated_worksheet_xml, verify_draft_output};
use super::materialization::{MaterializationAction, MaterializationPlan};
use super::package::{PackageReader, PackageWritePlan};
use super::package_additions::{
    NewRelationship, RelationshipIdAllocator, append_content_type_overrides, append_relationships,
};
use super::package_metadata_patch::{
    remove_calculation_chain_relationship, remove_content_type_overrides,
};
use super::phonetic_preservation::ensure_phonetic_edit_preservation;
use super::serialization::validate_phonetic_limits;
use super::sheet_reference_patch::rewrite_renamed_sheet_references;
use super::styles_patch::{StyleRequest, plan_document_styles};
use super::table_patch::patch_table_xml;
use super::workbook_edit::{WorkbookPatchOptions, patch_workbook_semantics};
use super::workbook_patch::HostRecalculation;
use super::worksheet_edit::{
    WorksheetSemanticEdit, patch_worksheet_semantics, read_cell_style_indices,
    read_date_typed_cells,
};
use super::worksheet_patch::{WorksheetCacheAction, WorksheetCellUpdate, patch_worksheet};
use super::worksheet_view_edit::patch_frozen_pane;
use super::{
    RecalculatedWorkbook, RecalculationWriteOptions, WriteLimits, WriteProvenance, WriteReport,
    XlsxWriteError, XlsxWriteErrorCode,
};
use crate::draft::DraftCellMutation;
use crate::xlsx::package::PartPath;
use crate::{
    CalculationCellId, CalculationSnapshot, MaterializedResultOrigin, Sheet, SheetId, SourceId,
    WorkbookDraft, XlsxDocument,
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
const DETAIL_MISSING_AUTO_FILTER: &str = "draft sheet auto-filter was not found";
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
    let auto_filter_sheets = changed_auto_filter_sheets(draft, document);
    if !draft.workbook_changed()
        && draft.cell_mutations().is_empty()
        && draft.presentation_cell_mutations().is_empty()
        && draft.presentation_sheet_mutations().is_empty()
        && draft.changed_table_ids().is_empty()
        && auto_filter_sheets.is_empty()
    {
        return super::recalculation::write_recalculated_xlsx_bytes(document, calculation, options);
    }
    let limits = options.write_options().limits();
    validate_draft_edits(draft, limits)?;
    let materialization = MaterializationPlan::new(calculation, options.policy(), limits)?;
    let host_recalculation = HostRecalculation::requested(
        !materialization.is_complete() || !draft.workbook().diagnostics().is_empty(),
    );
    let output = write_document_package(
        draft,
        document,
        &DocumentResultPlan {
            materialization: &materialization,
            host_recalculation: Some(host_recalculation),
            keep_calculation_chain: false,
        },
        limits,
    )?;
    let report = WriteReport::new(
        options.policy(),
        materialization.materialized_count(),
        materialization.invalidated_cells().to_vec(),
        super::report::VerifiedOutputReceipt::new(
            output.changed_parts,
            output.removed_parts,
            Vec::new(),
            &output.bytes,
        ),
        WriteProvenance::new(
            Some(document.input_hash()),
            draft.semantic_revision(),
            draft.presentation_revision(),
            calculation.provenance().provider().clone(),
            calculation.options(),
        ),
    );
    Ok(RecalculatedWorkbook::new(
        output.bytes,
        report,
        document.kind(),
    ))
}

/// How a document draft write treats formula caches, `calcPr`, and the calculation chain.
pub(super) struct DocumentResultPlan<'a> {
    pub(super) materialization: &'a MaterializationPlan,
    /// The recalculation request for `calcPr`; `None` leaves `calcPr` and an otherwise unchanged
    /// workbook part as they are.
    pub(super) host_recalculation: Option<HostRecalculation>,
    /// Keeps `calcChain.xml`, valid only while every formula keeps its cell and array kind.
    pub(super) keep_calculation_chain: bool,
}

/// A verified document draft package and the parts the write changed or removed.
pub(super) struct DocumentPackageOutput {
    pub(super) bytes: Vec<u8>,
    pub(super) changed_parts: Vec<SourceId>,
    pub(super) removed_parts: Vec<SourceId>,
}

pub(super) fn validate_draft_edits(
    draft: &WorkbookDraft,
    limits: WriteLimits,
) -> Result<(), XlsxWriteError> {
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
    )
}

/// Patches the edited semantics, the planned formula caches, and the package metadata of a
/// document draft into its preserved package, leaving every other part byte-for-byte intact.
pub(super) fn write_document_package(
    draft: &WorkbookDraft,
    document: &XlsxDocument,
    plan: &DocumentResultPlan<'_>,
    limits: WriteLimits,
) -> Result<DocumentPackageOutput, XlsxWriteError> {
    let materialization = plan.materialization;
    let auto_filter_sheets = changed_auto_filter_sheets(draft, document);
    let source = document.preserved_package();
    let mut source_reader = source.reader()?;
    let date_typed_literals =
        date_typed_literals_to_rewrite(draft, document, &mut source_reader, limits)?;
    let shared_strings_source = document
        .package_summary()
        .shared_strings_part()
        .map(|source_id| PartPath::from_archive_name(source_id.as_str().as_bytes()))
        .transpose()
        .map_err(|error| invalid_plan_with_cause(document.workbook_part_path(), error))?;
    let shared_strings_bytes = shared_strings_source
        .as_ref()
        .map(|part| source_reader.read_part(part))
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
        let bytes = source_reader.read_part(&part)?;
        replacements.insert(part.clone(), patch_table_xml(&bytes, &part, table, limits)?);
    }

    let workbook_part = document.workbook_part_path();
    let relationship_part = workbook_part
        .relationship_part()
        .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
    let relationship_bytes = source_reader.read_part(&relationship_part)?;
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
        .chain(auto_filter_sheets.iter().copied())
        .chain(date_typed_literals.keys().copied())
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
        let bytes = source_reader.read_part(&part)?;
        // Only a valued source cell's own style is read. A cell written over a removed source
        // cell is styled as a new cell, and so is one written over a blank source cell: the
        // reader records a blank cell's style for new cells unless it names no `cellXfs` entry.
        let source_sheet = document.workbook().sheet_by_id(sheet_id);
        let targets = draft
            .cell_mutations()
            .sheet_keys(sheet_id)
            .chain(
                draft
                    .presentation_cell_mutations()
                    .range(sheet_cell_bounds(sheet_id)),
            )
            .copied()
            .chain(
                date_typed_literals
                    .get(&sheet_id)
                    .into_iter()
                    .flatten()
                    .map(|address| CalculationCellId::new(sheet_id, *address)),
            )
            .filter(|id| {
                !matches!(
                    draft.cell_mutations().get(id),
                    Some(DraftCellMutation::Upsert {
                        replaces_source_cell: true,
                        ..
                    })
                )
            })
            .map(|id| id.address())
            .filter(|address| source_sheet.is_some_and(|sheet| sheet.cell(*address).is_some()))
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
            ..
        } = mutation
        else {
            continue;
        };
        let sheet = draft
            .workbook()
            .sheet_by_id(id.sheet_id())
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let cell = sheet
            .cell(id.address())
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let base_index = if draft.added_sheets().contains(&id.sheet_id()) {
            0
        } else {
            sheet_sources.get(&id.sheet_id()).map_or(0, |source| {
                source_style_index(&source.styles, sheet, id.address())
            })
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
        .map(|part| source_reader.read_part(part))
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
            .range(sheet_cell_bounds(sheet_id))
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
        let date_typed = date_typed_literals
            .get(&sheet_id)
            .cloned()
            .unwrap_or_default();
        let edit_addresses = draft
            .cell_mutations()
            .sheet_keys(sheet_id)
            .chain(
                draft
                    .presentation_cell_mutations()
                    .range(sheet_cell_bounds(sheet_id)),
            )
            .map(|id| id.address())
            .chain(date_typed.iter().copied())
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
                        || draft.presentation_cell_mutations().contains(&id)
                        || date_typed.contains(&address);
                    let style_index = style_indexes.get(&id).copied().unwrap_or_else(|| {
                        source_style_index(&sheet_source.styles, draft_sheet, address)
                    });
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
        let cache_updates = sheet_cache_updates(sheet_id, materialization)?;
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
        if auto_filter_sheets.contains(&sheet_id) {
            let filter = draft_sheet
                .auto_filter()
                .ok_or_else(|| invalid_plan(DETAIL_MISSING_AUTO_FILTER))?;
            bytes = patch_sheet_auto_filter(&bytes, &sheet_source.part, filter, limits)?;
        }
        replacements.insert(sheet_source.part, bytes);
    }
    rewrite_renamed_sheet_references(
        document,
        draft.workbook(),
        &mut source_reader,
        &mut replacements,
        limits,
    )?;

    for (sheet_id, part) in &added_parts {
        let sheet = draft
            .workbook()
            .sheet_by_id(*sheet_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let sheet_materialization = materialization
            .cells()
            .range(sheet_cell_bounds(*sheet_id))
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

    if plan.host_recalculation.is_some()
        || draft.workbook_changed()
        || !draft.presentation_sheet_mutations().is_empty()
    {
        let workbook_bytes = source_reader.read_part(workbook_part)?;
        replacements.insert(
            workbook_part.clone(),
            patch_workbook_semantics(
                &workbook_bytes,
                workbook_part,
                document.workbook(),
                draft.workbook(),
                &added_relationship_ids,
                WorkbookPatchOptions {
                    host_recalculation: plan.host_recalculation,
                    ensure_book_view: draft
                        .workbook()
                        .sheets()
                        .iter()
                        .any(|sheet| draft.presentation().frozen_pane(sheet.id()).is_some()),
                },
                limits,
            )?,
        );
    }

    let (removals, relationship_without_chain) = if plan.keep_calculation_chain {
        (BTreeSet::new(), None)
    } else {
        let chain = remove_calculation_chain_relationship(
            &relationship_bytes,
            &relationship_part,
            workbook_part,
            limits,
        )?;
        (chain.removed_parts, chain.relationship_bytes)
    };
    if relationship_without_chain.is_some() || !new_relationships.is_empty() {
        replacements.insert(
            relationship_part.clone(),
            append_relationships(
                relationship_without_chain
                    .as_deref()
                    .unwrap_or(&relationship_bytes),
                &relationship_part,
                &new_relationships,
                limits,
            )?,
        );
    }

    if !removals.is_empty() || !new_content_types.is_empty() {
        let content_types_part = PartPath::from_archive_name(CONTENT_TYPES_PART)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        let content_types = source_reader.read_part(&content_types_part)?;
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

    drop(source_reader);
    let changed_parts = replacements
        .keys()
        .chain(additions.keys())
        .map(PartPath::source_id)
        .collect::<Vec<_>>();
    let removed_parts = removals.iter().map(PartPath::source_id).collect::<Vec<_>>();
    let package_plan = PackageWritePlan::modified_with_additions(
        source,
        replacements,
        additions,
        &removals,
        limits,
    )?;
    let bytes = package_plan.write_to_vec(source)?;
    verify_draft_output(
        &ExpectedDraftOutput {
            workbook: draft.workbook(),
            presentation: draft.presentation(),
            materialization,
            kind: document.kind(),
            host_recalculation: plan.host_recalculation.unwrap_or(HostRecalculation::Clear),
        },
        &bytes,
        document.read_options().limits(),
        limits,
    )?;
    Ok(DocumentPackageOutput {
        bytes,
        changed_parts,
        removed_parts,
    })
}

/// Returns the cells stored as ISO 8601 dates (`t="d"`) that the draft still holds as literals once
/// it changed the date system, edited or not. The draft keeps each cell's serial, which the stored
/// calendar date would read as a different serial in the new system, so these cells are rewritten
/// from the draft: as their numbers also when an edit, such as a number format change, left their
/// value.
fn date_typed_literals_to_rewrite(
    draft: &WorkbookDraft,
    document: &XlsxDocument,
    source_reader: &mut PackageReader<'_>,
    limits: WriteLimits,
) -> Result<BTreeMap<SheetId, BTreeSet<crate::CellAddress>>, XlsxWriteError> {
    let mut literals = BTreeMap::new();
    if draft.workbook().date_system() == document.workbook().date_system() {
        return Ok(literals);
    }
    for sheet in draft.workbook().sheets() {
        let Some(part) = document.worksheet_part_path(sheet.id()) else {
            continue;
        };
        let cells = read_date_typed_cells(&source_reader.read_part(part)?, part, limits)?
            .into_iter()
            .filter(|address| {
                matches!(
                    sheet.cell(*address).map(crate::Cell::content),
                    Some(crate::CellContent::Literal(_))
                )
            })
            .collect::<BTreeSet<_>>();
        if !cells.is_empty() {
            literals.insert(sheet.id(), cells);
        }
    }
    Ok(literals)
}

/// Existing sheets whose sheet-level autoFilter the draft changed.
fn changed_auto_filter_sheets(draft: &WorkbookDraft, document: &XlsxDocument) -> BTreeSet<SheetId> {
    draft
        .workbook()
        .sheets()
        .iter()
        .filter(|sheet| {
            document
                .workbook()
                .sheet_by_id(sheet.id())
                .is_some_and(|source| source.auto_filter() != sheet.auto_filter())
        })
        .map(Sheet::id)
        .collect()
}

pub(super) fn validate_dynamic_formula_edits(
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

/// Returns the style index a written cell takes: the valued source cell's own style, else the
/// style a new cell takes at its position in the source worksheet, else the default style.
fn source_style_index(
    source_styles: &BTreeMap<crate::CellAddress, usize>,
    sheet: &crate::Sheet,
    address: crate::CellAddress,
) -> usize {
    source_styles
        .get(&address)
        .copied()
        .or_else(|| {
            sheet
                .new_cell_styles()
                .style(address)
                .map(|(index, _)| index)
        })
        .unwrap_or(0)
}

fn sheet_cell_bounds(sheet_id: SheetId) -> std::ops::RangeInclusive<CalculationCellId> {
    let first = crate::CellAddress::from_indices(1, 1).expect("first worksheet address");
    let last = crate::CellAddress::from_indices(crate::EXCEL_MAX_ROWS, crate::EXCEL_MAX_COLUMNS)
        .expect("last worksheet address");
    CalculationCellId::new(sheet_id, first)..=CalculationCellId::new(sheet_id, last)
}

fn sheet_cache_updates(
    sheet_id: SheetId,
    materialization: &MaterializationPlan,
) -> Result<BTreeMap<crate::CellAddress, WorksheetCellUpdate>, XlsxWriteError> {
    let mut updates = BTreeMap::new();
    for (id, planned) in materialization.cells().range(sheet_cell_bounds(sheet_id)) {
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
