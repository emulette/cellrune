use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use super::super::error::compatibility;
use super::super::xml::XmlBudget;
use super::super::{XlsxErrorCode, XlsxReadError};
use crate::{
    CellAddress, CellRange, Diagnostic, DiagnosticCode, DiagnosticSeverity, SheetId,
    SourceLocation,
};

/// Collects `<mergeCell>` declarations for one worksheet and validates them on finish.
///
/// Individual invalid declarations become diagnostics and are dropped; only the configured
/// workbook-wide declaration budget fails the read as a whole.
#[derive(Debug, Default)]
pub(super) struct MergedRangeCollector {
    ranges: Vec<CellRange>,
}

impl MergedRangeCollector {
    /// Records one `<mergeCell>` declaration.
    ///
    /// # Errors
    ///
    /// Returns [`XlsxErrorCode::TooManyMergedRanges`] when the workbook-wide declaration
    /// budget is exhausted. Malformed, reversed, out-of-bounds, and single-cell references
    /// are reported as diagnostics and dropped without failing the read.
    pub(super) fn record(
        &mut self,
        reference: Option<&str>,
        total_merged_ranges: &mut u64,
        sheet_id: SheetId,
        diagnostics: &mut Vec<Diagnostic>,
        budget: &XmlBudget,
    ) -> Result<(), XlsxReadError> {
        *total_merged_ranges = total_merged_ranges.saturating_add(1);
        if *total_merged_ranges > budget.limits().max_merged_ranges() {
            return Err(budget.error(XlsxErrorCode::TooManyMergedRanges));
        }
        let Some(reference) = reference else {
            push_diagnostic(
                diagnostics,
                compatibility::MERGED_RANGE_INVALID_CODE,
                compatibility::MERGED_RANGE_INVALID_MESSAGE,
                compatibility::MERGED_RANGE_MISSING_REF,
                sheet_id,
                budget,
            )?;
            return Ok(());
        };
        match parse_reference(reference) {
            ParsedReference::Range(range) => self.ranges.push(range),
            ParsedReference::SingleCell => push_diagnostic(
                diagnostics,
                compatibility::MERGED_RANGE_SINGLE_CELL_CODE,
                compatibility::MERGED_RANGE_SINGLE_CELL_MESSAGE,
                reference,
                sheet_id,
                budget,
            )?,
            ParsedReference::Invalid => push_diagnostic(
                diagnostics,
                compatibility::MERGED_RANGE_INVALID_CODE,
                compatibility::MERGED_RANGE_INVALID_MESSAGE,
                reference,
                sheet_id,
                budget,
            )?,
        }
        Ok(())
    }

    /// Sorts the surviving ranges by top-left then bottom-right address, drops later entries
    /// that overlap an earlier kept entry, and returns the deterministic final list.
    ///
    /// # Errors
    ///
    /// Returns an [`XlsxReadError`] only when a drop diagnostic cannot be constructed.
    pub(super) fn finish(
        mut self,
        sheet_id: SheetId,
        diagnostics: &mut Vec<Diagnostic>,
        budget: &XmlBudget,
    ) -> Result<Vec<CellRange>, XlsxReadError> {
        self.ranges.sort_unstable_by_key(sort_key);
        let mut kept = Vec::with_capacity(self.ranges.len());
        // Kept ranges whose row span still covers the current start row, keyed by their
        // pairwise-disjoint column intervals; the heap expires entries by end row.
        let mut active_columns = BTreeMap::<u32, u32>::new();
        let mut active_expiry = BinaryHeap::<Reverse<(u32, u32)>>::new();
        for range in self.ranges {
            let start_row = range.start().row().get();
            while let Some(Reverse((end_row, column_start))) = active_expiry.peek().copied() {
                if end_row >= start_row {
                    break;
                }
                active_expiry.pop();
                active_columns.remove(&column_start);
            }
            let column_start = range.start().column().get();
            let column_end = range.end().column().get();
            let overlaps = active_columns
                .range(..=column_end)
                .next_back()
                .is_some_and(|(_, active_end)| *active_end >= column_start);
            if overlaps {
                push_diagnostic(
                    diagnostics,
                    compatibility::MERGED_RANGE_OVERLAP_CODE,
                    compatibility::MERGED_RANGE_OVERLAP_MESSAGE,
                    &format_range(range),
                    sheet_id,
                    budget,
                )?;
                continue;
            }
            active_columns.insert(column_start, column_end);
            active_expiry.push(Reverse((range.end().row().get(), column_start)));
            kept.push(range);
        }
        Ok(kept)
    }
}

enum ParsedReference {
    Range(CellRange),
    SingleCell,
    Invalid,
}

fn parse_reference(reference: &str) -> ParsedReference {
    let mut parts = reference.split(':');
    let Some(first) = parts.next() else {
        return ParsedReference::Invalid;
    };
    let second = parts.next();
    if parts.next().is_some() {
        return ParsedReference::Invalid;
    }
    let Ok(start) = CellAddress::from_a1(first) else {
        return ParsedReference::Invalid;
    };
    let Some(second) = second else {
        return ParsedReference::SingleCell;
    };
    let Ok(end) = CellAddress::from_a1(second) else {
        return ParsedReference::Invalid;
    };
    match CellRange::new(start, end) {
        Ok(range) if range.height() == 1 && range.width() == 1 => ParsedReference::SingleCell,
        Ok(range) => ParsedReference::Range(range),
        Err(_) => ParsedReference::Invalid,
    }
}

fn sort_key(range: &CellRange) -> (u32, u32, u32, u32) {
    (
        range.start().row().get(),
        range.start().column().get(),
        range.end().row().get(),
        range.end().column().get(),
    )
}

fn format_range(range: CellRange) -> String {
    format!("{}:{}", range.start(), range.end())
}

fn push_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: &'static str,
    message: &'static str,
    reference: &str,
    sheet_id: SheetId,
    budget: &XmlBudget,
) -> Result<(), XlsxReadError> {
    let code = DiagnosticCode::new(code).map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorksheet)
            .with_cause(error)
    })?;
    let diagnostic = Diagnostic::new(
        code,
        DiagnosticSeverity::Warning,
        format!("{message}: {reference}"),
        Some(SourceLocation::sheet(budget.source_id().clone(), sheet_id)),
    )
    .map_err(|error| {
        budget
            .error(XlsxErrorCode::InvalidWorksheet)
            .with_cause(error)
    })?;
    diagnostics.push(diagnostic);
    Ok(())
}
