#![forbid(unsafe_code)]

mod conversion;
mod error;
mod workbook;

use pyo3::prelude::*;

use crate::error::CellRuneError;
use crate::workbook::Workbook;

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("CellRuneError", module.py().get_type::<CellRuneError>())?;
    module.add_class::<Workbook>()?;
    module.add("SCHEMA_VERSION", cellrune_interop::INTEROP_SCHEMA_VERSION)?;
    Ok(())
}
