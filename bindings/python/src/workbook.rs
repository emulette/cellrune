use std::path::PathBuf;

use cellrune_binding_support::{SharedWorkbookSession, WorkbookSessionGuard};
use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationLimitOptionsDto, CalculationOptionsDto,
    CellDependencyRequestDto, CsvReadOptionsDto, CsvWriteRequestDto,
    DefinedNameInspectionRequestDto, EditBatchDto, EditBatchV2Dto, EditWriteOptionsDto,
    FinancialSolverSemanticsDto, InteropError, LocaleDto, LocaleProfileDto, PreviewCursorDto,
    RangeReadOptionsDto, RangeRequestDto, RangeValuesRequestDto, RangeWriteRequestDto,
    ReadLimitOptionsDto, RecalculationModeDto, SheetPageRequestDto, TransactionDetailSectionDto,
    WorkbookSession, WritableCellValueDto, WriteOptionsDto,
};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyDict, PyInt, PyList};

use crate::conversion;
use crate::error::into_py_error;

#[pyclass(module = "cellrune._native")]
pub(crate) struct Workbook {
    session: SharedWorkbookSession,
}

#[pymethods]
impl Workbook {
    #[pyo3(signature = (targets, **options))]
    pub fn calculate_targets<'py>(
        &self,
        py: Python<'py>,
        targets: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request = crate::targeted::request_from_python(py, targets, options)?;
        let response = py
            .detach(|| cellrune_binding_support::calculate_targets(&self.session, &request))
            .map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &response)
    }

    #[staticmethod]
    pub fn create() -> Self {
        Self::new(WorkbookSession::create())
    }

    #[staticmethod]
    #[pyo3(signature = (path, *, read_limits=None))]
    pub fn open_path(
        py: Python<'_>,
        path: PathBuf,
        read_limits: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let limits = limits_from_python::<ReadLimitOptionsDto>(py, read_limits)?;
        py.detach(move || WorkbookSession::open_path_with_limits(path, limits))
            .map(Self::new)
            .map_err(|error| into_py_error(py, error))
    }

    #[staticmethod]
    #[pyo3(signature = (bytes, *, read_limits=None))]
    pub fn from_bytes(
        py: Python<'_>,
        bytes: &Bound<'_, PyBytes>,
        read_limits: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let limits = limits_from_python::<ReadLimitOptionsDto>(py, read_limits)?;
        let owned = bytes.as_bytes().to_vec();
        py.detach(move || WorkbookSession::open_bytes_with_limits(&owned, limits))
            .map(Self::new)
            .map_err(|error| into_py_error(py, error))
    }

    #[staticmethod]
    #[pyo3(signature = (data, *, sheet_name=None, delimiter=None))]
    pub fn from_csv(
        py: Python<'_>,
        data: &Bound<'_, PyBytes>,
        sheet_name: Option<String>,
        delimiter: Option<String>,
    ) -> PyResult<Self> {
        let owned = data.as_bytes().to_vec();
        let options = CsvReadOptionsDto {
            sheet_name,
            delimiter,
        };
        py.detach(move || WorkbookSession::open_csv_bytes(&owned, &options))
            .map(Self::new)
            .map_err(|error| into_py_error(py, error))
    }

    #[pyo3(signature = (sheet, *, delimiter=None))]
    pub fn to_csv<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        delimiter: Option<String>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let request = CsvWriteRequestDto { sheet, delimiter };
        let result = py.detach(move || self.lock_interop_wait()?.save_csv_bytes(&request));
        let bytes = result.map_err(|error| into_py_error(py, error))?;
        Ok(PyBytes::new(py, &bytes))
    }

    #[getter]
    pub fn closed(&self) -> bool {
        self.session.is_closed()
    }

    pub fn close(&self, py: Python<'_>) {
        py.detach(|| self.session.close());
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        if slf.closed() {
            return Err(into_py_error(slf.py(), InteropError::session_closed()));
        }
        Ok(slf)
    }

    fn __exit__(
        &self,
        py: Python<'_>,
        _exception_type: Option<&Bound<'_, PyAny>>,
        _exception: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) {
        self.close(py);
    }

    pub fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(|| self.lock_interop_wait().map(|session| session.summary()));
        let summary = result.map_err(|error| into_py_error(py, error))?;
        conversion::workbook_summary(py, &summary)
    }

    #[pyo3(signature = (
        sheet,
        start,
        end,
        *,
        offset=None,
        limit=None,
        omit_empty=false,
        values_only=false,
        compact=false,
        max_bytes=None
    ))]
    #[allow(clippy::too_many_arguments)]
    pub fn read_range<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        start: String,
        end: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
        omit_empty: bool,
        values_only: bool,
        compact: bool,
        max_bytes: Option<u64>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let offset = page_offset_from_python(offset).map_err(|error| into_py_error(py, error))?;
        let limit = page_limit_from_python(limit).map_err(|error| into_py_error(py, error))?;
        let options = RangeReadOptionsDto {
            omit_empty,
            values_only,
            compact,
            max_bytes,
        };
        let result = py.detach(move || {
            self.lock_interop_wait()?.read_range_with_options(
                &RangeRequestDto {
                    sheet,
                    start,
                    end,
                    offset,
                    limit,
                },
                &options,
            )
        });
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::range_page(py, &page)
    }

    #[pyo3(signature = (sheet, start, end, *, offset=None, limit=None))]
    pub fn read_formats<'py>(
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
            self.lock_interop_wait()?.read_formats(&RangeRequestDto {
                sheet,
                start,
                end,
                offset,
                limit,
            })
        });
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    #[pyo3(signature = (sheet, *, offset=None, limit=None))]
    pub fn data_validations<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request =
            sheet_page_request(sheet, offset, limit).map_err(|error| into_py_error(py, error))?;
        let result = py.detach(move || self.lock_interop_wait()?.data_validations(&request));
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    #[pyo3(signature = (sheet, *, offset=None, limit=None))]
    pub fn hyperlinks<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request =
            sheet_page_request(sheet, offset, limit).map_err(|error| into_py_error(py, error))?;
        let result = py.detach(move || self.lock_interop_wait()?.hyperlinks(&request));
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    #[pyo3(signature = (sheet, *, offset=None, limit=None))]
    pub fn comments<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request =
            sheet_page_request(sheet, offset, limit).map_err(|error| into_py_error(py, error))?;
        let result = py.detach(move || self.lock_interop_wait()?.comments(&request));
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    pub fn read_values<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        start: String,
        end: String,
    ) -> PyResult<Bound<'py, PyList>> {
        let request = RangeValuesRequestDto { sheet, start, end };
        let values = py
            .detach(move || self.lock_interop_wait()?.read_values(&request))
            .map_err(|error| into_py_error(py, error))?;
        conversion::range_value_rows(py, &values.rows)
    }

    pub fn write_values<'py>(
        &self,
        py: Python<'py>,
        sheet: &str,
        start: &str,
        rows: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let payload = PyDict::new(py);
        payload.set_item("sheet", sheet)?;
        payload.set_item("start", start)?;
        payload.set_item("rows", rows)?;
        let serialized = py
            .import("json")?
            .call_method1("dumps", (payload,))?
            .extract::<String>()?;
        let request =
            serde_json::from_str::<RangeWriteRequestDto>(&serialized).map_err(|error| {
                into_py_error(py, InteropError::invalid_change_payload(error.to_string()))
            })?;
        let receipt = py
            .detach(move || self.lock_interop_wait()?.write_values(request))
            .map_err(|error| into_py_error(py, error))?;
        conversion::edit_receipt(py, &receipt)
    }

    #[pyo3(signature = (sheet, address, *, offset=None, limit=None))]
    pub fn precedents<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        address: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request = dependency_request(sheet, address, offset, limit)
            .map_err(|error| into_py_error(py, error))?;
        let page = py
            .detach(move || self.lock_interop_wait()?.precedents(&request))
            .map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    #[pyo3(signature = (sheet, address, *, offset=None, limit=None))]
    pub fn dependents<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        address: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request = dependency_request(sheet, address, offset, limit)
            .map_err(|error| into_py_error(py, error))?;
        let page = py
            .detach(move || self.lock_interop_wait()?.dependents(&request))
            .map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    #[pyo3(signature = (sheet, *, offset=None, limit=None))]
    pub fn phonetics<'py>(
        &self,
        py: Python<'py>,
        sheet: String,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let request =
            sheet_page_request(sheet, offset, limit).map_err(|error| into_py_error(py, error))?;
        let result = py.detach(move || self.lock_interop_wait()?.phonetics(&request));
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::json_dict(py, &page)
    }

    #[pyo3(signature = (*, offset=None, limit=None, today_serial=None, now_serial=None))]
    pub fn scan_capabilities<'py>(
        &self,
        py: Python<'py>,
        offset: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
        today_serial: Option<&Bound<'_, PyAny>>,
        now_serial: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let offset = page_offset_from_python(offset).map_err(|error| into_py_error(py, error))?;
        let limit = page_limit_from_python(limit).map_err(|error| into_py_error(py, error))?;
        let options = CalculationOptionsDto {
            today_serial: optional_number_from_python(today_serial)
                .map_err(|error| into_py_error(py, error))?,
            now_serial: optional_number_from_python(now_serial)
                .map_err(|error| into_py_error(py, error))?,
            ..CalculationOptionsDto::default()
        };
        let result = py.detach(move || {
            self.lock_interop_wait()?
                .capabilities(options, offset, limit)
        });
        let page = result.map_err(|error| into_py_error(py, error))?;
        conversion::capability_page(py, &page)
    }

    #[pyo3(signature = (name, *, current_sheet=None))]
    pub fn inspect_defined_name<'py>(
        &self,
        py: Python<'py>,
        name: String,
        current_sheet: Option<String>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(move || {
            self.lock_interop_wait()?
                .inspect_defined_name(&DefinedNameInspectionRequestDto {
                    name,
                    current_sheet,
                })
        });
        let report = result.map_err(|error| into_py_error(py, error))?;
        conversion::defined_name_inspection(py, &report)
    }

    pub fn function_usage<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(|| {
            self.lock_interop_wait()
                .map(|session| session.function_usage())
        });
        let report = result.map_err(|error| into_py_error(py, error))?;
        conversion::function_usage(py, &report)
    }

    #[pyo3(signature = (
        *,
        today_serial=None,
        now_serial=None,
        random_seed=None,
        locale_profile=None,
        arithmetic_semantics="excel_near_zero",
        financial_solver_semantics="excel_iteration_budget",
        calculation_limits=None
    ))]
    // One parameter per Python keyword argument.
    #[allow(clippy::too_many_arguments)]
    pub fn calculate<'py>(
        &self,
        py: Python<'py>,
        today_serial: Option<&Bound<'_, PyAny>>,
        now_serial: Option<&Bound<'_, PyAny>>,
        random_seed: Option<&Bound<'_, PyAny>>,
        locale_profile: Option<&Bound<'_, PyAny>>,
        arithmetic_semantics: &str,
        financial_solver_semantics: &str,
        calculation_limits: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let calculation_limits =
            limits_from_python::<CalculationLimitOptionsDto>(py, calculation_limits)?;
        let today_serial =
            optional_number_from_python(today_serial).map_err(|error| into_py_error(py, error))?;
        let now_serial =
            optional_number_from_python(now_serial).map_err(|error| into_py_error(py, error))?;
        let random_seed =
            optional_seed_from_python(random_seed).map_err(|error| into_py_error(py, error))?;
        let locale_profile = optional_locale_profile_from_python(locale_profile)
            .map_err(|error| into_py_error(py, error))?;
        let arithmetic_semantics = parse_arithmetic_semantics(arithmetic_semantics)
            .map_err(|error| into_py_error(py, error))?;
        let financial_solver_semantics =
            parse_financial_solver_semantics(financial_solver_semantics)
                .map_err(|error| into_py_error(py, error))?;
        let prepared = py
            .detach(|| {
                self.lock_interop_wait()?.prepare_recalculation(
                    RecalculationModeDto::Auto,
                    CalculationOptionsDto {
                        today_serial,
                        now_serial,
                        random_seed,
                        locale_profile,
                        arithmetic_semantics,
                        financial_solver_semantics,
                        calculation_limits,
                    },
                )
            })
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
        random_seed=None,
        locale_profile=None,
        arithmetic_semantics="excel_near_zero",
        financial_solver_semantics="excel_iteration_budget",
        calculation_limits=None
    ))]
    // One parameter per Python keyword argument.
    #[allow(clippy::too_many_arguments)]
    pub fn recalculate<'py>(
        slf: PyRef<'py, Self>,
        mode: &str,
        today_serial: Option<&Bound<'_, PyAny>>,
        now_serial: Option<&Bound<'_, PyAny>>,
        random_seed: Option<&Bound<'_, PyAny>>,
        locale_profile: Option<&Bound<'_, PyAny>>,
        arithmetic_semantics: &str,
        financial_solver_semantics: &str,
        calculation_limits: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let py = slf.py();
        let workbook: &Self = &slf;
        let calculation_limits =
            limits_from_python::<CalculationLimitOptionsDto>(py, calculation_limits)?;
        let mode = parse_recalculation_mode(mode).map_err(|error| into_py_error(py, error))?;
        let today_serial =
            optional_number_from_python(today_serial).map_err(|error| into_py_error(py, error))?;
        let now_serial =
            optional_number_from_python(now_serial).map_err(|error| into_py_error(py, error))?;
        let random_seed =
            optional_seed_from_python(random_seed).map_err(|error| into_py_error(py, error))?;
        let locale_profile = optional_locale_profile_from_python(locale_profile)
            .map_err(|error| into_py_error(py, error))?;
        let arithmetic_semantics = parse_arithmetic_semantics(arithmetic_semantics)
            .map_err(|error| into_py_error(py, error))?;
        let financial_solver_semantics =
            parse_financial_solver_semantics(financial_solver_semantics)
                .map_err(|error| into_py_error(py, error))?;
        let prepared = py
            .detach(|| {
                workbook.lock_interop_wait()?.prepare_recalculation(
                    mode,
                    CalculationOptionsDto {
                        today_serial,
                        now_serial,
                        random_seed,
                        locale_profile,
                        arithmetic_semantics,
                        financial_solver_semantics,
                        calculation_limits,
                    },
                )
            })
            .map_err(|error| into_py_error(py, error))?;
        let request_id = prepared.request_id();
        let completed = match py.detach(move || prepared.run()) {
            Ok(completed) => completed,
            Err(error) => {
                let cleanup = py.detach(|| {
                    workbook
                        .lock_interop_wait()
                        .map(|mut session| session.abandon_recalculation(request_id))
                });
                return Err(into_py_error(py, cleanup.err().unwrap_or(error)));
            }
        };
        let delta = py
            .detach(move || {
                workbook
                    .lock_interop_wait()?
                    .install_recalculation(completed)
            })
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
        let receipt = py
            .detach(move || {
                self.lock_interop_wait()?
                    .apply_changes(expected_revision, batch)
            })
            .map_err(|error| into_py_error(py, error))?;
        conversion::edit_receipt(py, &receipt)
    }

    pub fn apply_changes_v2<'py>(
        &self,
        py: Python<'py>,
        expected_revision: &Bound<'_, PyAny>,
        changes: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let expected_revision =
            revision_from_python(expected_revision).map_err(|error| into_py_error(py, error))?;
        let batch = edit_batch_v2_from_python(py, changes)?;
        let receipt = py
            .detach(move || {
                self.lock_interop_wait()?
                    .apply_changes_v2(expected_revision, batch)
            })
            .map_err(|error| into_py_error(py, error))?;
        conversion::edit_receipt_v2(py, &receipt)
    }

    #[pyo3(signature = (expected_revision, changes, **options))]
    pub fn preview_changes<'py>(
        &self,
        py: Python<'py>,
        expected_revision: &Bound<'_, PyAny>,
        changes: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let expected_revision =
            revision_from_python(expected_revision).map_err(|error| into_py_error(py, error))?;
        let batch = edit_batch_v2_from_python(py, changes)?;
        let options = preview_options_from_python(py, options)?;
        let operation = self
            .session
            .cancellable_operation()
            .map_err(|error| into_py_error(py, error))?;
        let cancellation = operation.token().clone();
        let prepared = py.detach(move || {
            self.lock_interop_wait()?
                .prepare_preview_changes_cancellable(
                    expected_revision,
                    batch,
                    options.mode,
                    options.calculation_options,
                    &cancellation,
                )
        });
        let prepared = prepared.map_err(|error| into_py_error(py, error))?;
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
        let preview = match conversion::preview_changes(py, completed.summary()) {
            Ok(preview) => preview,
            Err(error) => {
                let cleanup = py.detach(|| {
                    self.lock_interop_wait()
                        .map(|mut session| session.abandon_recalculation(request_id))
                });
                return Err(cleanup
                    .err()
                    .map_or(error, |cleanup| into_py_error(py, cleanup)));
            }
        };
        py.detach(move || self.lock_interop_wait()?.publish_preview(completed))
            .map_err(|error| into_py_error(py, error))?;
        drop(operation);
        Ok(preview)
    }

    #[pyo3(signature = (preview_id, *, section, cursor=None, limit=None))]
    pub fn preview_changes_page<'py>(
        &self,
        py: Python<'py>,
        preview_id: &Bound<'_, PyAny>,
        section: &str,
        cursor: Option<&Bound<'_, PyAny>>,
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let preview_id =
            revision_from_python(preview_id).map_err(|error| into_py_error(py, error))?;
        let section = preview_section(section).map_err(|error| into_py_error(py, error))?;
        let cursor = preview_cursor_from_python(py, cursor)?;
        let limit = page_limit_from_python(limit).map_err(|error| into_py_error(py, error))?;
        let page = py.detach(move || {
            self.lock_interop_wait()?
                .preview_changes_page(preview_id, section, cursor, limit)
        });
        let page = page.map_err(|error| into_py_error(py, error))?;
        conversion::transaction_impact_page(py, &page)
    }

    pub fn commit_preview<'py>(
        &self,
        py: Python<'py>,
        preview_id: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let preview_id =
            revision_from_python(preview_id).map_err(|error| into_py_error(py, error))?;
        let receipt = py.detach(move || self.lock_interop_wait()?.commit_preview(preview_id));
        let receipt = receipt.map_err(|error| into_py_error(py, error))?;
        conversion::transaction_receipt(py, &receipt)
    }

    pub fn discard_preview(&self, py: Python<'_>, preview_id: &Bound<'_, PyAny>) -> PyResult<()> {
        let preview_id =
            revision_from_python(preview_id).map_err(|error| into_py_error(py, error))?;
        py.detach(move || self.lock_interop_wait()?.discard_preview(preview_id))
            .map_err(|error| into_py_error(py, error))
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
        let page = py
            .detach(move || self.lock_interop_wait()?.changes_since(cursor, limit))
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
        py.detach(|| {
            self.lock_interop_wait()?
                .set_formula(sheet, address, formula, dynamic_range)
        })
        .map_err(|error| into_py_error(py, error))
    }

    pub fn clear_cell(&self, py: Python<'_>, sheet: &str, address: &str) -> PyResult<bool> {
        py.detach(|| self.lock_interop_wait()?.clear_cell(sheet, address))
            .map_err(|error| into_py_error(py, error))
    }

    pub fn add_sheet(&self, py: Python<'_>, name: &str) -> PyResult<u32> {
        py.detach(|| self.lock_interop_wait()?.add_sheet(name))
            .map_err(|error| into_py_error(py, error))
    }

    pub fn rename_sheet(&self, py: Python<'_>, current_name: &str, new_name: &str) -> PyResult<()> {
        py.detach(|| {
            self.lock_interop_wait()?
                .rename_sheet(current_name, new_name)
        })
        .map_err(|error| into_py_error(py, error))
    }

    pub fn delete_sheet(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        py.detach(|| self.lock_interop_wait()?.delete_sheet(name))
            .map_err(|error| into_py_error(py, error))
    }

    #[pyo3(signature = (*, invalidate_unavailable=false))]
    pub fn to_bytes<'py>(
        &self,
        py: Python<'py>,
        invalidate_unavailable: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let result = py.detach(|| {
            self.lock_interop_wait()?.save_bytes(WriteOptionsDto {
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
            self.lock_interop_wait()?.save_path(
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

    pub fn edits_to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let result = py.detach(|| self.lock_interop_wait()?.save_edits_bytes());
        let (bytes, _) = result.map_err(|error| into_py_error(py, error))?;
        Ok(PyBytes::new(py, &bytes))
    }

    #[pyo3(signature = (path, *, replace_existing=false))]
    pub fn save_edits<'py>(
        &self,
        py: Python<'py>,
        path: PathBuf,
        replace_existing: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let result = py.detach(move || {
            self.lock_interop_wait()?
                .save_edits_path(path, EditWriteOptionsDto { replace_existing })
        });
        let report = result.map_err(|error| into_py_error(py, error))?;
        conversion::edit_write_report(py, &report)
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
        py.detach(move || self.lock_interop_wait()?.set_value(sheet, address, value))
            .map_err(|error| into_py_error(py, error))
    }

    /// Acquires the session without waiting, for calls that must not block behind other work.
    fn lock<'a>(&'a self, py: Python<'_>) -> PyResult<WorkbookSessionGuard<'a>> {
        self.session
            .try_lock()
            .map_err(|error| into_py_error(py, error))
    }

    fn lock_interop_wait(&self) -> Result<WorkbookSessionGuard<'_>, InteropError> {
        self.session.lock()
    }
}

struct PreviewOptionsInput {
    mode: RecalculationModeDto,
    calculation_options: CalculationOptionsDto,
}

fn dependency_request(
    sheet: String,
    address: String,
    offset: Option<&Bound<'_, PyAny>>,
    limit: Option<&Bound<'_, PyAny>>,
) -> Result<CellDependencyRequestDto, InteropError> {
    Ok(CellDependencyRequestDto {
        sheet,
        address,
        offset: page_offset_from_python(offset)?,
        limit: page_limit_from_python(limit)?,
    })
}

fn sheet_page_request(
    sheet: String,
    offset: Option<&Bound<'_, PyAny>>,
    limit: Option<&Bound<'_, PyAny>>,
) -> Result<SheetPageRequestDto, InteropError> {
    Ok(SheetPageRequestDto {
        sheet,
        offset: page_offset_from_python(offset)?,
        limit: page_limit_from_python(limit)?,
    })
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

/// Reads a locale profile: a `dict` with a `locale` tag and an optional `display_language` tag,
/// each `"en-US"` or `"ko-KR"`.
fn optional_locale_profile_from_python(
    value: Option<&Bound<'_, PyAny>>,
) -> Result<Option<LocaleProfileDto>, InteropError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let profile = value
        .cast::<PyDict>()
        .map_err(|_| InteropError::invalid_locale_profile())?;
    let mut locale = None;
    let mut display_language = None;
    for (key, tag) in profile.iter() {
        let tag = tag
            .extract::<String>()
            .map_err(|_| InteropError::invalid_locale_profile())?;
        let tag = LocaleDto::from_tag(&tag)?;
        match key.extract::<String>().as_deref() {
            Ok("locale") => locale = Some(tag),
            Ok("display_language") => display_language = Some(tag),
            _ => return Err(InteropError::invalid_locale_profile()),
        }
    }
    Ok(Some(LocaleProfileDto {
        locale: locale.ok_or_else(InteropError::invalid_locale_profile)?,
        display_language,
    }))
}

/// Reads a random seed: an `int` from 0 through 2^32 - 1, never a `bool` or `float`.
fn optional_seed_from_python(
    value: Option<&Bound<'_, PyAny>>,
) -> Result<Option<u32>, InteropError> {
    value
        .map(|value| {
            if !value.is_instance_of::<PyInt>() || value.is_instance_of::<PyBool>() {
                return Err(InteropError::invalid_number());
            }
            value
                .extract::<u32>()
                .map_err(|_| InteropError::invalid_number())
        })
        .transpose()
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
    let serialized = edit_batch_json_from_python(py, changes)?;
    serde_json::from_str(&serialized)
        .map_err(|error| into_py_error(py, InteropError::invalid_change_payload(error.to_string())))
}

fn edit_batch_v2_from_python(
    py: Python<'_>,
    changes: &Bound<'_, PyAny>,
) -> PyResult<EditBatchV2Dto> {
    let serialized = edit_batch_json_from_python(py, changes)?;
    serde_json::from_str(&serialized)
        .map_err(|error| into_py_error(py, InteropError::invalid_change_payload(error.to_string())))
}

fn preview_cursor_from_python(
    py: Python<'_>,
    value: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<PreviewCursorDto>> {
    let Some(value) = value else {
        return Ok(None);
    };
    // JSON would carry a sequence into the cursor by position.
    let cursor = value
        .cast::<PyDict>()
        .map_err(|_| into_py_error(py, InteropError::preview_cursor_invalid()))?;
    let serialized = py
        .import("json")?
        .call_method1("dumps", (cursor,))?
        .extract::<String>()?;
    serde_json::from_str(&serialized)
        .map(Some)
        .map_err(|_| into_py_error(py, InteropError::preview_cursor_invalid()))
}

fn preview_options_from_python(
    py: Python<'_>,
    options: Option<&Bound<'_, PyDict>>,
) -> PyResult<PreviewOptionsInput> {
    let Some(options) = options else {
        return Ok(PreviewOptionsInput {
            mode: RecalculationModeDto::Auto,
            calculation_options: CalculationOptionsDto::default(),
        });
    };
    const ALLOWED: &[&str] = &[
        "mode",
        "today_serial",
        "now_serial",
        "random_seed",
        "locale_profile",
        "arithmetic_semantics",
        "financial_solver_semantics",
        "calculation_limits",
    ];
    for key in options.keys().iter() {
        let key = key
            .extract::<String>()
            .map_err(|_| into_py_error(py, InteropError::invalid_change_payload(String::new())))?;
        if !ALLOWED.contains(&key.as_str()) {
            return Err(into_py_error(py, InteropError::invalid_change_payload(key)));
        }
    }
    let mode = option_text(
        py,
        options,
        "mode",
        "auto",
        InteropError::invalid_recalculation_mode,
    )?;
    let arithmetic_semantics = option_text(
        py,
        options,
        "arithmetic_semantics",
        "excel_near_zero",
        InteropError::invalid_arithmetic_semantics,
    )?;
    let financial_solver_semantics = option_text(
        py,
        options,
        "financial_solver_semantics",
        "excel_iteration_budget",
        InteropError::invalid_financial_solver_semantics,
    )?;
    let today_serial = options.get_item("today_serial")?;
    let now_serial = options.get_item("now_serial")?;
    let random_seed = options.get_item("random_seed")?;
    let locale_profile = options.get_item("locale_profile")?;
    let calculation_limits = options.get_item("calculation_limits")?;
    let calculation_limits = limits_from_python::<CalculationLimitOptionsDto>(
        py,
        calculation_limits.as_ref().filter(|value| !value.is_none()),
    )?;
    Ok(PreviewOptionsInput {
        mode: parse_recalculation_mode(&mode).map_err(|error| into_py_error(py, error))?,
        calculation_options: CalculationOptionsDto {
            today_serial: optional_number_from_python(
                today_serial.as_ref().filter(|value| !value.is_none()),
            )
            .map_err(|error| into_py_error(py, error))?,
            now_serial: optional_number_from_python(
                now_serial.as_ref().filter(|value| !value.is_none()),
            )
            .map_err(|error| into_py_error(py, error))?,
            random_seed: optional_seed_from_python(
                random_seed.as_ref().filter(|value| !value.is_none()),
            )
            .map_err(|error| into_py_error(py, error))?,
            locale_profile: optional_locale_profile_from_python(
                locale_profile.as_ref().filter(|value| !value.is_none()),
            )
            .map_err(|error| into_py_error(py, error))?,
            arithmetic_semantics: parse_arithmetic_semantics(&arithmetic_semantics)
                .map_err(|error| into_py_error(py, error))?,
            financial_solver_semantics: parse_financial_solver_semantics(
                &financial_solver_semantics,
            )
            .map_err(|error| into_py_error(py, error))?,
            calculation_limits,
        },
    })
}

/// Reads a limit mapping as the interop limit object, so another object, unknown names, and
/// values that are not positive integers fail as one stable input error. `None` keeps every
/// default.
fn limits_from_python<T: serde::de::DeserializeOwned + Default>(
    py: Python<'_>,
    value: Option<&Bound<'_, PyAny>>,
) -> PyResult<T> {
    let Some(value) = value else {
        return Ok(T::default());
    };
    // JSON would carry a sequence into the limits by position and a `None` limit as omitted.
    let limits = value.cast::<PyDict>().map_err(|error| {
        into_py_error(py, InteropError::invalid_limit_payload(error.to_string()))
    })?;
    if let Some((name, _)) = limits.iter().find(|(_, limit)| limit.is_none()) {
        return Err(into_py_error(
            py,
            InteropError::invalid_limit_payload(name.to_string()),
        ));
    }
    let json = py
        .import("json")?
        .call_method1("dumps", (limits,))
        .and_then(|dumped| dumped.extract::<String>())
        .map_err(|error| {
            into_py_error(py, InteropError::invalid_limit_payload(error.to_string()))
        })?;
    serde_json::from_str(&json)
        .map_err(|error| into_py_error(py, InteropError::invalid_limit_payload(error.to_string())))
}

fn option_text(
    py: Python<'_>,
    options: &Bound<'_, PyDict>,
    key: &str,
    default: &str,
    invalid: fn() -> InteropError,
) -> PyResult<String> {
    options
        .get_item(key)?
        .map(|value| {
            value
                .extract::<String>()
                .map_err(|_| into_py_error(py, invalid()))
        })
        .transpose()
        .map(|value| value.unwrap_or_else(|| default.to_owned()))
}

fn preview_section(value: &str) -> Result<TransactionDetailSectionDto, InteropError> {
    match value {
        "affected" => Ok(TransactionDetailSectionDto::Affected),
        "evaluated" => Ok(TransactionDetailSectionDto::Evaluated),
        "preview_results" => Ok(TransactionDetailSectionDto::PreviewResults),
        "preview_issues" => Ok(TransactionDetailSectionDto::PreviewIssues),
        "install_results" => Ok(TransactionDetailSectionDto::InstallResults),
        _ => Err(InteropError::invalid_preview_section()),
    }
}

fn edit_batch_json_from_python(py: Python<'_>, changes: &Bound<'_, PyAny>) -> PyResult<String> {
    let payload = PyDict::new(py);
    payload.set_item("changes", changes)?;
    py.import("json")?
        .call_method1("dumps", (payload,))?
        .extract::<String>()
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
