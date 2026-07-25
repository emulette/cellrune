use std::collections::BTreeMap;

use crate::{
    CalculationCellId, CalculationCellResult, CalculationSnapshot, CellValue,
    MaterializedResultOrigin,
};

use super::{RecalculationWritePolicy, WriteLimits, XlsxWriteError, XlsxWriteErrorCode};

const DETAIL_INCOMPLETE_CALCULATION: &str =
    "one or more materialized cells do not have a current calculation value";
const DETAIL_MATERIALIZATION_COUNT: &str = "max_materialized_formula_cells";
const DETAIL_FOLLOWER_COUNT: &str = "max_materialized_spill_cells";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MaterializationAction {
    Set(CellValue),
    Invalidate,
}

#[derive(Debug, Clone)]
pub(crate) struct PlannedMaterialization {
    pub(crate) origin: MaterializedResultOrigin,
    pub(crate) action: MaterializationAction,
}

pub(crate) struct MaterializationPlan {
    cells: BTreeMap<CalculationCellId, PlannedMaterialization>,
    invalidated_cells: Vec<CalculationCellId>,
    materialized_count: usize,
}

impl MaterializationPlan {
    pub(crate) fn new(
        calculation: &CalculationSnapshot,
        policy: RecalculationWritePolicy,
        limits: WriteLimits,
    ) -> Result<Self, XlsxWriteError> {
        enforce_count(
            DETAIL_MATERIALIZATION_COUNT,
            calculation.len(),
            limits.max_materialized_formula_cells(),
        )?;
        let follower_count = calculation
            .materialized_cells()
            .len()
            .saturating_sub(calculation.len());
        enforce_count(
            DETAIL_FOLLOWER_COUNT,
            follower_count,
            limits.max_materialized_spill_cells(),
        )?;

        let mut cells = BTreeMap::new();
        let mut invalidated_cells = Vec::new();
        let mut materialized_count = 0_usize;
        for (id, materialized) in calculation.materialized_cells() {
            let action = match materialized.result() {
                CalculationCellResult::Value(value) => {
                    materialized_count = materialized_count.saturating_add(1);
                    MaterializationAction::Set(value.clone())
                }
                CalculationCellResult::Unavailable(_) => match policy {
                    RecalculationWritePolicy::RequireComplete => {
                        return Err(
                            XlsxWriteError::new(XlsxWriteErrorCode::IncompleteCalculation)
                                .with_detail(DETAIL_INCOMPLETE_CALCULATION),
                        );
                    }
                    RecalculationWritePolicy::InvalidateUnavailable => {
                        invalidated_cells.push(id);
                        MaterializationAction::Invalidate
                    }
                },
            };
            cells.insert(
                id,
                PlannedMaterialization {
                    origin: materialized.origin(),
                    action,
                },
            );
        }
        Ok(Self {
            cells,
            invalidated_cells,
            materialized_count,
        })
    }

    pub(crate) const fn cells(&self) -> &BTreeMap<CalculationCellId, PlannedMaterialization> {
        &self.cells
    }

    pub(crate) fn invalidated_cells(&self) -> &[CalculationCellId] {
        &self.invalidated_cells
    }

    pub(crate) const fn materialized_count(&self) -> usize {
        self.materialized_count
    }

    pub(crate) const fn is_complete(&self) -> bool {
        self.invalidated_cells.is_empty()
    }
}

fn enforce_count(name: &'static str, actual: usize, maximum: u64) -> Result<(), XlsxWriteError> {
    if actual as u64 > maximum {
        return Err(
            XlsxWriteError::new(XlsxWriteErrorCode::ResourceLimitExceeded)
                .with_detail(format!("{name}: {actual} > {maximum}")),
        );
    }
    Ok(())
}
