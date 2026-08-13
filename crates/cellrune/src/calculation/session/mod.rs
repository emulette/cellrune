use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use super::eval::{CompiledWorkbook, clone_set_cancellable};
use super::formula_rewrite::{FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteLimits};
use super::pipeline::{calculate_and_compile, calculate_from_compiled};
use crate::draft::{
    BatchExecutionError, TableMaterializationBudget, TableMaterializationError,
    rewrite_limit_detail,
};
use crate::{
    CalculationCellId, CalculationOptions, CalculationSnapshot, EditBatch, EditReceipt,
    ValidationError, WorkbookDraft, WorkbookSnapshot,
};

mod delta;
mod error;
mod impact;
mod limits;

pub use delta::{CalculationDelta, CalculationDeltaCell, CalculationDeltaPage};
use delta::{DeltaMetadata, build_delta, build_empty_delta, build_incremental_delta};
use error::stale_calculation_cursor_detail;
pub use error::{SessionError, SessionErrorCode};
use impact::{affected_formulas, formula_cells, formula_cells_from_workbook};
pub use limits::{CancellationToken, SessionLimits};

/// Caller-selected recalculation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecalculationMode {
    /// Select incremental calculation when its complete impact boundary is known, otherwise full.
    #[default]
    Auto,
    /// Require a safe incremental pass and return an error instead of falling back.
    Incremental,
    /// Evaluate every formula cell.
    Full,
}

/// Actual schedule used by one installed calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalculationExecutionMode {
    /// Only dirty formula cells were evaluated.
    Incremental,
    /// Every schedulable formula cell was evaluated.
    Full,
}

/// Deterministic explanation for the selected calculation schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CalculationDecisionReason {
    /// No complete calculation state existed.
    InitialCalculation,
    /// The caller explicitly requested a full pass.
    FullRequested,
    /// The caller explicitly requested an incremental pass.
    IncrementalRequested,
    /// Auto mode selected a safe dirty-only pass.
    DirtySubset,
    /// No formula was dirty, so no evaluator work was required.
    NoDirtyFormulas,
    /// Formula, name, or sheet topology changed.
    TopologyChanged,
    /// Calculation options or workbook interpretation changed.
    OptionsChanged,
    /// Dynamic reference or undeclared spill topology requires a conservative full pass.
    DynamicTopology,
    /// Dirty formulas cover the complete compiled formula set.
    DirtySetCoversWorkbook,
}

/// Stateful workbook editor and persistent calculation engine.
#[derive(Debug)]
pub struct WorkbookCalculationSession {
    draft: WorkbookDraft,
    compiled: Option<Arc<CompiledWorkbook>>,
    calculation: Option<Arc<CalculationSnapshot>>,
    calculation_options: Option<CalculationOptions>,
    dirty: BTreeSet<CalculationCellId>,
    calculation_changes_pending: bool,
    requires_full_rebuild: bool,
    limits: SessionLimits,
    next_cursor: u64,
    history: VecDeque<CalculationDelta>,
}

impl WorkbookCalculationSession {
    /// Creates a session around an existing mutable workbook draft.
    pub fn new(draft: WorkbookDraft) -> Self {
        Self::with_limits(draft, SessionLimits::default())
    }

    /// Creates a session with explicit state and response limits.
    pub fn with_limits(draft: WorkbookDraft, limits: SessionLimits) -> Self {
        Self {
            draft,
            compiled: None,
            calculation: None,
            calculation_options: None,
            dirty: BTreeSet::new(),
            calculation_changes_pending: false,
            requires_full_rebuild: true,
            limits,
            next_cursor: 1,
            history: VecDeque::new(),
        }
    }

    /// Creates a new workbook session containing `Sheet1`.
    pub fn create() -> Self {
        Self::new(WorkbookDraft::new())
    }

    /// Returns the current immutable workbook snapshot.
    pub fn workbook(&self) -> &WorkbookSnapshot {
        self.draft.workbook()
    }

    /// Returns the mutable draft used by verified writers.
    pub const fn draft(&self) -> &WorkbookDraft {
        &self.draft
    }

    /// Returns the installed complete calculation, when available.
    pub fn calculation(&self) -> Option<&CalculationSnapshot> {
        self.calculation.as_deref()
    }

    /// Returns the configured session limits.
    pub const fn limits(&self) -> SessionLimits {
        self.limits
    }

    /// Applies one atomic batch after checking the caller's expected revision.
    ///
    /// # Errors
    ///
    /// Returns a stable session error for an outdated revision, empty or excessive batch, or a
    /// workbook validation error if any operation or final invariant fails.
    pub fn apply_changes(
        &mut self,
        expected_revision: u64,
        batch: EditBatch,
    ) -> Result<EditReceipt, ApplyChangesError> {
        let prepared = self.prepare_changes(expected_revision, batch)?;
        self.install_changes(prepared)
    }

