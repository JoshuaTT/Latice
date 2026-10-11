# engine-wasm/ — WASM build (Rust crate: `engine-wasm`)

## Purpose

Compiles the Lattice Engine to WebAssembly for the browser spectator viewer.
The viewer replays matches by running the same deterministic engine in the
browser.

## Owned contracts

None yet.

## Local commands

```bash
cargo build -p engine-wasm
cargo test  -p engine-wasm
cargo fmt   -- -p engine-wasm --check
cargo clippy -p engine-wasm --all-targets --all-features -- -D warnings
```

For a real WASM target build (M1+):

```bash
cargo build -p engine-wasm --target wasm32-unknown-unknown
```

## Acceptance tests

- Compiles for the host target as a `cdylib`.
- Re-exports `lattice-engine` so the WASM entry point has access to all engine
  types.
- A `wasm32` CI build job is out of scope for M0 but the crate structure and
  dependency on `lattice-engine` must be correct.

## What not to touch

- `spec/`, `.github/`, `tests/golden/`, `infra/` — code-owned paths.
- This crate re-exports `lattice-engine`; no game logic lives here.
