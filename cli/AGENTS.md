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
- `check [--ruleset PATH] [--json] [--library [--quota N]] <file.py>...`
  (logic in `src/check.rs`) runs the engine checker on each file.
  - Text output: `path:line:col: severity code: message` per diagnostic, then
    `path: ok (N nodes)` or `path: N errors`. `--json` prints one array of
    `{path, ok, node_count, interface, diagnostics}` objects in argument order.
  - `--library` treats the files as one team library: it sums `node_count`,
    prints `library: N / LIMIT nodes`, and fails if N exceeds the ruleset's
    `team_complexity_nodes` (or `--quota N`, which needs `--library`). Without
    `--library` there is no quota check.
  - Exit 0 only if every file passes and the quota holds. A missing file is an
    error on stderr and a non-zero exit. The default ruleset path is
    `spec/rulesets/default.toml` relative to the working directory.
- `sim`, `mapgen` and bare `replay` accept their documented flags and exit 0
  with stub behavior until their issues land.
- The binary is named `lattice` (the crate provides `[[bin]] name = "lattice"`).

## What not to touch

- `spec/`, `.github/`, `tests/golden/`, `infra/` — code-owned paths.
- No game logic lives here. This crate calls into `lattice-engine`.