    /// Stages one atomic batch without changing the live session.
    ///
    /// The returned batch can be inspected by a communication layer before installation, for
    /// example to enforce a response budget without reporting failure after an edit committed.
    ///
    /// # Errors
    ///
    /// Returns a stable session error for an outdated revision, empty or excessive batch, or a
    /// workbook validation error if any operation or final invariant fails.
    pub fn prepare_changes(
        &self,
        expected_revision: u64,
        batch: EditBatch,
    ) -> Result<PreparedEditBatch, ApplyChangesError> {
        self.prepare_changes_with_cancellation(expected_revision, batch, &CancellationToken::new())
    }

    /// Stages one atomic edit batch with cooperative cancellation.
    ///
    /// Cancellation or resource exhaustion leaves the live session and semantic revision
    /// unchanged.
    ///
    /// # Errors
    ///
    /// Returns the same validation and revision errors as [`Self::prepare_changes`], plus
    /// [`SessionErrorCode::Cancelled`], [`SessionErrorCode::RewriteLimitExceeded`], or
    /// [`SessionErrorCode::TableMaterializationLimitExceeded`].
    pub fn prepare_changes_cancellable(
        &self,
        expected_revision: u64,
        batch: EditBatch,
        cancellation: &CancellationToken,
    ) -> Result<PreparedEditBatch, ApplyChangesError> {
        self.prepare_changes_with_cancellation(expected_revision, batch, cancellation)
    }

