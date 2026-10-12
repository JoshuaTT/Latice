# engine/ — Lattice Engine (Rust crate: `lattice-engine`)

## Purpose

The deterministic simulation core of LATTICE. This crate owns the world model,
tick scheduler, LatticePy virtual machine, code checker, game library (stdlib),
map generator, ghost builder, and telemetry sharder.

## Owned contracts

The engine implements these; change the contract first (in a spec PR), then the code.

| Contract | Implemented by |
|---|---|
| `spec/contracts/engine-api.md` | `game` (Match, phases), `checker` (`check`), `events` (sinks), `snapshot`, `error` |
| `spec/contracts/state-hash.md` | `world/hash.rs` (format version 1) |
| `spec/contracts/events.schema.json` | `events` (event shapes and sequence numbers; emission by the resolvers, #53–#55) |
| `spec/contracts/ruleset.schema.json` | `ruleset` (loader; rejects unknown and missing keys) |

## Module map

On `main` today (`engine/src/`):

- `rng` — ChaCha RNG, one stream per (match seed, stream id).
- `ruleset` — ruleset loader and board-scaled `derived(side)` values.
- `world/` — `grid` (chunked, copy-on-write cells and marks), `entity` (drones, structures, Cores, Relics), `hash` (state hash), `ascii` (renderer). `World::check_invariants` verifies occupancy, bounds and terrain.
- `tick/` — scheduler: parallel think, serial resolve, `Resolver` and `DroneProgram` traits.
- `checker/` — LatticePy checker and node counter (**code-owned**); lowers into its own allow-list AST.
- `game` — `Match` phase machine. `events` — event types and sinks. `snapshot` — in-memory snapshot and restore. `error` — `EngineError`.

Planned (stub or absent): `vm` (#47, **code-owned**), `stdlib` (#56), `mapgen` (#50, #110), `ghost`, `shard`; resolvers for movement, combat and building (#53–#55); radio delivery (#103, blocked on a spec decision).

## Invariants every PR must keep

- **Determinism lints** (see below) stay on. A justified exception is a scoped `#[allow(clippy::disallowed_...)]` with the reason in a comment and in the PR (the benches and one checker timing test are the existing examples).
- **Entity positions change only through `World::move_entity`.** There is no `&mut Entity`; Forge, Relic and Core terrain is owned by their entities. Tests of world-changing code should end with `check_invariants()`.
- **New hashed state is added to `state-hash.md` first** (a spec PR, new format version if the encoding changes), then to `world/hash.rs`.
- **`think_threads` must never change a result**; think output is sorted by entity id before resolve.
- **Checker diagnostic codes are stable and append-only** (engine-api §1); the table is in the `checker` module docs.

## Local commands

```bash
cargo build -p lattice-engine
cargo test  -p lattice-engine
cargo fmt   -- -p lattice-engine --check
cargo clippy -p lattice-engine --all-targets --all-features -- -D warnings
```

Benchmarks (`engine/benches/budget.rs`, a custom `harness = false` runner):

```bash
cargo bench -p lattice-engine -- --budget            # PR-sized cases; exits non-zero over budget
cargo bench -p lattice-engine -- --budget --full     # adds worst-case cases (none yet: 1024² clash is M2)
cargo bench -p lattice-engine -- --budget --budget-scale 0.0001   # proves failure: shrinks every budget
```

Budgets live in one table in that file, each citing its spec line. Add a case
there when a new piece of code has a performance budget. The library's own
bench harness is disabled (`[lib] bench = false`) because it rejects `--budget`.

Sandbox (checker side; the VM half arrives with #47/#48):

```bash
cargo test -p lattice-engine --test sandbox          # static escape suite: real source, expected E_* codes
cd engine/fuzz && cargo +nightly fuzz run checker    # needs nightly and `cargo install cargo-fuzz`
cd engine/fuzz && cargo +nightly fuzz build          # what a nightly CI job runs
```

`engine/fuzz/` is its own workspace (stable CI never builds it). Its `checker`
target feeds arbitrary bytes to `check_bytes` and asserts the engine-API
invariants; seed inputs are in `engine/fuzz/corpus/checker/`. When a case in
`tests/sandbox.rs` is added, also add its source to that corpus.

## Acceptance tests

- `cargo build` and `cargo test` must pass.
- Module names follow `spec/system-design.md` §2.1: `world`, `tick`, `vm`,
  `checker`, `stdlib`, `mapgen`, `ghost`, `shard`. Shared support modules sit
  beside them (`rng`, `ruleset`, `game`, `events`, `snapshot`, `error`).
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

- `src/vm/` and `src/checker/` — the sandbox boundary; code-owned, so PRs wait for Josh and need escape tests (root `AGENTS.md` §11).
- `spec/` — spec changes need a separate PR and human approval.
- `.github/` — CI config is code-owned.
- `tests/golden/` — golden hashes need human approval.
- `infra/` — infrastructure code.
