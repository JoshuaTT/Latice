# cli/ — Lattice CLI (Rust crate: `lattice-cli`)

## Purpose

The `lattice` command-line tool for running simulations, checking LatticePy
code, generating maps, and replaying golden matches. The binary is the primary
way to drive the engine from scripts, CI, and during development.

## Owned contracts

None yet.

## Local commands

```bash
cargo build -p lattice-cli
cargo test  -p lattice-cli
cargo fmt   -- -p lattice-cli --check
cargo clippy -p lattice-cli --all-targets --all-features -- -D warnings
cargo run   -p lattice-cli -- <subcommand>
```

Verify the determinism harness:

```bash
cargo run --release -p lattice-cli -- replay --verify tests/golden
```

## Acceptance tests

- `replay --verify <dir>` (logic in `src/replay.rs`) discovers matches as
  immediate subdirectories of `<dir>` containing `match.json`, in sorted order.
  - No matches (e.g. only `README.md`): prints a notice and exits 0. The CI
    `determinism` job relies on this until M1 adds fixtures.
  - `<dir>` missing or not a directory: error, non-zero exit.
  - Any match: validates `match.json` (`match_id` = directory name) and
    `expected/hashes.json` (≥1 snapshot), then exits non-zero because engine
    replay is not implemented yet. Never report success for an unverified
    fixture.
  - `--target-parity wasm` with any match: non-zero until the WASM runner exists.
- All subcommands (`sim`, `check`, `mapgen`, `replay`) must accept their
  documented flags and exit 0 with empty/stub behavior.
- The binary is named `lattice` (the crate provides `[[bin]] name = "lattice"`).

## What not to touch

- `spec/`, `.github/`, `tests/golden/`, `infra/` — code-owned paths.
- No game logic lives here. This crate calls into `lattice-engine`.
