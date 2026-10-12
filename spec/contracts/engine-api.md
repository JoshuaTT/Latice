# Engine API contract

**Crate:** `lattice-engine`. **Status:** proposed for M1; see [Stability](#stability).
**Consumers:** `lattice-cli` (`sim`, `check`, `replay`), `engine-py` (PyO3, M3), `engine-wasm` (browser viewer, M5), and the orchestrator's replay service.

The signatures below are a contract, not compiled code. They fix names, ownership, error behaviour and determinism obligations so the tick, VM, checker and CLI work doesn't invent three interfaces. Anything not listed here is private to the crate. Per AGENTS.md §2.3, an interface change in code needs a change here first.

Sources: system-design §2.1 (modules), §2.4 (replay rebuilds any tick from seed + manifests + snapshots), §10.1 (two-phase ticks), §11.1; the schemas `ruleset.schema.json`, `manifest.schema.json` and `events.schema.json`; AGENTS.md §8 (determinism) and §9 (`thiserror`, no `unwrap` on fallible paths).

## Conventions

- All fallible functions return `Result<_, EngineError>` (or a more specific error that converts into it). No panics on bad input; bad *player code* is never an `Err` (see `step_tick`).
- No function reads the clock, environment, file system or network, and none uses a host float. Anything that needs a path (rulesets, maps) is loaded by the caller and passed in as a `&str` or a value.
- Every type crossing the API that holds game state is plain data: `Clone`, `Debug`, `Eq` where it has no floats. Where LatticePy floats appear they use the engine's software float `F` (#44), never `f32`/`f64`.
- Positions are `Pos { x: u16, y: u16 }`; teams are `Team { A, B, Neutral }`; names match `events.schema.json`.

## 1. Checking code

```rust
pub fn check(source: &str, ruleset: &Ruleset) -> CheckReport;

pub struct CheckReport {
    pub ok: bool,                       // true iff no Error diagnostics
    pub diagnostics: Vec<Diagnostic>,   // sorted by (line, col, code)
    pub node_count: u32,                // AST nodes counted for the quota (game-design §9.4)
    pub interface: ModuleInterface,     // which of init / tick / handlers are defined
}

pub struct Diagnostic {
    pub severity: Severity,             // Error | Warning
    pub code: &'static str,             // stable, e.g. "E_IMPORT", "W_UNUSED"
    pub line: u32,                      // 1-based
    pub col: u32,                       // 1-based
    pub message: String,                // written for an LLM reader: what, where, how to fix
}
```

`check` is a pure function of its arguments. It parses, validates the LatticePy subset, counts nodes and checks the module interface. It never executes code and never fails: a syntax error is an `Error` diagnostic with `ok == false`. Output order is fixed so identical input gives identical bytes. Budget: under 50 ms per module (system-design §2.1). The quota limit itself (`team_complexity_nodes`) is applied by the caller summing `node_count` across a library, because the quota is per team, not per module.

## 2. Building a match

```rust
pub struct MatchConfig {
    pub match_id: String,               // opaque; copied into events; NOT part of the state hash
    pub seed: u64,                      // match seed for every RNG stream (rng.rs `StreamId`)
    pub ruleset: Ruleset,
    pub map: Map,                       // placeholder until M2 mapgen: see Stability
    pub teams: [TeamDeployment; 2],     // index 0 = Team A, 1 = Team B
    pub options: EngineOptions,
}

pub struct EngineOptions {
    pub think_threads: usize,           // 0 = engine default; MUST NOT change any result
}

pub struct TeamDeployment {
    pub library: Vec<Module>,           // module sources available this epoch
    pub manifest: Manifest,
}

pub struct Module { pub name: String, pub source: String }   // name matches ^[a-z_][a-z0-9_]*$

/// Mirrors manifest.schema.json after TOML → JSON parsing.
pub struct Manifest {
    pub library: Vec<String>,                    // module names
    pub spawn: Vec<SpawnSpec>,                   // module, forge, offset: Option<[i32; 2]>, args
    pub board: BoardConfig,                      // init: BTreeMap<String, Json>, reserved: Vec<String>
}

pub fn Match::new(config: MatchConfig) -> Result<Match, EngineError>;
```

`Json` is the engine's ordered JSON value (`BTreeMap` objects, integers as `i64`, non-integers as `F`). `Match::new` consumes the validated inputs and returns a match positioned at the start of epoch 1, in phase `Deployed` (see §3). It performs the runtime checks that `manifest.schema.json` documents as "not enforced by this schema": every `spawn.module` is in `library` and in `TeamDeployment::library`; `spawn.forge` names a Forge the team owns; `spawn.offset` is within the Forge radius from the ruleset; `board.init` and `board.reserved` limits and uniqueness. It runs `check` on every module and rejects any with `ok == false` or a team total over the quota.

Determinism: the same `MatchConfig` (ignoring `match_id` and `options`) always yields the same initial state hash. Canonical manifest hashing (RFC 8785 + SHA-256) belongs to the manifest service, not the engine; the engine takes the already-parsed value.

## 3. Stepping

```rust
pub enum Phase { Deployed, InClash, EpochOver, MatchOver }

impl Match {
    pub fn phase(&self) -> Phase;
    pub fn epoch(&self) -> u32;         // 1-based, as in events.schema.json (1..=epochs_per_match)
    pub fn tick(&self) -> u64;          // 0-based tick within the current epoch's clash

    /// One think + resolve cycle. Only valid in Deployed or InClash.
    pub fn step_tick(&mut self, sink: &mut dyn EventSink) -> Result<TickOutcome, EngineError>;

    /// Steps until the clash ends (Core destroyed or captured, or the scaled
    /// clash length is reached). Equivalent to looping `step_tick`.
    pub fn run_clash(&mut self, sink: &mut dyn EventSink) -> Result<ClashResult, EngineError>;

    /// Between epochs: swaps in new deployments, then starts the next clash setup.
    /// `None` keeps that team's previous deployment. Valid only in EpochOver.
    pub fn begin_epoch(&mut self, next: [Option<TeamDeployment>; 2]) -> Result<(), EngineError>;
}

pub struct TickOutcome { pub clash_over: bool }

pub struct ClashResult {
    pub epoch: u32,
    pub ticks_run: u64,
    pub end: ClashEnd,                  // CoreDestroyed(Team) | CoreCaptured(Team) | LengthReached
    pub state_hash: [u8; 32],           // hash at the final tick
    pub scores: [u64; 2],               // placeholder breakdown until the scoring spec lands
}
```

**Two phases per tick** (system-design §10.1). *Think*: every drone's module runs against a read-only view of the state at the start of the tick, in parallel; each run is fuel-capped and produces an ordered list of acts. *Resolve*: acts apply serially in the order fixed by `events.schema.json` (action-type priority, then the rotating seat order that advances each epoch). Parallelism may change timing but never results, and `think_threads` must not appear in any hash.

**Player-code failure is not an engine error.** A drone that runs out of fuel, raises, or exceeds a memory or recursion cap simply stops for that tick (game-design §4.2); the engine records it as drone state and continues. `step_tick` returns `Err` only for host-level problems (wrong phase, internal invariant broken).

**Epochs.** `run_clash` ends in `EpochOver`, or `MatchOver` after the final epoch. `begin_epoch` is the only place new manifests and libraries enter; it re-validates them exactly as `Match::new` does, applies board `init` values, resets per-epoch clash state (tick to 0, rotates seat order), and returns the match to `Deployed` with `epoch()` incremented. Energy pools and territory persist across epochs (game-design §4.2). Calling any step in the wrong phase is `EngineError::WrongPhase`.

## 4. Observing

```rust
pub trait EventSink {
    fn emit(&mut self, event: &Event);  // infallible; MUST NOT affect the simulation
}

pub struct Event {
    pub match_id: String,
    pub epoch: u32,
    pub tick: u64,
    pub seq: u64,                       // per match, in resolve order
    pub kind: EventKind,                // one variant per event_type in events.schema.json
}
// Event::to_json(&self) -> Json yields exactly the events.schema.json shape.

pub struct NullSink;                    // discards events
pub struct VecSink(pub Vec<Event>);     // collects them

impl Match {
    pub fn state_hash(&self) -> [u8; 32];
    pub fn snapshot(&self) -> Snapshot;
    pub fn restore(snapshot: &Snapshot, ruleset: Ruleset) -> Result<Match, EngineError>;
}
```

- **Events** are emitted in `seq` order; `seq` is assigned by the engine and gaps are not allowed. A sink receives a borrowed event and cannot reject it, so sinks cannot change results. `match_id` comes from the config and carries no simulation meaning.
- **`state_hash`** hashes the canonical serialization defined by #40 (including RNG stream positions, per-drone VM state and event `seq`). It excludes `match_id`, `EngineOptions` and anything derived from wall-clock time. It's cheap enough to call every tick for debugging, and `lattice replay --verify` calls it at the recorded ticks.
- **`snapshot` / `restore`** capture everything needed to continue: world, per-drone VM state, RNG states, `seq`, tick, epoch, phase and both deployments. `Snapshot` is `Clone + Send + Sync` and uses copy-on-write chunks, so taking one is cheap. The ruleset is passed to `restore` rather than embedded; `restore` fails with `EngineError::RulesetMismatch` if its version differs from the one recorded. **Obligation:** for any match `m`, restoring `m.snapshot()` and then applying the same steps as `m` produces byte-identical events and hashes as the original run. Byte encoding of a `Snapshot` (`to_bytes` / `from_bytes`) is specified in #40.
- Replay (system-design §2.4) is `Match::new` plus recorded `begin_epoch` calls, optionally starting from a periodic snapshot.

## 5. Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    Ruleset(#[from] RulesetError),      // parse/validation, side too small
    Module { name: String, report: CheckReport },    // a library module failed check
    Quota { team: Team, used: u32, limit: u32 },
    Manifest(ManifestError),            // runtime manifest checks from §2
    Map(MapError),                      // placeholder until M2
    WrongPhase { expected: &'static str, found: Phase },
    RulesetMismatch { expected: String, found: String },
    Snapshot(SnapshotError),            // corrupt or incompatible snapshot (#40)
    Internal(&'static str),             // broken invariant; a bug, never player-caused
}
```

Variants and their payload fields are stable for M1; sub-error enums (`ManifestError`, `MapError`, `SnapshotError`) are each `thiserror` enums owned by their module. Error `Display` strings are for humans and are not part of the contract; match on the variant.

## 6. Threading, `Send`/`Sync`, and floats

| Type | Obligation |
|---|---|
| `Match` | `Send`. Driven by one thread; the engine spawns its own think-phase workers internally. |
| `Snapshot`, `Ruleset`, `CheckReport`, `Event`, `Manifest`, `TeamDeployment`, `Json` | `Send + Sync`, immutable once built. |
| Read-only think-phase view of the world | `Sync` (shared across workers). It exposes no interior mutability. |
| Per-drone VM instance | `Send`; owns its fuel counter and memory; never shared between drones. |
| `dyn EventSink` | Called only from the resolve phase on the driving thread, so it needs no `Send`/`Sync` bound. |

Everything above that can reach game logic must contain no `f32`/`f64` arithmetic; floats exist only as the software `F`. Ordered collections only (`BTreeMap`, sorted `Vec`) appear in any type whose iteration order can affect results. RNG access goes through `MatchRng` streams derived from `MatchConfig::seed`; there is no other source of randomness.

For the bindings: PyO3 and WASM wrap `Match` behind an owned handle and convert `Event` and `Snapshot` at the boundary. WASM must be single-threaded-capable, so `think_threads = 1` has to be a first-class path and give the same hashes as any other value.

## Stability

| Item | M1 | Notes |
|---|---|---|
| `check`, `CheckReport`, `Diagnostic` | Stable | Diagnostic codes are append-only |
| `Match::new`, `MatchConfig`, `Manifest` | Stable | Mirrors `manifest.schema.json` |
| `step_tick`, `run_clash`, `begin_epoch`, `Phase` | Stable | |
| `EventSink`, `Event`, `NullSink`, `VecSink` | Stable | Tracks `events.schema.json` |
| `state_hash`, `snapshot`, `restore` | Signatures stable | Hash and byte format follow #40 |
| `EngineError` top-level variants | Stable | Sub-errors may grow |
| `Map` | Placeholder | M1 loads a fixed 64² starter map (#50); the mapgen output type and seed-to-map entry point arrive in M2 |
| `ClashResult::scores` | Placeholder | Score breakdown follows the scoring spec |
| `EngineOptions` | Placeholder | May gain fields; `think_threads` stays result-neutral |
| Binding surfaces (PyO3, WASM) | M3 / M5 | Constrained by §6 only |

## Open items for follow-up PRs

- `engine/AGENTS.md` "Owned contracts" should point to this file; that is a code-side change for a follow-up PR (it is outside `spec/`).
- `engine/src/world/` currently starts `World::epoch` at 0; this contract's `epoch()` is 1-based to match `events.schema.json`. The tick scheduler PR (#52) should reconcile them.
- `Manifest`/`Json` types should be added to the engine crate in the PR that implements `Match::new`.
