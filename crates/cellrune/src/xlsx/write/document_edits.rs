//! Writes the edits of a document draft without calculating, leaving recalculation to the host.

use std::collections::BTreeSet;

use super::document_authoring::{
    DocumentResultPlan, validate_draft_edits, validate_dynamic_formula_edits,
    write_document_package,
};
use super::edit_report::{EditWriteReport, EditedWorkbook};
use super::materialization::MaterializationPlan;
use super::workbook_patch::HostRecalculation;
use super::{WriteOptions, XlsxWriteError, XlsxWriteErrorCode};
use crate::calculation::{DraftEdits, affected_formulas};
use crate::{
    CalculationCellId, Cell, CellAddress, CellContent, FormulaMetadata, OutputHash, SheetId,
    WorkbookDraft, WorkbookSnapshot,
};

const DETAIL_SOURCE_DOCUMENT: &str = "writing draft edits requires a draft opened from a document";

/// Writes a document draft's edits into its preserved package without calculating.
///
/// Every formula the edits can affect has its saved result removed, and only then is the host
/// asked to recalculate on open. Every other saved result, `calcChain.xml` while the formula layout
/// is unchanged, and every untouched part keep their source bytes.
pub(crate) fn write_document_edits(
    draft: &WorkbookDraft,
    options: WriteOptions,
) -> Result<EditedWorkbook, XlsxWriteError> {
    let document = draft.source_document().ok_or_else(|| {
        XlsxWriteError::new(XlsxWriteErrorCode::InvalidPackagePlan)
            .with_detail(DETAIL_SOURCE_DOCUMENT)
    })?;
    validate_dynamic_formula_edits(draft, document)?;
    let limits = options.limits();
    validate_draft_edits(draft, limits)?;
    let source = document.workbook();
    let changed_cells = calculation_changed_cells(draft, source);
    let affected = affected_formulas(
        &DraftEdits {
            source,
            draft: draft.workbook(),
            changed_cells: &changed_cells,
            changed_tables: draft.changed_table_ids(),
        },
        &|| false,
    )
    .expect("uncancellable impact analysis cannot be cancelled");
    let materialization = MaterializationPlan::invalidating(&affected, limits)?;
    let host_recalculation = if !affected.is_empty() {
        Some(HostRecalculation::OnLoad)
    } else if source.calculation_hints() != draft.workbook().calculation_hints() {
        Some(HostRecalculation::Clear)
    } else {
        None
    };
    let output = write_document_package(
        draft,
        document,
        &DocumentResultPlan {
            materialization: &materialization,
            host_recalculation,
            keep_calculation_chain: formula_layout(source) == formula_layout(draft.workbook()),
        },
        limits,
    )?;
    let report = EditWriteReport {
        affected_formulas: affected.into_iter().collect(),
        requests_host_recalculation: host_recalculation == Some(HostRecalculation::OnLoad),
        changed_parts: output.changed_parts,
        removed_parts: output.removed_parts,
        output_hash: OutputHash::for_bytes(&output.bytes),
        input_hash: document.input_hash(),
        semantic_revision: draft.semantic_revision(),
        presentation_revision: draft.presentation_revision(),
    };
    Ok(EditedWorkbook::new(output.bytes, report, document.kind()))
}

/// Cells whose calculation input the draft changed. A shared-formula follower that only became an
/// ordinary formula with the same text is not one.
fn calculation_changed_cells(
    draft: &WorkbookDraft,
    source: &WorkbookSnapshot,
) -> BTreeSet<CalculationCellId> {
    draft
        .cell_mutations()
        .keys()
        .filter(|id| !same_calculation_input(cell(source, **id), cell(draft.workbook(), **id)))
        .copied()
        .collect()
}

fn cell(workbook: &WorkbookSnapshot, id: CalculationCellId) -> Option<&Cell> {
    workbook
        .sheet_by_id(id.sheet_id())
        .and_then(|sheet| sheet.cell(id.address()))
}

fn same_calculation_input(before: Option<&Cell>, after: Option<&Cell>) -> bool {
    let (Some(before), Some(after)) = (before, after) else {
        return before.is_none() && after.is_none();
    };
    let scalar = |metadata: &FormulaMetadata| {
        matches!(
            metadata,
            FormulaMetadata::Normal | FormulaMetadata::Shared { .. }
        )
    };
    before.number_format() == after.number_format()
        && match (before.content(), after.content()) {
            (CellContent::Literal(before), CellContent::Literal(after)) => before == after,
            (CellContent::Formula(before), CellContent::Formula(after)) => {
                before.text() == after.text()
                    && before.recalculate_always() == after.recalculate_always()
                    && (before.metadata() == after.metadata()
                        || (scalar(before.metadata()) && scalar(after.metadata())))
            }
            _ => false,
        }
}

/// Every formula cell with whether it is an array formula, which is all `calcChain.xml` records.
fn formula_layout(workbook: &WorkbookSnapshot) -> BTreeSet<(SheetId, CellAddress, bool)> {
    workbook
        .sheets()
        .iter()
        .flat_map(|sheet| {
            sheet.formula_addresses().iter().filter_map(|address| {
                let CellContent::Formula(formula) = sheet.cell(*address)?.content() else {
                    return None;
                };
                let array = matches!(
                    formula.metadata(),
                    FormulaMetadata::Array { .. } | FormulaMetadata::DynamicArray { .. }
                );
                Some((sheet.id(), *address, array))
            })
        })
        .collect()
}
