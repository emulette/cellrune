#![forbid(unsafe_code)]

mod conversion;
mod error;
mod workbook;

use pyo3::prelude::*;

use crate::error::CellRuneError;
use crate::workbook::Workbook;

#[pyfunction]
fn function_catalog(py: Python<'_>) -> PyResult<Bound<'_, pyo3::types::PyDict>> {
    conversion::function_catalog(py, &cellrune_interop::function_catalog())
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("CellRuneError", module.py().get_type::<CellRuneError>())?;
    module.add_class::<Workbook>()?;
    module.add_function(wrap_pyfunction!(function_catalog, module)?)?;
    module.add("SCHEMA_VERSION", cellrune_interop::INTEROP_SCHEMA_VERSION)?;
    Ok(())
}
