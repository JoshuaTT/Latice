//! Performance budgets (AGENTS.md §7, system design §11.4, §2.1).
//!
//! `cargo bench -p lattice-engine -- --budget` runs the PR-sized cases and exits
//! non-zero if any exceeds its budget. `--budget --full` adds the worst-case
//! cases. `--budget-scale F` multiplies every budget by `F` (for example
//! `0.0001` to prove the runner fails when a budget is exceeded; CI never uses
//! it). Without `--budget` nothing runs, so a plain `cargo bench` stays quiet.
//!
//! Timing needs the wall clock, which game logic must never read
//! (AGENTS.md §8); this file only measures and feeds nothing back.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::process::ExitCode;
use std::time::Instant;

use lattice_engine::checker::{self, MAX_CODE_NODES};
use lattice_engine::ruleset::Ruleset;
use lattice_engine::tick::{Act, DroneProgram, EngineOptions, NoopResolver, RunOutcome, Scheduler};
use lattice_engine::world::{Cell, EntityId, Pos, Team, Terrain, World, grid::CHUNK_SIDE};

const DEFAULT_RULESET: &str = include_str!("../../spec/rulesets/default.toml");

#[derive(Clone, Copy)]
enum Unit {
    Micros,
    Bytes,
}

struct Budget {
    name: &'static str,
    /// Where the limit comes from.
    source: &'static str,
    limit: u64,
    unit: Unit,
    /// Only run with `--full`.
    full: bool,
}

/// The budgets, in one place. Times are for one core of CI hardware, not the
/// "8 cores" of system design §11.4, because none of these cases is parallel
/// (the 1024² clash, which is, arrives in M2); no scaling factor is applied.
const BUDGETS: [Budget; 4] = [
    Budget {
        name: "world_1024_memory",
        source: "system-design §2.1: a 1024² board in under 200 MB",
        limit: 200 * 1024 * 1024,
        unit: Unit::Bytes,
        full: false,
    },
    Budget {
        name: "world_1024_create_snapshot",
        // Not in the spec: chosen as a sanity bound, a fraction of the 5 s
        // practice-sim budget, since creation and copy-on-write snapshots
        // must be cheap (system-design §2.1).
        source: "derived from system-design §11.4 (practice sim 5 s)",
        limit: 1_000_000,
        unit: Unit::Micros,
        full: false,
    },
    Budget {
        name: "check_4000_node_module",
        source: "system-design §2.1: checker under 50 ms per module",
        limit: 50_000,
        unit: Unit::Micros,
        full: false,
    },
    Budget {
        name: "sim_500_ticks_64",
        source: "system-design §11.4: 500-tick practice sim under 5 s",
        limit: 5_000_000,
        unit: Unit::Micros,
        full: false,
    },
];

fn ruleset() -> Ruleset {
    Ruleset::from_toml_str(DEFAULT_RULESET).expect("default ruleset loads")
}

/// A drone that claims its cell every tick.
struct Claimer;

impl DroneProgram for Claimer {
    fn think(
        &mut self,
        _: &World,
        _: EntityId,
        _: &[lattice_engine::tick::Trigger],
        _: u32,
    ) -> RunOutcome {
        RunOutcome::Acts(vec![Act::Claim])
    }
}

/// Runs a case and returns its measurement in the budget's unit.
fn measure(name: &str) -> u64 {
    let r = ruleset();
    match name {
        "world_1024_memory" => {
            let mut w = World::new(1024, &r).expect("world");
            let wall = Cell {
                terrain: Terrain::Wall,
                ..Cell::default()
            };
            // One write per chunk allocates all of them: the worst case.
            for y in (0..1024).step_by(CHUNK_SIDE) {
                for x in (0..1024).step_by(CHUNK_SIDE) {
                    w.grid.set(Pos { x, y }, wall).expect("in bounds");
                }
            }
            w.grid.estimated_bytes() as u64
        }
        "world_1024_create_snapshot" => {
            let start = Instant::now();
            let w = World::new(1024, &r).expect("world");
            let snap = w.grid.snapshot();
            std::hint::black_box(&snap);
            start.elapsed().as_micros() as u64
        }
        "check_4000_node_module" => {
            // Five nodes per `me.f(1)` statement (see the checker tests).
            let body = "    me.f(1)\n".repeat(MAX_CODE_NODES / 5 - 10);
            let src = format!("def init(me, args):\n    pass\ndef tick(me):\n{body}");
            let start = Instant::now();
            let report = checker::check(&src, &r);
            let ms = start.elapsed().as_micros() as u64;
            assert!(report.ok, "bench module must check clean");
            ms
        }
        "sim_500_ticks_64" => {
            let mut w = World::new(64, &r).expect("world");
            let mut s =
                Scheduler::new(&r, 64, EngineOptions { think_threads: 1 }).expect("scheduler");
            for (team, x) in [(Team::A, 10), (Team::B, 50)] {
                let id = w.add_drone(team, Pos { x, y: 32 }, "bench").expect("drone");
                s.set_program(id, Box::new(Claimer));
            }
            let start = Instant::now();
            for _ in 0..500 {
                s.step_tick(&mut w, &mut NoopResolver);
            }
            start.elapsed().as_micros() as u64
        }
        other => unreachable!("no case {other}"),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--budget") {
        println!("budget bench skipped; run with `-- --budget` (and `--full` for worst cases)");
        return ExitCode::SUCCESS;
    }
    let full = args.iter().any(|a| a == "--full");
    let scale: f64 = args
        .iter()
        .position(|a| a == "--budget-scale")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);

    println!("{:<28} {:>12} {:>12}  source", "case", "measured", "budget");
    let mut failed = false;
    for b in BUDGETS.iter().filter(|b| full || !b.full) {
        let limit = (b.limit as f64 * scale) as u64;
        let got = measure(b.name);
        let (unit, ok) = match b.unit {
            Unit::Micros => ("us", got <= limit),
            Unit::Bytes => ("B", got <= limit),
        };
        failed |= !ok;
        println!(
            "{:<28} {:>9} {unit:<2} {:>9} {unit:<2}  {} {}",
            b.name,
            got,
            limit,
            b.source,
            if ok { "ok" } else { "OVER BUDGET" }
        );
    }
    if full {
        println!(
            "note: the 1024² worst-case clash (system-design §11.4, 90 s on 8 cores) is not built yet (M2); --full adds no cases"
        );
    }
    if failed {
        println!("FAILED: at least one case exceeded its budget");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
