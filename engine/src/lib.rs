//! Lattice Engine — the deterministic simulation core.
//!
//! This crate owns the world model, tick scheduler, LatticePy virtual machine,
//! code checker, game library (stdlib), map generator, ghost builder, and
//! telemetry sharder.

pub mod ruleset;

/// World model: chunked grid, cells, terrain, ownership, integrity, marks,
/// structures, drones, Cores, Relics.
pub mod world;

/// Tick scheduler: think phase (parallel) and resolve phase (serial) each tick.
pub mod tick;

/// LatticePy virtual machine: parse, validate, compile to bytecode, execute
/// with fuel metering and memory caps.
pub mod vm {}

/// Code checker and quota counter: validates the Python subset, counts AST
/// nodes, checks the module interface, gives lint messages.
pub mod checker {}

/// Game library (`lattice`): sensing, movement, pathfinding, economy, combat,
/// hacking, building, comms, high-level behaviors.
pub mod stdlib {}

/// Map generator: seeded, symmetric maps with fairness validation.
pub mod mapgen {}

/// Ghost builder: builds practice opponents from known enemy modules plus
/// house bots.
pub mod ghost {}

/// Telemetry sharder: splits clash events into per-player, per-team, and god
/// views; builds digests.
pub mod shard {}

/// Seeded ChaCha RNG with stream IDs: the engine's only source of randomness.
pub mod rng;