    fn prepare_changes_with_cancellation(
        &self,
        expected_revision: u64,
        batch: EditBatch,
        cancellation: &CancellationToken,
    ) -> Result<PreparedEditBatch, ApplyChangesError> {
        let current_revision = self.workbook().semantic_revision();
        if expected_revision != current_revision {
            return Err(SessionError::new(
                SessionErrorCode::RevisionMismatch,
                Some(format!(
                    "expected={expected_revision}, current={current_revision}"
                )),
            )
            .into());
        }
        if batch.is_empty() {
            return Err(SessionError::new(SessionErrorCode::EmptyBatch, None).into());
        }
        if batch.len() > self.limits.max_batch_operations {
            return Err(SessionError::new(
                SessionErrorCode::BatchLimitExceeded,
                Some(format!(
                    "operations={}, limit={}",
                    batch.len(),
                    self.limits.max_batch_operations
                )),
            )
            .into());
        }
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None).into());
        }
        let cancelled = || cancellation.is_cancelled();
        let mut draft = self
            .draft
            .clone_cancellable(&cancelled)
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let rewrite_limits = FormulaRewriteLimits {
            max_formulas: self.limits.max_rewrite_formulas,
            max_source_bytes: self.limits.max_rewrite_source_bytes,
            max_ast_nodes: self.limits.max_rewrite_ast_nodes,
            max_source_edits: self.limits.max_rewrite_source_edits,
        };
        let mut rewrite_budget = FormulaRewriteBudget::new(rewrite_limits, &cancelled);
        let mut materialization_budget =
            TableMaterializationBudget::new(self.limits.max_table_materialized_cells, &cancelled);
        let receipt = draft
            .apply_changes_controlled(batch, &mut rewrite_budget, &mut materialization_budget)
            .map_err(map_batch_execution_error)?;
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None).into());
        }
        let topology_or_metadata_changed =
            receipt.topology_changed() || receipt.calculation_metadata_changed();
        let next_requires_full_rebuild = self.requires_full_rebuild || topology_or_metadata_changed;
        let replacement_dirty_state =
            if !topology_or_metadata_changed && let Some(compiled) = &self.compiled {
                affected_formulas(
                    draft.workbook(),
                    compiled,
                    self.calculation.as_deref(),
                    receipt.calculation_changed_cells(),
                    &self.dirty,
                    &cancelled,
                )
                .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?
            } else {
                clone_set_cancellable(&self.dirty, &cancelled)
                    .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?
            };
        let next_calculation_changes_pending =
            self.calculation_changes_pending || !receipt.calculation_changed_cells().is_empty();
        Ok(PreparedEditBatch {
            base_revision: current_revision,
            base_cursor: self.next_cursor,
            draft,
            receipt,
            replacement_dirty_state,
            next_requires_full_rebuild,
            next_calculation_changes_pending,
        })
    }

    /// Installs a previously staged edit batch if its source revision and calculation
    /// generation are still current.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::RevisionMismatch`] without changing the session when another
    /// edit was installed after the batch was prepared, or [`SessionErrorCode::StaleResult`]
    /// when a calculation was installed after the batch was prepared.
    pub fn install_changes(
        &mut self,
        prepared: PreparedEditBatch,
    ) -> Result<EditReceipt, ApplyChangesError> {
        let current_revision = self.workbook().semantic_revision();
        if current_revision != prepared.base_revision {
            return Err(SessionError::new(
                SessionErrorCode::RevisionMismatch,
                Some(format!(
                    "expected={}, current={current_revision}",
                    prepared.base_revision
                )),
            )
            .into());
        }
        if prepared.base_cursor != self.next_cursor {
            return Err(SessionError::new(
                SessionErrorCode::StaleResult,
                Some(stale_calculation_cursor_detail(
                    prepared.base_cursor,
                    self.next_cursor,
                )),
            )
            .into());
        }
        self.requires_full_rebuild = prepared.next_requires_full_rebuild;
        self.dirty = prepared.replacement_dirty_state;
        self.calculation_changes_pending = prepared.next_calculation_changes_pending;
        self.draft = prepared.draft;
        Ok(prepared.receipt)
    }

    /// Captures an immutable calculation job that can run without holding the session lock.
    ///
    /// # Errors
    ///
    /// Returns a stable error when forced incremental calculation lacks a complete safe state.
    pub fn prepare_recalculation(
        &self,
        mode: RecalculationMode,
        options: CalculationOptions,
        cancellation: CancellationToken,
    ) -> Result<PreparedCalculation, SessionError> {
        let cancelled = || cancellation.is_cancelled();
        if cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let current_revision = self.workbook().semantic_revision();
        let previous_revision = self
            .calculation
            .as_ref()
            .map_or(current_revision, |calculation| {
                calculation.source_revision()
            });
        if let Some(previous) = &self.calculation
            && previous.source_revision() > current_revision
        {
            return Err(SessionError::new(
                SessionErrorCode::StateRevisionMismatch,
                Some(format!(
                    "calculation={}, workbook={current_revision}",
                    previous.source_revision()
                )),
            ));
        }

        let options_changed = self
            .calculation_options
            .is_some_and(|previous| previous != options);
        let no_state = self.calculation.is_none() || self.compiled.is_none();
        let compiled_limit_changed = self
            .compiled
            .as_ref()
            .is_some_and(|compiled| compiled.limits() != options.limits());
        let table_topology_changed = match &self.compiled {
            Some(compiled) => !compiled
                .table_topology_matches(self.workbook(), &cancelled)
                .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?,
            None => false,
        };
        let unsafe_dynamic = self
            .compiled
            .as_ref()
            .is_some_and(|compiled| !compiled.incremental_safe())
            && self.calculation_changes_pending;
        let compile_required = no_state
            || self.requires_full_rebuild
            || compiled_limit_changed
            || table_topology_changed
            || unsafe_dynamic;

        let (execution_mode, reason) = match mode {
            RecalculationMode::Full => (
                CalculationExecutionMode::Full,
                CalculationDecisionReason::FullRequested,
            ),
            RecalculationMode::Incremental if no_state => {
                return Err(SessionError::new(
                    SessionErrorCode::CalculationUninitialized,
                    None,
                ));
            }
            RecalculationMode::Incremental
                if compile_required || options_changed || unsafe_dynamic =>
            {
                return Err(SessionError::new(
                    SessionErrorCode::IncrementalUnsafe,
                    Some(incremental_unsafe_detail(
                        self.requires_full_rebuild || table_topology_changed,
                        options_changed || compiled_limit_changed,
                        unsafe_dynamic,
                    )),
                ));
            }
            RecalculationMode::Incremental => (
                CalculationExecutionMode::Incremental,
                if self.dirty.is_empty() {
                    CalculationDecisionReason::NoDirtyFormulas
                } else {
                    CalculationDecisionReason::IncrementalRequested
                },
            ),
            RecalculationMode::Auto if no_state => (
                CalculationExecutionMode::Full,
                CalculationDecisionReason::InitialCalculation,
            ),
            RecalculationMode::Auto if self.requires_full_rebuild || table_topology_changed => (
                CalculationExecutionMode::Full,
                CalculationDecisionReason::TopologyChanged,
            ),
            RecalculationMode::Auto if options_changed || compiled_limit_changed => (
                CalculationExecutionMode::Full,
                CalculationDecisionReason::OptionsChanged,
            ),
            RecalculationMode::Auto if unsafe_dynamic => (
                CalculationExecutionMode::Full,
                CalculationDecisionReason::DynamicTopology,
            ),
            RecalculationMode::Auto
                if self.compiled.as_ref().is_some_and(|compiled| {
                    !self.dirty.is_empty() && self.dirty.len() >= compiled.formula_count()
                }) =>
            {
                (
                    CalculationExecutionMode::Full,
                    CalculationDecisionReason::DirtySetCoversWorkbook,
                )
            }
            RecalculationMode::Auto if self.dirty.is_empty() => (
                CalculationExecutionMode::Incremental,
                CalculationDecisionReason::NoDirtyFormulas,
            ),
            RecalculationMode::Auto => (
                CalculationExecutionMode::Incremental,
                CalculationDecisionReason::DirtySubset,
            ),
        };

        let dirty = if execution_mode == CalculationExecutionMode::Full {
            match self.compiled.as_deref() {
                Some(compiled) if !compile_required => {
                    formula_cells(self.workbook(), compiled, &cancelled)
                }
                _ => formula_cells_from_workbook(self.workbook(), &cancelled),
            }
        } else {
            clone_set_cancellable(&self.dirty, &cancelled)
        }
        .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let workbook = self.draft.shared_workbook();
        let compiled = self.compiled.as_ref().map(Arc::clone);
        let previous = self.calculation.as_ref().map(Arc::clone);
        if cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        Ok(PreparedCalculation {
            workbook,
            expected_revision: current_revision,
            base_cursor: self.next_cursor,
            base_revision: previous_revision,
            options,
            execution_mode,
            reason,
            compile_required,
            compiled,
            previous,
            dirty,
            cancellation,
            limits: self.limits,
        })
    }

    /// Installs a completed calculation only if its workbook and calculation state are current.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::StaleResult`] or [`SessionErrorCode::Cancelled`] without
    /// changing the installed state.
    pub fn install(
        &mut self,
        completed: CompletedCalculation,
    ) -> Result<CalculationDelta, SessionError> {
        let delta = self.preview_install(&completed)?;
        let history_delta = delta
            .clone_cancellable(&|| completed.cancellation.is_cancelled())
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        if completed.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        self.next_cursor = delta.cursor().checked_add(1).ok_or_else(|| {
            SessionError::new(
                SessionErrorCode::DeltaLimitExceeded,
                Some("delta cursor exhausted".to_owned()),
            )
        })?;
        self.compiled = Some(completed.compiled);
        self.calculation = Some(completed.calculation);
        self.calculation_options = Some(completed.options);
        self.dirty.clear();
        self.calculation_changes_pending = false;
        self.requires_full_rebuild = false;
        self.history.push_back(history_delta);
        while self.history.len() > self.limits.max_retained_deltas {
            self.history.pop_front();
        }
        Ok(delta)
    }

    /// Returns the exact delta that installing a completed calculation would commit.
    ///
    /// This validates cancellation, revision, and cursor state without changing the session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::Cancelled`], [`SessionErrorCode::StaleResult`], or
    /// [`SessionErrorCode::DeltaLimitExceeded`] without changing installed state.
    pub fn preview_install(
        &self,
        completed: &CompletedCalculation,
    ) -> Result<CalculationDelta, SessionError> {
        if completed.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let current_revision = self.workbook().semantic_revision();
        if completed.expected_revision != current_revision {
            return Err(SessionError::new(
                SessionErrorCode::StaleResult,
                Some(format!(
                    "calculated={}, current={current_revision}",
                    completed.expected_revision
                )),
            ));
        }
        if completed.base_cursor != self.next_cursor {
            return Err(SessionError::new(
                SessionErrorCode::StaleResult,
                Some(stale_calculation_cursor_detail(
                    completed.base_cursor,
                    self.next_cursor,
                )),
            ));
        }
        self.next_cursor.checked_add(1).ok_or_else(|| {
            SessionError::new(
                SessionErrorCode::DeltaLimitExceeded,
                Some("delta cursor exhausted".to_owned()),
            )
        })?;
        let mut delta = completed
            .delta
            .clone_cancellable(&|| completed.cancellation.is_cancelled())
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        if completed.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        delta.cursor = self.next_cursor;
        Ok(delta)
    }

    /// Prepares, runs, and installs one calculation synchronously.
    ///
    /// # Errors
    ///
    /// Returns a stable session error for unsafe incremental mode, cancellation, resource limits,
    /// or a stale installation.
    pub fn recalculate(
        &mut self,
        mode: RecalculationMode,
        options: CalculationOptions,
        cancellation: CancellationToken,
    ) -> Result<CalculationDelta, SessionError> {
        let prepared = self.prepare_recalculation(mode, options, cancellation)?;
        let completed = prepared.run()?;
        self.install(completed)
    }

    /// Returns complete installed deltas after an exclusive cursor.
    ///
    /// A zero cursor starts at the oldest retained delta. A zero limit selects the configured page
    /// maximum.
    ///
    /// # Errors
    ///
    /// Returns a stable error for an expired cursor or excessive page size.
    pub fn changes_since(
        &self,
        cursor: u64,
        limit: usize,
    ) -> Result<CalculationDeltaPage, SessionError> {
        let limit = if limit == 0 {
            self.limits.max_delta_page
        } else {
            limit
        };
        if limit > self.limits.max_delta_page {
            return Err(SessionError::new(
                SessionErrorCode::PageLimitExceeded,
                Some(format!(
                    "requested={limit}, limit={}",
                    self.limits.max_delta_page
                )),
            ));
        }
        if let Some(oldest) = self.history.front()
            && cursor != 0
            && cursor < oldest.cursor().saturating_sub(1)
        {
            return Err(SessionError::new(
                SessionErrorCode::CursorExpired,
                Some(format!("cursor={cursor}, oldest={}", oldest.cursor())),
            ));
        }
        let available = self
            .history
            .iter()
            .filter(|delta| delta.cursor() > cursor)
            .collect::<Vec<_>>();
        let deltas = available
            .iter()
            .take(limit)
            .map(|delta| (*delta).clone())
            .collect::<Vec<_>>();
        let next_cursor = (available.len() > deltas.len())
            .then(|| deltas.last().map_or(cursor, CalculationDelta::cursor));
        Ok(CalculationDeltaPage::new(cursor, next_cursor, deltas))
    }
}

