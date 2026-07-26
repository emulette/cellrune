use std::path::PathBuf;

use cellrune_binding_support::{SharedWorkbookSession, WorkbookSessionGuard};
use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationOptionsDto, EditBatchDto, FinancialSolverSemanticsDto,
    InteropError, RangeRequestDto, RecalculationModeDto, WorkbookSession, WritableCellValueDto,
    WriteOptionsDto,
};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyDict, PyInt};

use crate::conversion;
use crate::error::into_py_error;

#[pyclass(module = "cellrune._native")]
pub(crate) struct Workbook {
    session: SharedWorkbookSession,
}

#[pymethods]
impl Workbook {
    #[staticmethod]
    pub fn create() -> Self {
        Self::new(WorkbookSession::create())
    }

    #[staticmethod]
    pub fn open_path(py: Python<'_>, path: PathBuf) -> PyResult<Self> {
        py.detach(move || WorkbookSession::open_path(path))
            .map(Self::new)
            .map_err(|error| into_py_error(py, error))
    }

    #[staticmethod]
    pub fn from_bytes(py: Python<'_>, bytes: &Bound<'_, PyBytes>) -> PyResult<Self> {
        let owned = bytes.as_bytes().to_vec();
        py.detach(move || WorkbookSession::open_bytes(&owned))
            .map(Self::new)
            .map_err(|error| into_py_error(py, error))
    }

    #[getter]
    pub fn closed(&self) -> bool {
        self.session.is_closed()
    }

    pub fn close(&self) {
        self.session.close();
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        if slf.closed() {
            return Err(into_py_error(slf.py(), InteropError::session_closed()));
        }
        Ok(slf)
    }

    fn __exit__(
        &self,
        _exception_type: Option<&Bound<'_, PyAny>>,
        _exception: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) {
        self.close();
    }

