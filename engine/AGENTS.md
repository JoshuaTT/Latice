# engine/ — Lattice Engine (Rust crate: `lattice-engine`)

## Purpose

The deterministic simulation core of LATTICE. This crate owns the world model,
tick scheduler, LatticePy virtual machine, code checker, game library (stdlib),
map generator, ghost builder, and telemetry sharder.

## Owned contracts

None yet. Contracts will land in `spec/contracts/` during M0/M1.

## Local commands

```bash
cargo build -p lattice-engine
cargo test  -p lattice-engine
cargo fmt   -- -p lattice-engine --check
cargo clippy -p lattice-engine --all-targets --all-features -- -D warnings
```

When benchmarks exist (M1+):

```bash
cargo bench -p lattice-engine -- --budget
```

## Acceptance tests

- `cargo build` and `cargo test` must pass.
- The module stubs must exactly match the names listed in
  `spec/system-design.md` §2.1: `world`, `tick`, `vm`, `checker`, `stdlib`,
  `mapgen`, `ghost`, `shard`. Shared support modules sit beside them
  (`ruleset`).
- All game parameters must be read from rulesets, never hard-coded. Load them
  with `ruleset::Ruleset::from_toml_str` (it rejects unknown and missing keys)
  and compute board-scaled values with `Ruleset::derived(side)`. Game logic
  takes the string or the struct, never a path.
- Determinism rules (root `AGENTS.md` §8) apply to all logic under this crate:
  no unordered iteration, no host floats, no ambient inputs, seeded RNG only,
  deterministic parallelism.

## Determinism lints

`engine/clippy.toml` bans `HashMap`/`HashSet` (std and hashbrown), `Instant`,
`SystemTime`, `thread_rng`/`rand::random`, `getrandom`, `std::env::var(s)`,
`thread::current`, and the `std::fs` read entry points. `lib.rs` denies host
float arithmetic and comparison (`clippy::float_arithmetic`, `float_cmp`,
`cast_precision_loss`). The only exception is `Ruleset::from_path`. Any new
`#[allow(clippy::disallowed_...)]` needs a reason in the code and in the PR.

## What not to touch

- `spec/` — spec changes need a separate PR and human approval.
- `.github/` — CI config is code-owned.
- `tests/golden/` — golden hashes need human approval.
- `infra/` — infrastructure code.
