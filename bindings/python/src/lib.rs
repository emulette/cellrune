#![forbid(unsafe_code)]

mod conversion;
mod error;
mod targeted;
mod workbook;

use pyo3::prelude::*;

use cellrune_interop::InteropError;
use pyo3::types::PyDict;

use crate::error::{CellRuneError, into_py_error};
use crate::workbook::Workbook;

/// Returns the text a cell holding `value` shows with the number-format code `format_code`.
#[pyfunction]
#[pyo3(signature = (value, format_code, *, date_system="excel_1900", locale_profile=None))]
fn format_display_text(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    format_code: &str,
    date_system: &str,
    locale_profile: Option<&Bound<'_, PyAny>>,
) -> PyResult<String> {
    let invalid =
        |error: String| into_py_error(py, InteropError::invalid_display_format_request(error));
    let request = PyDict::new(py);
    request.set_item("value", value)?;
    request.set_item("format_code", format_code)?;
    request.set_item("date_system", date_system)?;
    if let Some(locale_profile) = locale_profile.filter(|value| !value.is_none()) {
        // The profile fails with the code every other Python call reports for it.
        workbook::optional_locale_profile_from_python(Some(locale_profile))
            .map_err(|error| into_py_error(py, error))?;
        request.set_item("locale_profile", locale_profile)?;
    }
    let json = py
        .import("json")?
        .call_method1("dumps", (request,))
        .and_then(|dumped| dumped.extract::<String>())
        .map_err(|error| invalid(error.to_string()))?;
    let request = serde_json::from_str(&json).map_err(|error| invalid(error.to_string()))?;
    cellrune_interop::format_display_text(request)
        .map(|result| result.text)
        .map_err(|error| into_py_error(py, error))
}

#[pyfunction]
fn function_catalog(py: Python<'_>) -> PyResult<Bound<'_, pyo3::types::PyDict>> {
    conversion::function_catalog(py, &cellrune_interop::function_catalog())
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("CellRuneError", module.py().get_type::<CellRuneError>())?;
    module.add_class::<Workbook>()?;
    module.add_function(wrap_pyfunction!(function_catalog, module)?)?;
    module.add_function(wrap_pyfunction!(format_display_text, module)?)?;
    module.add("SCHEMA_VERSION", cellrune_interop::INTEROP_SCHEMA_VERSION)?;
    Ok(())
}