impl Default for WorkbookCalculationSession {
    fn default() -> Self {
        Self::create()
    }
}

/// An atomic workbook edit batch staged for guarded installation.
#[derive(Debug)]
pub struct PreparedEditBatch {
    base_revision: u64,
    base_cursor: u64,
    draft: WorkbookDraft,
    receipt: EditReceipt,
    replacement_dirty_state: BTreeSet<CalculationCellId>,
    next_requires_full_rebuild: bool,
    next_calculation_changes_pending: bool,
}

impl PreparedEditBatch {
    /// Returns the workbook state that would be installed.
    pub fn workbook(&self) -> &WorkbookSnapshot {
        self.draft.workbook()
    }

    /// Returns the exact receipt that installation would commit.
    pub const fn receipt(&self) -> &EditReceipt {
        &self.receipt
    }
}

/// An immutable calculation job safe to execute outside a session lock.
#[derive(Debug)]
pub struct PreparedCalculation {
    workbook: Arc<WorkbookSnapshot>,
    expected_revision: u64,
    base_cursor: u64,
    base_revision: u64,
    options: CalculationOptions,
    execution_mode: CalculationExecutionMode,
    reason: CalculationDecisionReason,
    compile_required: bool,
    compiled: Option<Arc<CompiledWorkbook>>,
    previous: Option<Arc<CalculationSnapshot>>,
    dirty: BTreeSet<CalculationCellId>,
    cancellation: CancellationToken,
    limits: SessionLimits,
}