    pub fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(|| self.lock_interop().map(|session| session.summary()));
        let summary = result.map_err(|error| into_py_error(py, error))?;
        conversion::workbook_summary(py, &summary)
    }

    #[pyo3(signature = (sheet, start, end, *, offset=None, limit=None))]
    pub fn read_range<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        start: String,
        end: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let offset = page_offset_from_python(offset).map_err(|error| into_py_error(py, error))?;
        let limit = page_limit_from_python(limit).map_err(|error| into_py_error(py, error))?;
        let result = py.detach(move || {
            self.lock_interop()?.read_range(&RangeRequestDto {
                sheet,
                start,
                end,
                offset,
                limit,
            })
        });
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::range_page(py, &page)
    }

    pub fn function_usage<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(|| self.lock_interop().map(|session| session.function_usage()));
        let report = result.map_err(|error| into_py_error(py, error))?;
        conversion::function_usage(py, &report)
    }

    #[pyo3(signature = (
        *,
        today_serial=None,
        now_serial=None,
        arithmetic_semantics="excel_near_zero",
        financial_solver_semantics="excel_iteration_budget"
    ))]
    pub fn calculate<'py>(
        &self,
        py: Python<'py>,
        today_serial: Option<&Bound<'_, PyAny>>,
        now_serial: Option<&Bound<'_, PyAny>>,
        arithmetic_semantics: &str,
        financial_solver_semantics: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let today_serial =
            optional_number_from_python(today_serial).map_err(|error| into_py_error(py, error))?;
        let now_serial =
            optional_number_from_python(now_serial).map_err(|error| into_py_error(py, error))?;
        let arithmetic_semantics = parse_arithmetic_semantics(arithmetic_semantics)
            .map_err(|error| into_py_error(py, error))?;
        let financial_solver_semantics =
            parse_financial_solver_semantics(financial_solver_semantics)
                .map_err(|error| into_py_error(py, error))?;
        let prepared = self
            .lock(py)?
            .prepare_recalculation(
                RecalculationModeDto::Auto,
                CalculationOptionsDto {
                    today_serial,
                    now_serial,
                    arithmetic_semantics,
                    financial_solver_semantics,
                },
            )
            .map_err(|error| into_py_error(py, error))?;
        let request_id = prepared.request_id();
        let completed = match py.detach(move || prepared.run()) {
            Ok(completed) => completed,
            Err(error) => {
                let cleanup = py.detach(|| {
                    self.lock_interop_wait()
                        .map(|mut session| session.abandon_recalculation(request_id))
                });
                return Err(into_py_error(py, cleanup.err().unwrap_or(error)));
            }
        };
        let report = py
            .detach(move || {
                let mut session = self.lock_interop_wait()?;
                session
                    .install_recalculation(completed)
                    .and_then(|_| session.calculation_report())
            })
            .map_err(|error| into_py_error(py, error))?;
        conversion::calculation_report(py, &report)
    }

    #[pyo3(signature = (
        *,
        mode="auto",
        today_serial=None,
        now_serial=None,
        arithmetic_semantics="excel_near_zero",
        financial_solver_semantics="excel_iteration_budget"
    ))]
    pub fn recalculate<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        today_serial: Option<&Bound<'_, PyAny>>,
        now_serial: Option<&Bound<'_, PyAny>>,
        arithmetic_semantics: &str,
        financial_solver_semantics: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mode = parse_recalculation_mode(mode).map_err(|error| into_py_error(py, error))?;
        let today_serial =
            optional_number_from_python(today_serial).map_err(|error| into_py_error(py, error))?;
        let now_serial =
            optional_number_from_python(now_serial).map_err(|error| into_py_error(py, error))?;
        let arithmetic_semantics = parse_arithmetic_semantics(arithmetic_semantics)
            .map_err(|error| into_py_error(py, error))?;
        let financial_solver_semantics =
            parse_financial_solver_semantics(financial_solver_semantics)
                .map_err(|error| into_py_error(py, error))?;
        let prepared = self
            .lock(py)?
            .prepare_recalculation(
                mode,
                CalculationOptionsDto {
                    today_serial,
                    now_serial,
                    arithmetic_semantics,
                    financial_solver_semantics,
                },
            )
            .map_err(|error| into_py_error(py, error))?;
        let request_id = prepared.request_id();
        let completed = match py.detach(move || prepared.run()) {
            Ok(completed) => completed,
            Err(error) => {
                let cleanup = py.detach(|| {
                    self.lock_interop_wait()
                        .map(|mut session| session.abandon_recalculation(request_id))
                });
                return Err(into_py_error(py, cleanup.err().unwrap_or(error)));
            }
        };
        let delta = py
            .detach(move || self.lock_interop_wait()?.install_recalculation(completed))
            .map_err(|error| into_py_error(py, error))?;
        conversion::calculation_delta(py, &delta)
    }

    pub fn apply_changes<'py>(
        &self,
        py: Python<'py>,
        expected_revision: &Bound<'_, PyAny>,
        changes: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let expected_revision =
            revision_from_python(expected_revision).map_err(|error| into_py_error(py, error))?;
        let batch = edit_batch_from_python(py, changes)?;
        let receipt = self
            .lock(py)?
            .apply_changes(expected_revision, batch)
            .map_err(|error| into_py_error(py, error))?;
        conversion::edit_receipt(py, &receipt)
    }

    #[pyo3(signature = (cursor=None, *, limit=None))]
    pub fn changes_since<'py>(
        &self,
        py: Python<'py>,
        cursor: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let cursor = cursor
            .map(revision_from_python)
            .transpose()
            .map_err(|error| into_py_error(py, error))?
            .unwrap_or(0);
        let limit = page_limit_from_python(limit).map_err(|error| into_py_error(py, error))?;
        let page = self
            .lock(py)?
            .changes_since(cursor, limit)
            .map_err(|error| into_py_error(py, error))?;
        conversion::calculation_delta_page(py, &page)
    }

    pub fn cancel_calculation(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(self.lock(py)?.cancel_calculation())
    }

    pub fn calculation_active(&self, py: Python<'_>) -> PyResult<bool> {
        Ok(self.lock(py)?.calculation_active())
    }

    pub fn set_blank(&self, py: Python<'_>, sheet: &str, address: &str) -> PyResult<()> {
        self.set_value(py, sheet, address, WritableCellValueDto::Blank)
    }

    pub fn set_number(
        &self,
        py: Python<'_>,
        sheet: &str,
        address: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let value = number_from_python(value).map_err(|error| into_py_error(py, error))?;
        self.set_value(py, sheet, address, WritableCellValueDto::Number { value })
    }

    pub fn set_text(
        &self,
        py: Python<'_>,
        sheet: &str,
        address: &str,
        value: String,
    ) -> PyResult<()> {
        self.set_value(py, sheet, address, WritableCellValueDto::Text { value })
    }

    pub fn set_logical(
        &self,
        py: Python<'_>,
        sheet: &str,
        address: &str,
        value: bool,
    ) -> PyResult<()> {
        self.set_value(py, sheet, address, WritableCellValueDto::Logical { value })
    }

    pub fn set_error(
        &self,
        py: Python<'_>,
        sheet: &str,
        address: &str,
        value: String,
    ) -> PyResult<()> {
        self.set_value(py, sheet, address, WritableCellValueDto::Error { value })
    }

    #[pyo3(signature = (sheet, address, formula, *, dynamic_range=None))]
    pub fn set_formula(
        &self,
        py: Python<'_>,
        sheet: &str,
        address: &str,
        formula: &str,
        dynamic_range: Option<&str>,
    ) -> PyResult<()> {
        self.lock(py)?
            .set_formula(sheet, address, formula, dynamic_range)
            .map_err(|error| into_py_error(py, error))
    }

    pub fn clear_cell(&self, py: Python<'_>, sheet: &str, address: &str) -> PyResult<bool> {
        self.lock(py)?
            .clear_cell(sheet, address)
            .map_err(|error| into_py_error(py, error))
    }

    pub fn add_sheet(&self, py: Python<'_>, name: &str) -> PyResult<u32> {
        self.lock(py)?
            .add_sheet(name)
            .map_err(|error| into_py_error(py, error))
    }

    pub fn rename_sheet(&self, py: Python<'_>, current_name: &str, new_name: &str) -> PyResult<()> {
        self.lock(py)?
            .rename_sheet(current_name, new_name)
            .map_err(|error| into_py_error(py, error))
    }

    #[pyo3(signature = (*, invalidate_unavailable=false))]
    pub fn to_bytes<'py>(
        &self,
        py: Python<'py>,
        invalidate_unavailable: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let result = py.detach(|| {
            self.lock_interop()?.save_bytes(WriteOptionsDto {
                invalidate_unavailable,
                replace_existing: false,
            })
        });
        let (bytes, _) = result.map_err(|error| into_py_error(py, error))?;
        Ok(PyBytes::new(py, &bytes))
    }

    #[pyo3(signature = (path, *, invalidate_unavailable=false, replace_existing=false))]
    pub fn save<'py>(
        &self,
        py: Python<'py>,
        path: PathBuf,
        invalidate_unavailable: bool,
        replace_existing: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(move || {
            self.lock_interop()?.save_path(
                path,
                WriteOptionsDto {
                    invalidate_unavailable,
                    replace_existing,
                },
            )
        });
        let report = result.map_err(|error| into_py_error(py, error))?;
        conversion::write_report(py, &report)
    }
}

