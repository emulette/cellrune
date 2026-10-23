use std::collections::{BTreeMap, BTreeSet};

use super::auto_filter_patch::rewrite_sheet_auto_filter;
use super::canonical::{ExpectedDraftOutput, generated_worksheet_xml, verify_draft_output};
use super::grid_parts::replay_grid_edits;
use super::layout_patch::{LayoutChanges, patch_sheet_layout};
use super::materialization::{MaterializationAction, MaterializationPlan};
use super::package::{PackageReader, PackageWritePlan};
use super::package_additions::{
    NewRelationship, RelationshipIdAllocator, append_content_type_overrides, append_relationships,
};
use super::package_metadata_patch::{
    remove_calculation_chain_relationship, remove_content_type_overrides, remove_relationships,
};
use super::phonetic_preservation::ensure_phonetic_edit_preservation;
use super::serialization::validate_phonetic_limits;
use super::sheet_copy_parts::{CopySource, duplicate_sheet_parts, highest_vml_block};
use super::sheet_deletion::{plan_sheet_deletion, remove_pivot_cache_entries};
use super::sheet_positions::with_explicit_positions;
use super::sheet_reference_patch::rewrite_sheet_references;
use super::sheet_reference_patch::{CopiedParts, CopyReferences};
use super::styles_patch::{StyleRequest, plan_document_styles};
use super::table::{TABLE_CONTENT_TYPE, table_xml};
use super::table_parts::{
    TABLE_RELATIONSHIP, TableAddition, add_table_parts, plan_table_removal, relationships_xml,
    remove_table_parts,
};
use super::table_patch::{patch_table_xml, remove_table_columns};
use super::workbook_edit::{WorkbookPatchOptions, patch_workbook_semantics};
use super::workbook_patch::HostRecalculation;
use super::worksheet_edit::{
    WorksheetSemanticEdit, patch_worksheet_semantics, read_cell_style_indices,
    read_date_typed_cells,
};
use super::worksheet_patch::{WorksheetCacheAction, WorksheetCellUpdate, patch_worksheet};
use super::worksheet_view_edit::{patch_frozen_pane, without_tab_selection};
use super::{
    RecalculatedWorkbook, RecalculationWriteOptions, WriteLimits, WriteProvenance, WriteReport,
    XlsxWriteError, XlsxWriteErrorCode,
};
use crate::draft::DraftCellMutation;
use crate::grid::GridHistory;
use crate::xlsx::package::{PartPath, parse_content_types};
use crate::{
    CalculationCellId, CalculationSnapshot, CellAddress, CellFormat, MaterializedResultOrigin,
    Sheet, SheetId, SourceId, WorkbookDraft, XlsxDocument,
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
const DETAIL_COPY_GRID_EDITS: &str =
    "a sheet copy cannot save structural row, column, or range edits of the copy or its source";
const DETAIL_COPY_TABLES_CHANGED: &str = "a sheet copy cannot save a table created or converted to a range on the copy or on its source before the copy";
const DETAIL_EDITED_CELL_COUNT: &str = "max_edited_cells";
const DETAIL_EDITED_SHEET_COUNT: &str = "max_edited_sheets";
const DETAIL_DYNAMIC_FORMULA_METADATA: &str =
    "document-backed dynamic formula edits require source metadata index merging";
const DETAIL_RTL_FROZEN_PANE: &str =
    "right-to-left frozen-pane authoring requires a verified native contract";

struct ExistingSheetSource {
    /// The source part read, a copy's source sheet's for a copy.
    part: PartPath,
    /// The part written: the source part, or a copy's new part.
    output: PartPath,
    /// The source sheet whose cells and presentation the part holds.
    origin: SheetId,
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
    validate_dynamic_formula_edits(draft)?;
    let auto_filter_sheets = changed_auto_filter_sheets(draft, document);
    if !draft.workbook_changed()
        && draft.grid_histories().is_empty()
        && draft.cell_mutations().is_empty()
        && draft.presentation_cell_mutations().is_empty()
        && draft.presentation_sheet_mutations().is_empty()
        && draft.changed_table_ids().is_empty()
        && auto_filter_sheets.is_empty()
        && format_edits(draft).is_empty()
        && layout_sheets(draft, document.presentation()).is_empty()
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
    let formatted = format_edits(draft)
        .into_iter()
        .flat_map(|(sheet_id, edits)| {
            edits
                .into_keys()
                .map(move |address| CalculationCellId::new(sheet_id, address))
        })
        .collect::<BTreeSet<_>>();
    enforce_count(
        DETAIL_EDITED_CELL_COUNT,
        draft
            .cell_mutations()
            .keys()
            .chain(draft.presentation_cell_mutations().iter())
            .chain(formatted.iter())
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        limits.max_edited_cells(),
    )
}

/// Returns the edited cell formats of every sheet the draft holds, by sheet.
fn format_edits(draft: &WorkbookDraft) -> BTreeMap<SheetId, BTreeMap<CellAddress, CellFormat>> {
    draft
        .workbook()
        .sheets()
        .iter()
        .filter_map(|sheet| {
            let edits = draft
                .presentation()
                .edited_cell_formats(sheet.id())
                .map(|(address, format)| (address, format.clone()))
                .collect::<BTreeMap<_, _>>();
            (!edits.is_empty()).then_some((sheet.id(), edits))
        })
        .collect()
}

/// Returns the column width and row height changes of each source sheet whose layout the draft
/// changed, from `source`, the source presentation moved by the draft's structural edits.
fn layout_sheets(
    draft: &WorkbookDraft,
    source: &crate::DocumentPresentation,
) -> BTreeMap<SheetId, LayoutChanges> {
    draft
        .workbook()
        .sheets()
        .iter()
        .filter(|sheet| !draft.added_sheets().contains(&sheet.id()))
        .filter_map(|sheet| {
            let source_id = source_sheet_id(draft, sheet.id());
            let changes =
                LayoutChanges::between(source, source_id, draft.presentation(), sheet.id());
            (!changes.is_empty()).then_some((sheet.id(), changes))
        })
        .collect()
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
    let format_edits = format_edits(draft);
    let source_presentation = moved_source_presentation(draft, document);
    let layout_changes = layout_sheets(draft, &source_presentation);
    let source = document.preserved_package();
    let mut source_reader = source.reader()?;
    let deletion = plan_sheet_deletion(document, draft.workbook(), &mut source_reader)?;
    let table_removal = plan_table_removal(document, draft.workbook(), &mut source_reader)?;
    // A copy duplicates its source's part as the source package holds it, so the structural edits
    // of the copy or its source cannot be replayed on it.
    if let Some(copy) = draft.copied_sheets().iter().find_map(|(copy, origin)| {
        let histories = draft.grid_histories();
        (histories.contains_key(copy) || histories.contains_key(origin)).then_some(*copy)
    }) {
        let name = draft
            .workbook()
            .sheet_by_id(copy)
            .map_or("", |sheet| sheet.name().as_str());
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
                .with_detail(format!("{DETAIL_COPY_GRID_EDITS}: {name}")),
        );
    }
    let mut replacements = BTreeMap::<PartPath, Vec<u8>>::new();
    replay_grid_edits(
        draft,
        document,
        &mut source_reader,
        &mut replacements,
        &deletion.removed_parts,
        limits,
    )?;
    let date_typed_literals =
        date_typed_literals_to_rewrite(draft, document, &mut source_reader, &replacements, limits)?;
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
    let mut additions = BTreeMap::<PartPath, Vec<u8>>::new();
    let mut new_content_types = BTreeMap::<PartPath, std::borrow::Cow<'static, str>>::new();
    let mut new_relationships = Vec::<NewRelationship>::new();

    for table_id in draft.changed_table_ids() {
        // A table the draft created has no source part; it is added with its sheet below.
        if document.workbook().table_by_id(*table_id).is_none() {
            continue;
        }
        let part = document
            .table_part_path(*table_id)
            .cloned()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_TABLE_PART))?;
        let table = draft
            .workbook()
            .table_by_id(*table_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_TABLE_PART))?;
        let mut bytes = source_reader.read_part(&part)?;
        // A range change can take columns out of the table.
        let removed_columns = document
            .workbook()
            .table_by_id(*table_id)
            .into_iter()
            .flat_map(|source| source.columns())
            .map(crate::TableColumn::id)
            .filter(|id| !table.columns().iter().any(|column| column.id() == *id))
            .collect::<BTreeSet<_>>();
        if !removed_columns.is_empty() {
            bytes = remove_table_columns(&bytes, &part, &removed_columns, limits)?;
        }
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
    // A copy of a source sheet is a new worksheet part too, duplicated from its source's.
    let mut copy_parts = BTreeMap::<SheetId, PartPath>::new();
    for sheet_id in draft
        .added_sheets()
        .iter()
        .chain(draft.copied_sheets().keys())
    {
        let relationship_id =
            relationship_ids.allocate(&format!("rIdCellRuneSheet{}", sheet_id.get()));
        let target = format!("worksheets/cellrune-sheet-{}.xml", sheet_id.get());
        let part = PartPath::resolve_relationship(Some(workbook_part), &target)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        added_relationship_ids.insert(*sheet_id, relationship_id.clone());
        if draft.copied_sheets().contains_key(sheet_id) {
            copy_parts.insert(*sheet_id, part.clone());
        } else {
            added_parts.insert(*sheet_id, part.clone());
        }
        new_relationships.push(NewRelationship {
            id: relationship_id,
            kind: REL_WORKSHEET,
            target,
        });
        new_content_types.insert(part, CONTENT_WORKSHEET.into());
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
        .chain(format_edits.keys().copied())
        .chain(layout_changes.keys().copied())
        .chain(copy_parts.keys().copied())
        .chain(
            draft
                .grid_histories()
                .keys()
                .copied()
                .filter(|sheet_id| draft.workbook().sheet_by_id(*sheet_id).is_some()),
        )
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
        let origin = source_sheet_id(draft, sheet_id);
        let part = document
            .worksheet_part_path(origin)
            .cloned()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        // A copy is refused when its source has structural edits, so either part's replacement
        // holds only references other sheets' edits moved.
        let bytes = match replacements.get(&part) {
            Some(bytes) => bytes.clone(),
            None => source_reader.read_part(&part)?,
        };
        let bytes = with_explicit_positions(&bytes, &part, limits)?.unwrap_or(bytes);
        // A copy writes every cell over its source's part, so every source style is read, and it
        // does not take its source's tab selection.
        let copied = copy_parts.contains_key(&sheet_id);
        let bytes = if copied {
            without_tab_selection(&bytes, &part, limits)?.unwrap_or(bytes)
        } else {
            bytes
        };
        // Only a valued source cell's own style is read. A cell written over a removed source
        // cell is styled as a new cell, and so is one written over a blank source cell: the
        // reader records a blank cell's style for new cells unless it names no `cellXfs` entry.
        let source_sheet = document.workbook().sheet_by_id(origin);
        let source_cells = draft.source_cells();
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
            .chain(
                format_edits
                    .get(&sheet_id)
                    .into_iter()
                    .flat_map(BTreeMap::keys)
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
            .chain(
                source_sheet
                    .filter(|_| copied)
                    .into_iter()
                    .flat_map(|sheet| sheet.cells().map(crate::Cell::address)),
            )
            .filter(|address| source_cells.cell(origin, *address).is_some())
            .collect::<BTreeSet<_>>();
        let styles = read_cell_style_indices(&bytes, &part, &targets, limits)?;
        sheet_sources.insert(
            sheet_id,
            ExistingSheetSource {
                output: copy_parts
                    .get(&sheet_id)
                    .cloned()
                    .unwrap_or_else(|| part.clone()),
                origin,
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
                presentation: None,
            },
        );
    }
    for (sheet_id, edits) in &format_edits {
        let sheet = draft
            .workbook()
            .sheet_by_id(*sheet_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let added = draft.added_sheets().contains(sheet_id);
        for (address, format) in edits {
            let id = CalculationCellId::new(*sheet_id, *address);
            if let Some(request) = style_requests.get_mut(&id) {
                request.presentation = Some(format.clone());
                continue;
            }
            let (base_index, number_format) = if added {
                (
                    0,
                    sheet
                        .cell(*address)
                        .map(|cell| cell.number_format().clone())
                        .unwrap_or_default(),
                )
            } else {
                let base_index = sheet_sources.get(sheet_id).map_or(0, |source| {
                    source_style_index(&source.styles, sheet, *address)
                });
                let number_format = sheet.cell(*address).map_or_else(
                    || sheet.new_cell_number_format(*address),
                    |cell| cell.number_format().clone(),
                );
                (base_index, number_format)
            };
            style_requests.insert(
                id,
                StyleRequest {
                    base_index,
                    format: number_format,
                    presentation: Some(format.clone()),
                },
            );
        }
    }
    let existing_style_bytes = styles_part
        .as_ref()
        .map(|part| source_reader.read_part(part))
        .transpose()?;
    let style_plan = plan_document_styles(
        existing_style_bytes.as_deref(),
        &target_styles_part,
        &style_requests,
        draft.presentation(),
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
            new_content_types.insert(target_styles_part.clone(), CONTENT_STYLES.into());
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
        let copied = copy_parts.contains_key(&sheet_id);
        let origin_sheet = document.workbook().sheet_by_id(sheet_source.origin);
        // A copy's part starts as its source's, so each cell of either is an edit.
        let copy_addresses = if copied {
            origin_sheet
                .into_iter()
                .flat_map(|sheet| sheet.cells().map(crate::Cell::address))
                .chain(draft_sheet.cells().map(crate::Cell::address))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
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
            .chain(
                format_edits
                    .get(&sheet_id)
                    .into_iter()
                    .flat_map(BTreeMap::keys)
                    .copied(),
            )
            .chain(copy_addresses)
            .collect::<BTreeSet<_>>();
        let sheet_format_edits = format_edits.get(&sheet_id);
        for address in edit_addresses {
            let id = CalculationCellId::new(sheet_id, address);
            let mutation = draft.cell_mutations().get(&id);
            let formatted = sheet_format_edits.is_some_and(|edits| edits.contains_key(&address));
            // A formatted position that holds no value keeps its format as an empty styled cell,
            // as Excel keeps the format of a cell whose content is cleared.
            if formatted && draft_sheet.cell(address).is_none() {
                semantic_edits.insert(
                    address,
                    WorksheetSemanticEdit::Blank {
                        style_index: style_indexes.get(&id).copied().unwrap_or(0),
                    },
                );
                continue;
            }
            let edit = match mutation {
                Some(DraftCellMutation::Remove) => WorksheetSemanticEdit::Remove,
                None if copied && draft_sheet.cell(address).is_none() => {
                    WorksheetSemanticEdit::Remove
                }
                Some(DraftCellMutation::Upsert { .. }) | None => {
                    let cell = draft_sheet
                        .cell(address)
                        .cloned()
                        .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
                    let original_cell = draft.source_cells().cell(sheet_source.origin, address);
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
        let copied_pane = copied
            && draft.presentation().frozen_pane(sheet_id)
                != document.presentation().frozen_pane(sheet_source.origin);
        if draft.presentation_sheet_mutations().contains(&sheet_id) || copied_pane {
            if document.presentation().right_to_left(sheet_source.origin) {
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
            let original =
                origin_sheet.and_then(|origin| replayed_auto_filter(draft, origin, sheet_id));
            bytes = rewrite_sheet_auto_filter(
                &bytes,
                &sheet_source.part,
                original.as_ref(),
                draft_sheet.auto_filter(),
                limits,
            )?;
        }
        if let Some(changes) = layout_changes.get(&sheet_id) {
            bytes = patch_sheet_layout(&bytes, &sheet_source.part, changes, limits)?;
        }
        if copied {
            additions.insert(sheet_source.output, bytes);
        } else {
            replacements.insert(sheet_source.part, bytes);
        }
    }

    // A copy duplicates every part its source sheet reaches, renaming and repointing them.
    let mut copy_references = Vec::new();
    if !copy_parts.is_empty() {
        let content_types_part = PartPath::from_archive_name(CONTENT_TYPES_PART)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        let content_types = parse_content_types(
            &source_reader.read_part(&content_types_part)?,
            &content_types_part,
            document.read_options().limits(),
        )
        .map_err(|error| invalid_plan_with_cause(&content_types_part, error))?;
        let vml = document
            .link_part_paths()
            .iter()
            .filter(|part| part.extension() == Some("vml"))
            .map(|part| source_reader.read_part(part))
            .collect::<Result<Vec<_>, _>>()?;
        let mut next_vml_block = highest_vml_block(&vml) + 1;
        for (copy_id, copy_part) in &copy_parts {
            let origin = source_sheet_id(draft, *copy_id);
            let origin_part = document
                .worksheet_part_path(origin)
                .cloned()
                .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
            let copy_sheet = draft
                .workbook()
                .sheet_by_id(*copy_id)
                .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
            let origin_tables = document
                .workbook()
                .sheet_by_id(origin)
                .map(Sheet::tables)
                .unwrap_or_default();
            // Each source table part is duplicated for the copied table that duplicates its
            // table. A table created or converted on the copy, or on its source before the copy,
            // has no such pair, and its part or entry is not written.
            let mut tables = BTreeMap::new();
            for copied in copy_sheet.tables() {
                let source = draft
                    .copied_tables()
                    .get(&copied.id())
                    .and_then(|source| origin_tables.iter().find(|table| table.id() == *source))
                    .ok_or_else(|| copy_tables_changed(copy_part))?;
                let part = document
                    .table_part_path(source.id())
                    .cloned()
                    .ok_or_else(|| invalid_plan(DETAIL_MISSING_TABLE_PART))?;
                tables.insert(part, (source, copied));
            }
            if tables.len() != origin_tables.len() {
                return Err(copy_tables_changed(copy_part));
            }
            let duplicated = duplicate_sheet_parts(
                &mut source_reader,
                &CopySource {
                    origin: &origin_part,
                    copy: copy_part,
                    copy_id: *copy_id,
                    tables: &tables,
                    content_types: &content_types,
                    chart_parts: document.chart_part_paths(),
                },
                &mut next_vml_block,
                limits,
            )?;
            additions.extend(duplicated.parts);
            new_content_types.extend(duplicated.content_types);
            copy_references.push(CopyReferences {
                copy: *copy_id,
                origin,
                worksheet: copy_part.clone(),
                charts: duplicated.charts,
            });
        }
    }
    rewrite_sheet_references(
        document,
        draft,
        &mut source_reader,
        &mut replacements,
        CopiedParts {
            copies: &copy_references,
            parts: &mut additions,
        },
        &deletion.removed_parts,
        limits,
    )?;
    // A table converted to a range leaves its sheet's `<tableParts>` and relationships.
    for (worksheet, (relationship_part, ids)) in &table_removal.worksheets {
        let bytes = match replacements.get(worksheet) {
            Some(bytes) => remove_table_parts(bytes, worksheet, ids, limits)?,
            None => {
                remove_table_parts(&source_reader.read_part(worksheet)?, worksheet, ids, limits)?
            }
        };
        replacements.insert(worksheet.clone(), bytes);
        let relationships = remove_relationships(
            &source_reader.read_part(relationship_part)?,
            relationship_part,
            ids,
            limits,
        )?;
        replacements.insert(relationship_part.clone(), relationships);
    }

    // Each table the draft created on a source sheet: a new part, a relationship, and an entry.
    for sheet in draft.workbook().sheets().iter().filter(|sheet| {
        !draft.added_sheets().contains(&sheet.id()) && !copy_parts.contains_key(&sheet.id())
    }) {
        let created = sheet
            .tables()
            .iter()
            .filter(|table| document.workbook().table_by_id(table.id()).is_none())
            .collect::<Vec<_>>();
        if created.is_empty() {
            continue;
        }
        let worksheet = document
            .worksheet_part_path(sheet.id())
            .cloned()
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        let sheet_relationships = worksheet
            .relationship_part()
            .map_err(|error| invalid_plan_with_cause(&worksheet, error))?;
        let current = match replacements.get(&sheet_relationships) {
            Some(bytes) => Some(bytes.clone()),
            None if source_reader.contains(&sheet_relationships) => {
                Some(source_reader.read_part(&sheet_relationships)?)
            }
            None => None,
        };
        let mut allocator = RelationshipIdAllocator::from_xml(
            current
                .as_deref()
                .unwrap_or(relationships_xml(&[]).as_bytes()),
            &sheet_relationships,
            limits,
        )?;
        let mut table_additions = Vec::with_capacity(created.len());
        for table in created {
            let relationship_id =
                allocator.allocate(&format!("rIdCellRuneTable{}", table.id().get()));
            let addition = TableAddition::new(&worksheet, table.id().get(), relationship_id)?;
            additions.insert(addition.part.clone(), table_xml(table)?.into_bytes());
            new_content_types.insert(addition.part.clone(), TABLE_CONTENT_TYPE.into());
            table_additions.push(addition);
        }
        match current {
            Some(bytes) => {
                let appended = table_additions
                    .iter()
                    .map(|addition| NewRelationship {
                        id: addition.relationship_id.clone(),
                        kind: TABLE_RELATIONSHIP,
                        target: addition.target.clone(),
                    })
                    .collect::<Vec<_>>();
                replacements.insert(
                    sheet_relationships.clone(),
                    append_relationships(&bytes, &sheet_relationships, &appended, limits)?,
                );
            }
            None => {
                additions.insert(
                    sheet_relationships,
                    relationships_xml(&table_additions).into_bytes(),
                );
            }
        }
        let ids = table_additions
            .iter()
            .map(|addition| addition.relationship_id.clone())
            .collect::<Vec<_>>();
        let base = match replacements.get(&worksheet) {
            Some(bytes) => bytes.clone(),
            None => source_reader.read_part(&worksheet)?,
        };
        replacements.insert(
            worksheet.clone(),
            add_table_parts(&base, &worksheet, &ids, limits)?,
        );
    }

    for (sheet_id, part) in &added_parts {
        let sheet = draft
            .workbook()
            .sheet_by_id(*sheet_id)
            .ok_or_else(|| invalid_plan(DETAIL_MISSING_SHEET_PART))?;
        // An added sheet's worksheet names its tables `rIdCellRuneTable` and the table ID.
        if !sheet.tables().is_empty() {
            let mut table_additions = Vec::with_capacity(sheet.tables().len());
            for table in sheet.tables() {
                let addition = TableAddition::new(
                    part,
                    table.id().get(),
                    super::table::table_relationship_id(table),
                )?;
                additions.insert(addition.part.clone(), table_xml(table)?.into_bytes());
                new_content_types.insert(addition.part.clone(), TABLE_CONTENT_TYPE.into());
                table_additions.push(addition);
            }
            let sheet_relationships = part
                .relationship_part()
                .map_err(|error| invalid_plan_with_cause(part, error))?;
            additions.insert(
                sheet_relationships,
                relationships_xml(&table_additions).into_bytes(),
            );
        }
        let sheet_materialization = materialization
            .cells()
            .range(sheet_cell_bounds(*sheet_id))
            .map(|(id, planned)| (id.address(), planned))
            .collect::<BTreeMap<_, _>>();
        let sheet_styles = sheet
            .cells()
            .map(|cell| cell.address())
            .chain(
                format_edits
                    .get(sheet_id)
                    .into_iter()
                    .flat_map(BTreeMap::keys)
                    .copied(),
            )
            .map(|address| {
                let id = CalculationCellId::new(*sheet_id, address);
                (address, style_indexes.get(&id).copied().unwrap_or(0))
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
        let mut patched = patch_workbook_semantics(
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
                tab_steps: draft.tab_steps(),
            },
            limits,
        )?;
        if !deletion.pivot_cache_relationship_ids.is_empty() {
            patched = remove_pivot_cache_entries(
                &patched,
                workbook_part,
                &deletion.pivot_cache_relationship_ids,
                limits,
            )?;
        }
        replacements.insert(workbook_part.clone(), patched);
    }

    let without_deleted = (!deletion.workbook_relationship_ids.is_empty())
        .then(|| {
            remove_relationships(
                &relationship_bytes,
                &relationship_part,
                &deletion.workbook_relationship_ids,
                limits,
            )
        })
        .transpose()?;
    let relationships = without_deleted.as_deref().unwrap_or(&relationship_bytes);
    let (chain_removals, relationship_without_chain) = if plan.keep_calculation_chain {
        (BTreeSet::new(), None)
    } else {
        let chain = remove_calculation_chain_relationship(
            relationships,
            &relationship_part,
            workbook_part,
            limits,
        )?;
        (chain.removed_parts, chain.relationship_bytes)
    };
    if without_deleted.is_some()
        || relationship_without_chain.is_some()
        || !new_relationships.is_empty()
    {
        replacements.insert(
            relationship_part.clone(),
            append_relationships(
                relationship_without_chain
                    .as_deref()
                    .unwrap_or(relationships),
                &relationship_part,
                &new_relationships,
                limits,
            )?,
        );
    }

    let mut optional_removals = deletion.removed_parts;
    optional_removals.extend(table_removal.parts);
    if !chain_removals.is_empty() || !optional_removals.is_empty() || !new_content_types.is_empty()
    {
        let content_types_part = PartPath::from_archive_name(CONTENT_TYPES_PART)
            .map_err(|error| invalid_plan_with_cause(workbook_part, error))?;
        let content_types = source_reader.read_part(&content_types_part)?;
        let without_removed = if chain_removals.is_empty() && optional_removals.is_empty() {
            content_types
        } else {
            remove_content_type_overrides(
                &content_types,
                &content_types_part,
                &chain_removals,
                &optional_removals,
                limits,
            )?
        };
        replacements.insert(
            content_types_part.clone(),
            append_content_type_overrides(
                &without_removed,
                &content_types_part,
                &new_content_types,
                limits,
            )?,
        );
    }
    let mut removals = chain_removals;
    removals.extend(optional_removals);

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
            source: Some(draft.source_cells()),
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
    replacements: &BTreeMap<PartPath, Vec<u8>>,
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
        let bytes = match replacements.get(part) {
            Some(bytes) => bytes.clone(),
            None => source_reader.read_part(part)?,
        };
        let bytes = with_explicit_positions(&bytes, part, limits)?.unwrap_or(bytes);
        let cells = read_date_typed_cells(&bytes, part, limits)?
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

/// Returns the source presentation with the draft's structural edits applied, which is what the
/// worksheet parts they rewrite read back as.
fn moved_source_presentation<'a>(
    draft: &WorkbookDraft,
    document: &'a XlsxDocument,
) -> std::borrow::Cow<'a, crate::DocumentPresentation> {
    if draft.grid_histories().is_empty() {
        return std::borrow::Cow::Borrowed(document.presentation());
    }
    let mut presentation = document.presentation().clone();
    for (sheet_id, history) in draft.grid_histories() {
        for edit in history.edits() {
            presentation.apply_grid_edit(*sheet_id, *edit);
        }
    }
    std::borrow::Cow::Owned(presentation)
}

/// Existing sheets, copies included, whose sheet-level autoFilter differs from the one their part
/// holds once the structural edits are replayed on it, which moved or deleted it with its range.
fn changed_auto_filter_sheets(draft: &WorkbookDraft, document: &XlsxDocument) -> BTreeSet<SheetId> {
    draft
        .workbook()
        .sheets()
        .iter()
        .filter(|sheet| {
            document
                .workbook()
                .sheet_by_id(source_sheet_id(draft, sheet.id()))
                .is_some_and(|source| {
                    replayed_auto_filter(draft, source, sheet.id()).as_ref() != sheet.auto_filter()
                })
        })
        .map(Sheet::id)
        .collect()
}

/// Returns the sheet-level autoFilter of `source` as the structural edits the draft recorded for
/// `sheet_id` moved it, which is the filter the replayed part holds.
fn replayed_auto_filter(
    draft: &WorkbookDraft,
    source: &Sheet,
    sheet_id: SheetId,
) -> Option<crate::SheetAutoFilter> {
    let mut filter = source.auto_filter().cloned();
    for edit in draft
        .grid_histories()
        .get(&sheet_id)
        .into_iter()
        .flat_map(GridHistory::edits)
    {
        filter = filter.and_then(|filter| filter.grid_mapped(*edit));
    }
    filter
}

/// Returns the source sheet whose part a draft sheet's part patches: the sheet itself, or for a
/// copy, the source sheet whose part it duplicates.
fn source_sheet_id(draft: &WorkbookDraft, sheet_id: SheetId) -> SheetId {
    draft
        .copied_sheets()
        .get(&sheet_id)
        .copied()
        .unwrap_or(sheet_id)
}

pub(super) fn validate_dynamic_formula_edits(draft: &WorkbookDraft) -> Result<(), XlsxWriteError> {
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
        // A structural edit moves a source formula's declared spill range with its cell.
        let source_metadata = draft
            .source_cells()
            .formula_metadata(id.sheet_id(), id.address());
        let preserves_dynamic_metadata = current_formula
            .is_none_or(|current| source_metadata.as_ref() == Some(current.metadata()));
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

/// Refuses a copy whose tables are not one copied table for each table of its source part.
fn copy_tables_changed(copy: &PartPath) -> XlsxWriteError {
    XlsxWriteError::new(XlsxWriteErrorCode::UnsupportedPreservation)
        .with_detail(format!("{DETAIL_COPY_TABLES_CHANGED}: {}", copy.as_str()))
        .at_source(copy.source_id())
}