impl PreparedCalculation {
    /// Returns the workbook revision captured by this job.
    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }

    /// Returns the selected execution mode.
    pub const fn execution_mode(&self) -> CalculationExecutionMode {
        self.execution_mode
    }

    /// Executes the job without mutating the source session.
    ///
    /// # Errors
    ///
    /// Returns a stable cancellation or session resource-limit error.
    pub fn run(self) -> Result<CompletedCalculation, SessionError> {
        if self.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let planned_evaluations = self.dirty.len();
        if planned_evaluations > self.limits.max_evaluated_cells {
            return Err(SessionError::new(
                SessionErrorCode::EvaluationLimitExceeded,
                Some(format!(
                    "planned={planned_evaluations}, limit={}",
                    self.limits.max_evaluated_cells
                )),
            ));
        }
        let dirty =
            (self.execution_mode == CalculationExecutionMode::Incremental).then_some(&self.dirty);
        if !self.compile_required && self.reason == CalculationDecisionReason::NoDirtyFormulas {
            let previous = self.previous.as_ref().ok_or_else(|| {
                SessionError::new(SessionErrorCode::CalculationUninitialized, None)
            })?;
            let compiled = self.compiled.as_ref().map(Arc::clone).ok_or_else(|| {
                SessionError::new(SessionErrorCode::CalculationUninitialized, None)
            })?;
            let calculation = Arc::new(
                previous
                    .rebase_source_cancellable(&self.workbook, self.options, &|| {
                        self.cancellation.is_cancelled()
                    })
                    .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?,
            );
            let delta = build_empty_delta(DeltaMetadata {
                base_revision: self.base_revision,
                result_revision: self.expected_revision,
                mode: self.execution_mode,
                reason: self.reason,
                dirty_count: 0,
                evaluated_count: 0,
                parsed_formula_count: 0,
            });
            return Ok(CompletedCalculation {
                expected_revision: self.expected_revision,
                base_cursor: self.base_cursor,
                options: self.options,
                compiled,
                calculation,
                delta,
                cancellation: self.cancellation,
            });
        }
        let (calculation, compiled, evaluated_count, parsed_formula_count) =
            if self.compile_required {
                let (calculation, compiled, evaluated) =
                    calculate_and_compile(&self.workbook, self.options, || {
                        self.cancellation.is_cancelled()
                    })
                    .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
                let parsed = compiled.formula_count();
                (Arc::new(calculation), Arc::new(compiled), evaluated, parsed)
            } else {
                let compiled = self.compiled.as_ref().ok_or_else(|| {
                    SessionError::new(SessionErrorCode::CalculationUninitialized, None)
                })?;
                let previous = (self.execution_mode == CalculationExecutionMode::Incremental)
                    .then_some(self.previous.as_deref())
                    .flatten();
                let (calculation, evaluated) = calculate_from_compiled(
                    &self.workbook,
                    self.options,
                    compiled,
                    previous,
                    dirty,
                    || self.cancellation.is_cancelled(),
                )
                .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
                let compiled = self.compiled.as_ref().map(Arc::clone).ok_or_else(|| {
                    SessionError::new(SessionErrorCode::CalculationUninitialized, None)
                })?;
                (Arc::new(calculation), compiled, evaluated, 0)
            };
        if self.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        if evaluated_count > self.limits.max_evaluated_cells {
            return Err(SessionError::new(
                SessionErrorCode::EvaluationLimitExceeded,
                Some(format!(
                    "evaluated={evaluated_count}, limit={}",
                    self.limits.max_evaluated_cells
                )),
            ));
        }
        let delta = if self.execution_mode == CalculationExecutionMode::Incremental {
            let previous = self.previous.as_deref().ok_or_else(|| {
                SessionError::new(SessionErrorCode::CalculationUninitialized, None)
            })?;
            build_incremental_delta(
                previous,
                &calculation,
                &self.dirty,
                DeltaMetadata {
                    base_revision: self.base_revision,
                    result_revision: self.expected_revision,
                    mode: self.execution_mode,
                    reason: self.reason,
                    dirty_count: self.dirty.len(),
                    evaluated_count,
                    parsed_formula_count: 0,
                },
                self.limits.max_delta_cells,
                &|| self.cancellation.is_cancelled(),
            )?
        } else {
            build_delta(
                self.previous.as_deref(),
                &calculation,
                DeltaMetadata {
                    base_revision: self.base_revision,
                    result_revision: self.expected_revision,
                    mode: self.execution_mode,
                    reason: self.reason,
                    dirty_count: self.dirty.len(),
                    evaluated_count,
                    parsed_formula_count,
                },
                self.limits.max_delta_cells,
                &|| self.cancellation.is_cancelled(),
            )?
        };
        if self.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        Ok(CompletedCalculation {
            expected_revision: self.expected_revision,
            base_cursor: self.base_cursor,
            options: self.options,
            compiled,
            calculation,
            delta,
            cancellation: self.cancellation,
        })
    }
}

