use std::collections::{BTreeSet, VecDeque};

use super::eval::{CompiledWorkbook, clone_set_cancellable};
use super::pipeline::{calculate_and_compile, calculate_from_compiled};
use crate::{
    CalculationCellId, CalculationOptions, CalculationSnapshot, EditBatch, EditReceipt,
    ValidationError, WorkbookDraft, WorkbookSnapshot,
};

mod delta;
mod error;
mod impact;
mod limits;

use delta::build_delta;
pub use delta::{CalculationDelta, CalculationDeltaCell, CalculationDeltaPage};
pub use error::{SessionError, SessionErrorCode};
use impact::{affected_formulas, formula_cells};
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
    compiled: Option<CompiledWorkbook>,
    calculation: Option<CalculationSnapshot>,
    calculation_options: Option<CalculationOptions>,
    dirty: BTreeSet<CalculationCellId>,
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
    pub const fn workbook(&self) -> &WorkbookSnapshot {
        self.draft.workbook()
    }

    /// Returns the mutable draft used by verified writers.
    pub const fn draft(&self) -> &WorkbookDraft {
        &self.draft
    }

    /// Returns the installed complete calculation, when available.
    pub const fn calculation(&self) -> Option<&CalculationSnapshot> {
        self.calculation.as_ref()
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
        let mut draft = self.draft.clone();
        let receipt = draft.apply_changes(batch)?;
        Ok(PreparedEditBatch {
            base_revision: current_revision,
            draft,
            receipt,
        })
    }

    /// Installs a previously staged edit batch if its source revision is still current.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::RevisionMismatch`] without changing the session when another
    /// edit was installed after the batch was prepared.
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
        if prepared.receipt.topology_changed() || prepared.receipt.calculation_metadata_changed() {
            self.requires_full_rebuild = true;
        } else if let Some(compiled) = &self.compiled {
            self.dirty.extend(affected_formulas(
                prepared.draft.workbook(),
                compiled,
                prepared.receipt.calculation_changed_cells(),
            ));
        }
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
            .map_or(current_revision, CalculationSnapshot::source_revision);
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
        let unsafe_dynamic = self
            .compiled
            .as_ref()
            .is_some_and(|compiled| !compiled.incremental_safe())
            && !self.dirty.is_empty();
        let compile_required = no_state || self.requires_full_rebuild || compiled_limit_changed;

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
                        self.requires_full_rebuild,
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
            RecalculationMode::Auto if self.requires_full_rebuild => (
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
            formula_cells(self.workbook(), &cancelled)
        } else {
            clone_set_cancellable(&self.dirty, &cancelled)
        }
        .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let workbook = self
            .workbook()
            .clone_cancellable(&cancelled)
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let compiled = self
            .compiled
            .as_ref()
            .map(|compiled| compiled.clone_cancellable(&cancelled))
            .transpose()
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let previous = self
            .calculation
            .as_ref()
            .map(|calculation| calculation.clone_cancellable(&cancelled))
            .transpose()
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        if cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        Ok(PreparedCalculation {
            workbook,
            expected_revision: current_revision,
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

    /// Installs a completed calculation only if the workbook revision is still current.
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
    draft: WorkbookDraft,
    receipt: EditReceipt,
}

impl PreparedEditBatch {
    /// Returns the workbook state that would be installed.
    pub const fn workbook(&self) -> &WorkbookSnapshot {
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
    workbook: WorkbookSnapshot,
    expected_revision: u64,
    base_revision: u64,
    options: CalculationOptions,
    execution_mode: CalculationExecutionMode,
    reason: CalculationDecisionReason,
    compile_required: bool,
    compiled: Option<CompiledWorkbook>,
    previous: Option<CalculationSnapshot>,
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
        let (calculation, compiled, evaluated_count, parsed_formula_count) =
            if self.compile_required {
                let (calculation, compiled, evaluated) =
                    calculate_and_compile(&self.workbook, self.options, || {
                        self.cancellation.is_cancelled()
                    })
                    .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
                let parsed = compiled.formula_count();
                (calculation, compiled, evaluated, parsed)
            } else {
                let compiled = self.compiled.as_ref().ok_or_else(|| {
                    SessionError::new(SessionErrorCode::CalculationUninitialized, None)
                })?;
                let previous = (self.execution_mode == CalculationExecutionMode::Incremental)
                    .then_some(self.previous.as_ref())
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
                let compiled = compiled
                    .clone_cancellable(&|| self.cancellation.is_cancelled())
                    .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
                (calculation, compiled, evaluated, 0)
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
        let delta = build_delta(
            self.previous.as_ref(),
            &calculation,
            self.base_revision,
            self.expected_revision,
            self.execution_mode,
            self.reason,
            self.dirty.len(),
            evaluated_count,
            parsed_formula_count,
            self.limits.max_delta_cells,
            &|| self.cancellation.is_cancelled(),
        )?;
        if self.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        Ok(CompletedCalculation {
            expected_revision: self.expected_revision,
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
    options: CalculationOptions,
    compiled: CompiledWorkbook,
    calculation: CalculationSnapshot,
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