impl Workbook {
    fn new(session: WorkbookSession) -> Self {
        Self {
            session: SharedWorkbookSession::new(session),
        }
    }

    fn set_value(
        &self,
        py: Python<'_>,
        sheet: &str,
        address: &str,
        value: WritableCellValueDto,
    ) -> PyResult<()> {
        self.lock(py)?
            .set_value(sheet, address, value)
            .map_err(|error| into_py_error(py, error))
    }

    fn lock<'a>(&'a self, py: Python<'_>) -> PyResult<WorkbookSessionGuard<'a>> {
        self.lock_interop()
            .map_err(|error| into_py_error(py, error))
    }

    fn lock_interop(&self) -> Result<WorkbookSessionGuard<'_>, InteropError> {
        self.session.try_lock()
    }

    fn lock_interop_wait(&self) -> Result<WorkbookSessionGuard<'_>, InteropError> {
        self.session.lock()
    }
}

fn page_offset_from_python(value: Option<&Bound<'_, PyAny>>) -> Result<u64, InteropError> {
    let Some(value) = value else {
        return Ok(0);
    };
    if !value.is_instance_of::<PyInt>() || value.is_instance_of::<PyBool>() {
        return Err(InteropError::invalid_page_offset());
    }
    value
        .extract::<u64>()
        .map_err(|_| InteropError::invalid_page_offset())
}

