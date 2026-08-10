use super::Engine;
use super::reference::cell_at;
use crate::CellContent;
use crate::calculation::MaterializedResultOrigin;
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::runtime::{CellId, Rect};
use crate::calculation::scope::ArrayEvaluation;
use crate::calculation::value::{ErrorKind, Value};

#[derive(Debug, Clone, Copy)]
pub(super) struct ArrayRegion {
    pub(super) anchor: CellId,
    pub(super) rect: Rect,
    pub(super) provisional: bool,
}

impl Engine<'_> {
    pub(super) fn legacy_array_range(&self, cell: CellId) -> Option<Rect> {
        let sheet = self.workbook.sheets().get(cell.0)?;
        let source = cell_at(sheet, cell.1, cell.2)?;
        let CellContent::Formula(formula) = source.content() else {
            return None;
        };
        let range = formula.metadata().legacy_array_range_at(source.address())?;
        Some(Rect {
            sheet: cell.0,
            row_start: range.start().row().get(),
            col_start: range.start().column().get(),
            row_end: range.end().row().get(),
            col_end: range.end().column().get(),
            whole_rows: false,
        })
    }

    pub(super) fn dynamic_array_range(&self, cell: CellId) -> Option<Option<Rect>> {
        let sheet = self.workbook.sheets().get(cell.0)?;
        let source = cell_at(sheet, cell.1, cell.2)?;
        let CellContent::Formula(formula) = source.content() else {
            return None;
        };
        formula
            .metadata()
            .dynamic_array_range_at(source.address())
            .map(|range| {
                range.map(|range| Rect {
                    sheet: cell.0,
                    row_start: range.start().row().get(),
                    col_start: range.start().column().get(),
                    row_end: range.end().row().get(),
                    col_end: range.end().column().get(),
                    whole_rows: false,
                })
            })
    }

    pub(super) fn materialize_legacy_array(
        &mut self,
        anchor: CellId,
        range: Rect,
        result: Result<ArrayEvaluation, ErrorKind>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        let declared_cells = range.height().checked_mul(range.width());
        let evaluated = match declared_cells
            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))
            .and_then(|cells| self.ensure_array_cells(cells))
            .and(result)
        {
            Ok(evaluated) => evaluated,
            Err(kind) => {
                self.results.insert(anchor, Value::Error(kind));
                return Ok(());
            }
        };
        let array = evaluated.array;
        for row in range.row_start..=range.row_end {
            for column in range.col_start..=range.col_end {
                if cancelled() {
                    return Err(());
                }
                let array_row = row - range.row_start;
                let array_column = column - range.col_start;
                let value = if array.is_scalar() {
                    array.at(0, 0).clone()
                } else if array_row < array.rows && array_column < array.cols {
                    array.at(array_row, array_column).clone()
                } else {
                    Value::Error(ErrorKind::NA)
                };
                self.results.insert(
                    (range.sheet, row, column),
                    if matches!(value, Value::Blank) {
                        Value::Number(0.0)
                    } else {
                        value
                    },
                );
                let trace_index = if array.is_scalar() {
                    0
                } else {
                    array_row as usize * array.cols as usize + array_column as usize
                };
                if let Some(Some(trace)) = evaluated.decimal_traces.get(trace_index)
                    && (array.is_scalar() || (array_row < array.rows && array_column < array.cols))
                {
                    self.numeric_decimal_traces
                        .insert((range.sheet, row, column), *trace);
                }
            }
        }
        Ok(())
    }

    pub(super) fn materialize_dynamic_array(
        &mut self,
        anchor: CellId,
        declared_range: Option<Rect>,
        result: Result<ArrayEvaluation, ErrorKind>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), ()> {
        let evaluated = match result {
            Ok(evaluated) => evaluated,
            Err(kind) => {
                self.results.insert(anchor, Value::Error(kind));
                return Ok(());
            }
        };
        let array = evaluated.array;
        let Some(range) = (Rect {
            sheet: anchor.0,
            row_start: anchor.1,
            col_start: anchor.2,
            row_end: anchor.1,
            col_end: anchor.2,
            whole_rows: false,
        })
        .resized_from_anchor(u64::from(array.rows), u64::from(array.cols)) else {
            self.results.insert(anchor, Value::Error(ErrorKind::Spill));
            return Ok(());
        };
        if declared_range.is_some_and(|declared| declared != range)
            || self.dynamic_spill_collides(anchor, range, declared_range, cancelled)?
        {
            self.results.insert(anchor, Value::Error(ErrorKind::Spill));
            return Ok(());
        }
        for row in range.row_start..=range.row_end {
            for column in range.col_start..=range.col_end {
                if cancelled() {
                    return Err(());
                }
                let value = array
                    .at(row - range.row_start, column - range.col_start)
                    .clone();
                self.results.insert(
                    (range.sheet, row, column),
                    if matches!(value, Value::Blank) {
                        Value::Number(0.0)
                    } else {
                        value
                    },
                );
                let trace_index = (row - range.row_start) as usize * array.cols as usize
                    + (column - range.col_start) as usize;
                if let Some(Some(trace)) = evaluated.decimal_traces.get(trace_index) {
                    self.numeric_decimal_traces
                        .insert((range.sheet, row, column), *trace);
                }
            }
        }
        self.dynamic_spills.insert(anchor, range);
        if declared_range.is_none() {
            self.array_regions.push(ArrayRegion {
                anchor,
                rect: range,
                provisional: true,
            });
        }
        Ok(())
    }

    fn dynamic_spill_collides(
        &self,
        anchor: CellId,
        range: Rect,
        declared_range: Option<Rect>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool, ()> {
        for row in range.row_start..=range.row_end {
            for column in range.col_start..=range.col_end {
                if cancelled() {
                    return Err(());
                }
                let target = (range.sheet, row, column);
                if target == anchor {
                    continue;
                }
                if self.results.contains_key(&target) {
                    return Ok(true);
                }
                if self.previous_materialized(target).is_some() {
                    return Ok(true);
                }
                let occupied = self
                    .workbook
                    .sheets()
                    .get(range.sheet)
                    .and_then(|sheet| cell_at(sheet, row, column));
                if occupied.is_some_and(|cell| {
                    declared_range.is_none() || matches!(cell.content(), CellContent::Formula(_))
                }) {
                    return Ok(true);
                }
            }
        }
        for existing in self.dynamic_spills.values() {
            if cancelled() {
                return Err(());
            }
            if rects_intersect(existing, &range) {
                return Ok(true);
            }
        }
        for region in &self.array_regions {
            if cancelled() {
                return Err(());
            }
            if region.anchor != anchor && rects_intersect(&region.rect, &range) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(in crate::calculation) fn dynamic_spill(&self, anchor: CellId) -> Option<Rect> {
        self.dynamic_spills.get(&anchor).copied().or_else(|| {
            let public = super::internal_to_public(self.workbook, anchor)?;
            let materialized = self.previous_materialized(anchor)?;
            let MaterializedResultOrigin::DynamicSpill {
                anchor: spill_anchor,
                range,
            } = materialized.origin()
            else {
                return None;
            };
            if spill_anchor != public {
                return None;
            }
            Some(Rect {
                sheet: anchor.0,
                row_start: range.start().row().get(),
                col_start: range.start().column().get(),
                row_end: range.end().row().get(),
                col_end: range.end().column().get(),
                whole_rows: false,
            })
        })
    }

    pub(super) fn array_owner(&self, cell: CellId) -> Option<CellId> {
        self.array_regions
            .iter()
            .find(|region| {
                region.rect.sheet == cell.0
                    && (region.rect.row_start..=region.rect.row_end).contains(&cell.1)
                    && (region.rect.col_start..=region.rect.col_end).contains(&cell.2)
            })
            .map(|region| region.anchor)
    }
}

fn rects_intersect(left: &Rect, right: &Rect) -> bool {
    left.sheet == right.sheet
        && left.row_start <= right.row_end
        && right.row_start <= left.row_end
        && left.col_start <= right.col_end
        && right.col_start <= left.col_end
}
