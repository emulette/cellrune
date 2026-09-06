use crate::error::into_py_error;
use cellrune_interop::{InteropError, TargetCalculationRequestDto};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};

pub(crate) fn request_from_python(
    py: Python<'_>,
    targets: &Bound<'_, PyAny>,
    options: Option<&Bound<'_, PyDict>>,
) -> PyResult<TargetCalculationRequestDto> {
    let payload = PyDict::new(py);
    payload.set_item("targets", targets)?;
    let calculation_options = PyDict::new(py);
    if let Some(options) = options {
        for (key, value) in options.iter() {
            if key.extract::<String>()? == "limits" {
                if !value.is_none() {
                    payload.set_item("limits", value)?;
                }
            } else {
                calculation_options.set_item(key, value)?;
            }
        }
    }
    payload.set_item("options", calculation_options)?;
    let serialized: String = py
        .import("json")?
        .call_method1("dumps", (payload,))
        .and_then(|value| value.extract())
        .map_err(|error| {
            into_py_error(py, InteropError::invalid_target_payload(error.to_string()))
        })?;
    serde_json::from_str::<TargetCalculationRequestDto>(&serialized)
        .map_err(|error| into_py_error(py, InteropError::invalid_target_payload(error.to_string())))
}
