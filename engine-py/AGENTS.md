# engine-py/ — Python bindings (Rust crate: `engine-py`)

## Purpose

PyO3 bindings that expose the Lattice Engine to the Python match orchestrator.
The orchestrator calls into Rust for simulation ticks, code checking, map
generation, and replay.

## Owned contracts

None yet.

## Local commands

```bash
cargo build -p engine-py
cargo test  -p engine-py
cargo fmt   -- -p engine-py --check
cargo clippy -p engine-py --all-targets --all-features -- -D warnings
```

Build requires `libpython3.12.so` available on the linker path. On CI this is
provided by the runner image. During local development, set `RUSTFLAGS` and
`LIBRARY_PATH` if the dev headers are not installed.

## Acceptance tests

- Compiles with `abi3-py312` feature, producing a `cdylib` and `lib`.
- No `unsafe` blocks (the initial stub `#[pyfunction]` is safe).
- The module is named `engine_py` (Python import name).

## What not to touch

- `spec/`, `.github/`, `tests/golden/`, `infra/` — code-owned paths.
- This crate wraps `lattice-engine`; no game logic lives here.
