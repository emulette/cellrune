use crate::error::into_py_error;
use cellrune_interop::{
    ArithmeticSemanticsDto, CalculationLimitOptionsDto, CalculationOptionsDto,
    CalculationTargetDto, FinancialSolverSemanticsDto, InteropError, TargetCalculationLimitsDto,
    TargetCalculationRequestDto,
};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use serde::Deserialize;

/// A target calculation request as the keyword arguments carry it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRequestInput {
    targets: Vec<CalculationTargetDto>,
    #[serde(default)]
    options: TargetOptionsInput,
    #[serde(default)]
    limits: TargetCalculationLimitsDto,
}

/// Deterministic inputs for a target calculation. Targeted calculation keeps the default
/// calculation limits, so `calculation_limits` fails like any other unknown option.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetOptionsInput {
    today_serial: Option<f64>,
    now_serial: Option<f64>,
    #[serde(default)]
    arithmetic_semantics: ArithmeticSemanticsDto,
    #[serde(default)]
    financial_solver_semantics: FinancialSolverSemanticsDto,
}

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
    let request = serde_json::from_str::<TargetRequestInput>(&serialized).map_err(|error| {
        into_py_error(py, InteropError::invalid_target_payload(error.to_string()))
    })?;
    Ok(TargetCalculationRequestDto {
        targets: request.targets,
        options: CalculationOptionsDto {
            today_serial: request.options.today_serial,
            now_serial: request.options.now_serial,
            arithmetic_semantics: request.options.arithmetic_semantics,
            financial_solver_semantics: request.options.financial_solver_semantics,
            calculation_limits: CalculationLimitOptionsDto::default(),
        },
        limits: request.limits,
    })
}
