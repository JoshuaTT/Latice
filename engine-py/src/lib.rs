use pyo3::prelude::*;

/// Returns the engine version as a string.
#[pyfunction]
fn engine_version() -> String {
    "0.1.0".to_string()
}

/// Python bindings for the Lattice Engine.
#[pymodule]
fn engine_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(engine_version, m)?)?;
    Ok(())
}