/// A calculated but not yet installed session result.
#[derive(Debug)]
pub struct CompletedCalculation {
    expected_revision: u64,
    base_cursor: u64,
    options: CalculationOptions,
    compiled: Arc<CompiledWorkbook>,
    calculation: Arc<CalculationSnapshot>,
    delta: CalculationDelta,
    cancellation: CancellationToken,
}

/// Error boundary for atomic edit validation and state conflicts.
#[derive(Debug)]
pub enum ApplyChangesError {
    /// The session state rejected the request before workbook mutation.
    Session(SessionError),
    /// A workbook invariant rejected the staged batch.
    Validation(ValidationError),
}

impl From<SessionError> for ApplyChangesError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl From<ValidationError> for ApplyChangesError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

fn map_batch_execution_error(error: BatchExecutionError) -> ApplyChangesError {
    match error {
        BatchExecutionError::Rewrite(FormulaRewriteError::Cancelled) => {
            SessionError::new(SessionErrorCode::Cancelled, None).into()
        }
        BatchExecutionError::Rewrite(FormulaRewriteError::LimitExceeded {
            kind,
            limit,
            actual,
        }) => SessionError::new(
            SessionErrorCode::RewriteLimitExceeded,
            Some(rewrite_limit_detail(kind, limit, actual)),
        )
        .into(),
        BatchExecutionError::Materialization(TableMaterializationError::Cancelled) => {
            SessionError::new(SessionErrorCode::Cancelled, None).into()
        }
        BatchExecutionError::Materialization(TableMaterializationError::LimitExceeded {
            limit,
            actual,
        }) => SessionError::new(
            SessionErrorCode::TableMaterializationLimitExceeded,
            Some(format!("cells={actual}, limit={limit}")),
        )
        .into(),
        other => other.into_validation().into(),
    }
}

impl std::fmt::Display for ApplyChangesError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session(error) => error.fmt(formatter),
            Self::Validation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ApplyChangesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Session(error) => Some(error),
            Self::Validation(error) => Some(error),
        }
    }
}

