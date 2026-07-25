use cellrune_interop::{InteropError, InteropErrorKind};
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;

create_exception!(
    cellrune,
    CellRuneError,
    PyException,
    "A typed CellRune read, validation, state, calculation, or write error."
);

pub(crate) fn into_py_error(py: Python<'_>, error: InteropError) -> PyErr {
    let py_error = CellRuneError::new_err(error.message().to_owned());
    let value = py_error.value(py);
    let details = PyDict::new(py);
    let _ = details.set_item("source_code", error.details().source_code.as_deref());
    let _ = details.set_item("source_id", error.details().source_id.as_deref());
    let _ = details.set_item("detail", error.details().detail.as_deref());
    let _ = value.setattr("code", error.code());
    let _ = value.setattr("kind", error_kind(error.kind()));
    let _ = value.setattr("details", details);
    py_error
}

const fn error_kind(kind: InteropErrorKind) -> &'static str {
    match kind {
        InteropErrorKind::Input => "input",
        InteropErrorKind::Validation => "validation",
        InteropErrorKind::Read => "read",
        InteropErrorKind::Write => "write",
        InteropErrorKind::State => "state",
    }
}