fn page_limit_from_python(value: Option<&Bound<'_, PyAny>>) -> Result<u32, InteropError> {
    let Some(value) = value else {
        return Ok(0);
    };
    if !value.is_instance_of::<PyInt>() || value.is_instance_of::<PyBool>() {
        return Err(InteropError::invalid_page_limit());
    }
    value
        .extract::<u32>()
        .map_err(|_| InteropError::invalid_page_limit())
}

fn revision_from_python(value: &Bound<'_, PyAny>) -> Result<u64, InteropError> {
    if !value.is_instance_of::<PyInt>() || value.is_instance_of::<PyBool>() {
        return Err(InteropError::invalid_revision_or_cursor());
    }
    value
        .extract::<u64>()
        .map_err(|_| InteropError::invalid_revision_or_cursor())
}

fn optional_number_from_python(
    value: Option<&Bound<'_, PyAny>>,
) -> Result<Option<f64>, InteropError> {
    value.map(number_from_python).transpose()
}

fn number_from_python(value: &Bound<'_, PyAny>) -> Result<f64, InteropError> {
    if value.is_instance_of::<PyBool>() {
        return Err(InteropError::invalid_number());
    }
    value
        .extract::<f64>()
        .map_err(|_| InteropError::invalid_number())
}

fn edit_batch_from_python(py: Python<'_>, changes: &Bound<'_, PyAny>) -> PyResult<EditBatchDto> {
    let payload = PyDict::new(py);
    payload.set_item("changes", changes)?;
    let serialized = py
        .import("json")?
        .call_method1("dumps", (payload,))?
        .extract::<String>()?;
    serde_json::from_str(&serialized)
        .map_err(|error| into_py_error(py, InteropError::invalid_change_payload(error.to_string())))
}

fn parse_recalculation_mode(value: &str) -> Result<RecalculationModeDto, InteropError> {
    match value {
        "auto" => Ok(RecalculationModeDto::Auto),
        "incremental" => Ok(RecalculationModeDto::Incremental),
        "full" => Ok(RecalculationModeDto::Full),
        _ => Err(InteropError::invalid_recalculation_mode()),
    }
}

fn parse_arithmetic_semantics(value: &str) -> Result<ArithmeticSemanticsDto, InteropError> {
    match value {
        "excel_near_zero" => Ok(ArithmeticSemanticsDto::ExcelNearZero),
        "ieee_754" => Ok(ArithmeticSemanticsDto::Ieee754),
        _ => Err(InteropError::invalid_arithmetic_semantics()),
    }
}

fn parse_financial_solver_semantics(
    value: &str,
) -> Result<FinancialSolverSemanticsDto, InteropError> {
    match value {
        "excel_iteration_budget" => Ok(FinancialSolverSemanticsDto::ExcelIterationBudget),
        "extended_search" => Ok(FinancialSolverSemanticsDto::ExtendedSearch),
        _ => Err(InteropError::invalid_financial_solver_semantics()),
    }
}