fn incremental_unsafe_detail(
    topology_changed: bool,
    options_changed: bool,
    dynamic_topology: bool,
) -> String {
    if topology_changed {
        "formula, name, sheet, or calculation topology changed".to_owned()
    } else if options_changed {
        "calculation options changed".to_owned()
    } else if dynamic_topology {
        "dynamic dependency or spill topology requires full recalculation".to_owned()
    } else {
        "incremental state is unavailable".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CellAddress, CellValue, FiniteNumber, FormulaText, SheetId, WorkbookChange};

    fn address(value: &str) -> CellAddress {
        CellAddress::from_a1(value).expect("valid test address")
    }

    fn number(value: f64) -> CellValue {
        CellValue::Number(FiniteNumber::new(value).expect("finite test number"))
    }

    #[test]
    fn stable_incremental_work_reuses_compiled_indexes_and_snapshot_chunks() {
        let sheet = SheetId::new(1).expect("valid default sheet ID");
        let mut session = WorkbookCalculationSession::create();
        session
            .apply_changes(
                0,
                EditBatch::new([
                    WorkbookChange::set_cell_value(sheet, address("A1"), number(1.0)),
                    WorkbookChange::set_cell_formula(
                        sheet,
                        address("B1"),
                        FormulaText::from_xlsx("A1+1").expect("valid test formula"),
                    ),
                    WorkbookChange::set_cell_value(sheet, address("A2"), number(1.0)),
                    WorkbookChange::set_cell_formula(
                        sheet,
                        address("B2"),
                        FormulaText::from_xlsx("A2+1").expect("valid test formula"),
                    ),
                ]),
            )
            .expect("initial test workbook");
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("initial calculation");

        let no_dirty = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("no-dirty calculation");
        assert_eq!(no_dirty.evaluated_count(), 0);

        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    address("A1"),
                    number(2.0),
                )]),
            )
            .expect("one-cell test edit");
        let one_dirty = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("one-dirty calculation");
        assert_eq!(one_dirty.evaluated_count(), 1);
    }

    #[test]
    fn range_impact_index_visits_only_the_matching_sheet_local_intervals() {
        let sheet = SheetId::new(1).expect("valid default sheet ID");
        let mut changes = Vec::with_capacity(10_400);
        for column in 1..=10 {
            for row in 1..=1_000 {
                changes.push(WorkbookChange::set_cell_value(
                    sheet,
                    CellAddress::from_indices(row, column).expect("valid range input"),
                    number(1.0),
                ));
            }
            let column_name =
                char::from_u32(u32::from(b'A') + column - 1).expect("generated A:J column");
            for row in 1_002..=1_041 {
                changes.push(WorkbookChange::set_cell_formula(
                    sheet,
                    CellAddress::from_indices(row, column).expect("valid range formula"),
                    FormulaText::from_xlsx(format!("SUM({column_name}1:{column_name}1000)"))
                        .expect("valid generated range formula"),
                ));
            }
        }
        let mut session = WorkbookCalculationSession::create();
        session
            .apply_changes(0, EditBatch::new(changes))
            .expect("range-index test workbook");
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("initial range calculation");

        session
            .apply_changes(
                session.workbook().semantic_revision(),
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    address("A500"),
                    number(2.0),
                )]),
            )
            .expect("one range input edit");
        let delta = session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("range-index incremental calculation");
        assert_eq!(delta.evaluated_count(), 40);
    }

    #[test]
    fn cancelled_impact_preparation_leaves_session_unchanged() {
        let sheet = SheetId::new(1).expect("valid default sheet ID");
        let mut session = WorkbookCalculationSession::create();
        session
            .apply_changes(
                0,
                EditBatch::new([
                    WorkbookChange::set_cell_value(sheet, address("A1"), number(1.0)),
                    WorkbookChange::set_cell_formula(
                        sheet,
                        address("B1"),
                        FormulaText::from_xlsx("A1+1").expect("valid test formula"),
                    ),
                ]),
            )
            .expect("initial workbook");
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("initial calculation");

        let before_dirty = session.dirty.clone();
        let before_requires_full_rebuild = session.requires_full_rebuild;
        let before_pending = session.calculation_changes_pending;
        let before_revision = session.workbook().semantic_revision();
        let before_calculation = session.calculation.clone();

        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = session
            .prepare_changes_cancellable(
                before_revision,
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    address("A1"),
                    number(2.0),
                )]),
                &cancellation,
            )
            .expect_err("pre-cancelled preparation must fail");

        let code = match error {
            ApplyChangesError::Session(error) => error.code(),
            ApplyChangesError::Validation(_) => panic!("expected a session cancellation error"),
        };
        assert_eq!(code, SessionErrorCode::Cancelled);

        assert_eq!(session.dirty, before_dirty);
        assert_eq!(session.requires_full_rebuild, before_requires_full_rebuild);
        assert_eq!(session.calculation_changes_pending, before_pending);
        assert_eq!(session.workbook().semantic_revision(), before_revision);
        match (&before_calculation, &session.calculation) {
            (Some(before), Some(after)) => assert!(Arc::ptr_eq(before, after)),
            (None, None) => {}
            _ => panic!("calculation state must be unchanged"),
        }
    }

    #[test]
    fn impact_preparation_observes_mid_flight_cancellation() {
        use std::cell::Cell;
        use std::rc::Rc;

        let sheet = SheetId::new(1).expect("valid default sheet ID");
        const CHAIN_LEN: u32 = 1_025;
        let mut changes = Vec::with_capacity(CHAIN_LEN as usize);
        changes.push(WorkbookChange::set_cell_value(
            sheet,
            CellAddress::from_indices(1, 1).expect("chain input cell"),
            number(1.0),
        ));
        for row in 2..=CHAIN_LEN {
            changes.push(WorkbookChange::set_cell_formula(
                sheet,
                CellAddress::from_indices(row, 1).expect("chain formula cell"),
                FormulaText::from_xlsx(format!("A{}+1", row - 1))
                    .expect("valid generated chain formula"),
            ));
        }
        let limits = SessionLimits::new(2_000, 2_000, 2_000, 256, 100).expect("test fanout limits");
        let mut session = WorkbookCalculationSession::with_limits(WorkbookDraft::new(), limits);
        session
            .apply_changes(0, EditBatch::new(changes))
            .expect("chain workbook");
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("initial chain calculation");

        let compiled = session.compiled.clone().expect("compiled workbook");
        let workbook = session.workbook();
        let previous = session.calculation.as_deref();
        let changed = [CalculationCellId::new(sheet, address("A1"))];

        let polls = Rc::new(Cell::new(0_u32));
        let cancelled = {
            let polls = Rc::clone(&polls);
            move || {
                let count = polls.get() + 1;
                polls.set(count);
                count >= 2
            }
        };

        let result = affected_formulas(
            workbook,
            compiled.as_ref(),
            previous,
            &changed,
            &BTreeSet::new(),
            &cancelled,
        );

        assert!(
            result.is_err(),
            "cancellation on the second poll must abort impact preparation"
        );
        assert_eq!(polls.get(), 2);
    }

    #[test]
    fn direct_fanout_observes_mid_flight_cancellation() {
        use std::cell::Cell;
        use std::rc::Rc;

        let sheet = SheetId::new(1).expect("valid default sheet ID");
        const FORMULAS: u32 = 1_024;
        let mut changes = Vec::with_capacity(FORMULAS as usize + 1);
        changes.push(WorkbookChange::set_cell_value(
            sheet,
            address("A1"),
            number(1.0),
        ));
        for row in 2..=FORMULAS + 1 {
            changes.push(WorkbookChange::set_cell_formula(
                sheet,
                CellAddress::from_indices(row, 2).expect("fanout formula cell"),
                FormulaText::from_xlsx("$A$1+1").expect("valid fanout formula"),
            ));
        }
        let limits = SessionLimits::new(2_000, 2_000, 2_000, 256, 100).expect("test fanout limits");
        let mut session = WorkbookCalculationSession::with_limits(WorkbookDraft::new(), limits);
        session
            .apply_changes(0, EditBatch::new(changes))
            .expect("direct fanout workbook");
        session
            .recalculate(
                RecalculationMode::Full,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("initial direct fanout calculation");

        let compiled = session.compiled.clone().expect("compiled workbook");
        let changed = [CalculationCellId::new(sheet, address("A1"))];
        let polls = Rc::new(Cell::new(0_u32));
        let cancelled = {
            let polls = Rc::clone(&polls);
            move || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 3
            }
        };
        let result = affected_formulas(
            session.workbook(),
            compiled.as_ref(),
            session.calculation.as_deref(),
            &changed,
            &BTreeSet::new(),
            &cancelled,
        );
        assert!(result.is_err());
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn staged_edit_rejects_an_intervening_calculation_generation() {
        let sheet = SheetId::new(1).expect("valid default sheet ID");
        let mut session = WorkbookCalculationSession::create();
        session
            .apply_changes(
                0,
                EditBatch::new([
                    WorkbookChange::set_cell_value(sheet, address("A1"), number(1.0)),
                    WorkbookChange::set_cell_formula(
                        sheet,
                        address("B1"),
                        FormulaText::from_xlsx("A1+1").expect("valid test formula"),
                    ),
                ]),
            )
            .expect("initial workbook");
        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("initial calculation");

        let revision = session.workbook().semantic_revision();
        let prepared = session
            .prepare_changes(
                revision,
                EditBatch::new([WorkbookChange::set_cell_value(
                    sheet,
                    address("A1"),
                    number(2.0),
                )]),
            )
            .expect("staged edit");

        session
            .recalculate(
                RecalculationMode::Auto,
                CalculationOptions::default(),
                CancellationToken::new(),
            )
            .expect("intervening calculation");

        let error = session
            .install_changes(prepared)
            .expect_err("install must reject a stale calculation generation");
        let ApplyChangesError::Session(error) = error else {
            panic!("expected a session error");
        };
        assert_eq!(error.code(), SessionErrorCode::StaleResult);
    }
}
