//! The `Match` facade: phase machine over the tick scheduler
//! (`spec/contracts/engine-api.md` §3).
//!
//! `Deployed → InClash → EpochOver → (begin_epoch) → Deployed … → MatchOver`.
//! `Match::new` (manifest and library validation) comes later; until then
//! matches are built with [`Match::from_parts`].

use sha2::{Digest, Sha256};

use crate::error::EngineError;
use crate::rng::{MatchRng, StreamId};
use crate::ruleset::Ruleset;
use crate::tick::{DroneProgram, EngineOptions, NoopResolver, Resolver, Scheduler};
use crate::world::grid::Team;
use crate::world::hash::{Extras, Phase};
use crate::world::{EntityId, World};

/// An event as seen by a sink. Placeholder until #59 adds the per-type
/// payloads from `events.schema.json`; `seq` is assigned in resolve order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Event {
    pub epoch: u32,
    pub tick: u64,
    pub seq: u64,
}

/// Receives events as they are emitted. Infallible and must not influence the
/// simulation.
pub trait EventSink {
    fn emit(&mut self, event: &Event);
}

/// Discards events.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&mut self, _: &Event) {}
}

/// A module's name and source (`engine-api.md` §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    pub source: String,
}

/// What a team deploys for an epoch. The manifest joins `library` when
/// `Match::new` lands.
// TODO: add `manifest` with the Match::new issue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TeamDeployment {
    pub library: Vec<Module>,
}

/// How a clash ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClashEnd {
    CoreDestroyed(Team),
    CoreCaptured(Team),
    LengthReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickOutcome {
    pub clash_over: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClashResult {
    pub epoch: u32,
    pub ticks_run: u64,
    pub end: ClashEnd,
    pub state_hash: [u8; 32],
    /// Placeholder until the scoring spec lands.
    pub scores: [u64; 2],
}

/// Decides, after each tick, whether a Core was destroyed or captured.
// TODO(#54): the combat PR supplies the real hook.
type WinHook = Box<dyn FnMut(&World) -> Option<ClashEnd> + Send>;

pub struct Match {
    world: World,
    scheduler: Scheduler,
    resolver: Box<dyn Resolver + Send>,
    win: Option<WinHook>,
    ruleset: Ruleset,
    seed: u64,
    phase: Phase,
    clash_length: u64,
    /// Why the clash that just finished ended; taken by `run_clash`.
    ended: Option<ClashEnd>,
    deployments: [TeamDeployment; 2],
    rng: [(StreamId, MatchRng); 2],
}

impl Match {
    /// Provisional constructor for tests and `lattice sim` until `Match::new`
    /// exists. Starts at epoch 1, tick 0, in `Deployed`.
    #[doc(hidden)]
    pub fn from_parts(
        world: World,
        programs: Vec<(EntityId, Box<dyn DroneProgram>)>,
        resolver: Box<dyn Resolver + Send>,
        ruleset: Ruleset,
        seed: u64,
        options: EngineOptions,
    ) -> Result<Match, EngineError> {
        let side = world.grid.side();
        let mut scheduler = Scheduler::new(&ruleset, side, options)?;
        for (id, program) in programs {
            scheduler.set_program(id, program);
        }
        let clash_length = ruleset.derived(side)?.clash_length_ticks;
        Ok(Match {
            world,
            scheduler,
            resolver,
            win: None,
            ruleset,
            seed,
            phase: Phase::Deployed,
            clash_length,
            ended: None,
            deployments: [TeamDeployment::default(), TeamDeployment::default()],
            rng: [StreamId::Mapgen, StreamId::TieBreak].map(|s| (s, MatchRng::new(seed, s))),
        })
    }

    /// A match whose drones do nothing: for scheduler-free phase tests.
    #[doc(hidden)]
    pub fn idle(world: World, ruleset: Ruleset, seed: u64) -> Result<Match, EngineError> {
        Match::from_parts(
            world,
            Vec::new(),
            Box::new(NoopResolver),
            ruleset,
            seed,
            EngineOptions::default(),
        )
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 1-based, as in `events.schema.json`.
    pub fn epoch(&self) -> u32 {
        self.world.epoch
    }

    /// 0-based tick within the current clash.
    pub fn tick(&self) -> u64 {
        self.world.tick
    }

    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn deployment(&self, team: Team) -> Option<&TeamDeployment> {
        match team {
            Team::A => Some(&self.deployments[0]),
            Team::B => Some(&self.deployments[1]),
            Team::Neutral => None,
        }
    }

    /// Installs the Core-destroyed/captured check run after every tick.
    #[doc(hidden)]
    pub fn set_win_hook(&mut self, hook: WinHook) {
        self.win = Some(hook);
    }

    /// Hash of the whole match state at this moment (`state-hash.md`).
    pub fn state_hash(&self) -> [u8; 32] {
        let libraries = self.deployments.clone().map(|d| {
            d.library
                .iter()
                .map(|m| (m.name.clone(), Sha256::digest(m.source.as_bytes()).into()))
                .collect()
        });
        let extras = Extras {
            seed: self.seed,
            phase: self.phase,
            libraries,
            rng: self.rng.iter().map(|(s, r)| (*s, r.state())).collect(),
            // TODO: event seq, blackboards, radio and handler queues, drone
            // RNG streams, once the systems that own them land.
            ..Extras::default()
        };
        self.world.state_hash(&extras)
    }

    fn require(&self, expected: &'static str, ok: bool) -> Result<(), EngineError> {
        if ok {
            Ok(())
        } else {
            Err(EngineError::WrongPhase {
                expected,
                found: self.phase,
            })
        }
    }

    /// One think + resolve cycle. Valid in `Deployed` or `InClash`.
    pub fn step_tick(&mut self, _sink: &mut dyn EventSink) -> Result<TickOutcome, EngineError> {
        self.require(
            "Deployed or InClash",
            matches!(self.phase, Phase::Deployed | Phase::InClash),
        )?;
        self.phase = Phase::InClash;
        self.scheduler
            .step_tick(&mut self.world, self.resolver.as_mut());
        let end = self
            .win
            .as_mut()
            .and_then(|hook| hook(&self.world))
            .or((self.world.tick >= self.clash_length).then_some(ClashEnd::LengthReached));
        if end.is_some() {
            self.phase = if self.epoch() >= self.ruleset.timing.epochs_per_match {
                Phase::MatchOver
            } else {
                Phase::EpochOver
            };
            self.ended = end;
        }
        Ok(TickOutcome {
            clash_over: end.is_some(),
        })
    }

    /// Steps until the clash ends. Valid in `Deployed` or `InClash`.
    pub fn run_clash(&mut self, sink: &mut dyn EventSink) -> Result<ClashResult, EngineError> {
        self.require(
            "Deployed or InClash",
            matches!(self.phase, Phase::Deployed | Phase::InClash),
        )?;
        while !self.step_tick(sink)?.clash_over {}
        let end = self
            .ended
            .take()
            .ok_or(EngineError::Internal("clash ended without a reason"))?;
        Ok(ClashResult {
            epoch: self.epoch(),
            ticks_run: self.world.tick,
            end,
            state_hash: self.state_hash(),
            scores: [0, 0],
        })
    }

    /// Between epochs: swaps in the deployments that are `Some` (`None` keeps
    /// a team's previous one), resets the clash tick, and returns to
    /// `Deployed`. Energy pools and territory persist (game-design §4.2).
    pub fn begin_epoch(&mut self, next: [Option<TeamDeployment>; 2]) -> Result<(), EngineError> {
        self.require("EpochOver", self.phase == Phase::EpochOver)?;
        // TODO(#45): re-validate incoming deployments with `check` and the
        // quota before swapping them in.
        for (slot, new) in self.deployments.iter_mut().zip(next) {
            if let Some(dep) = new {
                *slot = dep;
            }
        }
        self.ended = None;
        self.world.tick = 0;
        self.world.epoch += 1;
        self.phase = Phase::Deployed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Terrain;
    use crate::world::grid::{Cell, Pos};

    const EPOCHS: u32 = 3;
    const CLASH: u64 = 5;

    fn ruleset() -> Ruleset {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
        let mut r = Ruleset::from_path(&path).expect("default ruleset loads");
        r.timing.epochs_per_match = EPOCHS;
        r.scaling.clash_length_ticks.base = CLASH as u32;
        r
    }

    fn new_match() -> Match {
        let r = ruleset();
        let world = World::new(64, &r).unwrap();
        Match::idle(world, r, 7).unwrap()
    }

    fn dep(name: &str) -> TeamDeployment {
        TeamDeployment {
            library: vec![Module {
                name: name.into(),
                source: "def init(me, args):\n    pass\n".into(),
            }],
        }
    }

    #[test]
    fn match_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Match>();
    }

    #[test]
    fn wrong_phase_calls_error_and_leave_the_match_unchanged() {
        let mut m = new_match();
        let sink = &mut NullSink;

        // Deployed: only stepping is valid.
        assert!(matches!(
            m.begin_epoch([None, None]),
            Err(EngineError::WrongPhase {
                expected: "EpochOver",
                found: Phase::Deployed
            })
        ));
        m.step_tick(sink).unwrap();
        assert_eq!(m.phase(), Phase::InClash);
        assert!(matches!(
            m.begin_epoch([None, None]),
            Err(EngineError::WrongPhase {
                expected: "EpochOver",
                found: Phase::InClash
            })
        ));

        // EpochOver: stepping and running are invalid.
        m.run_clash(sink).unwrap();
        assert_eq!(m.phase(), Phase::EpochOver);
        let (hash, tick, epoch) = (m.state_hash(), m.tick(), m.epoch());
        assert!(matches!(
            m.step_tick(sink),
            Err(EngineError::WrongPhase {
                expected: "Deployed or InClash",
                found: Phase::EpochOver
            })
        ));
        assert!(matches!(
            m.run_clash(sink),
            Err(EngineError::WrongPhase { .. })
        ));
        assert_eq!(
            (m.state_hash(), m.tick(), m.epoch(), m.phase()),
            (hash, tick, epoch, Phase::EpochOver)
        );

        // MatchOver: nothing is valid.
        for _ in 1..EPOCHS {
            m.begin_epoch([None, None]).unwrap();
            m.run_clash(sink).unwrap();
        }
        assert_eq!(m.phase(), Phase::MatchOver);
        let hash = m.state_hash();
        assert!(matches!(
            m.step_tick(sink),
            Err(EngineError::WrongPhase {
                found: Phase::MatchOver,
                ..
            })
        ));
        assert!(matches!(
            m.run_clash(sink),
            Err(EngineError::WrongPhase { .. })
        ));
        assert!(matches!(
            m.begin_epoch([None, None]),
            Err(EngineError::WrongPhase {
                expected: "EpochOver",
                found: Phase::MatchOver
            })
        ));
        assert_eq!((m.state_hash(), m.phase()), (hash, Phase::MatchOver));
    }

    #[test]
    fn full_match_walks_the_phases_with_one_based_epochs_and_tick_reset() {
        let mut m = new_match();
        let sink = &mut NullSink;
        for epoch in 1..=EPOCHS {
            assert_eq!(
                (m.phase(), m.epoch(), m.tick()),
                (Phase::Deployed, epoch, 0)
            );
            assert_eq!(
                m.step_tick(sink).unwrap(),
                TickOutcome { clash_over: false }
            );
            assert_eq!((m.phase(), m.tick()), (Phase::InClash, 1));
            let result = m.run_clash(sink).unwrap();
            assert_eq!(result.epoch, epoch);
            assert_eq!(result.ticks_run, CLASH);
            assert_eq!(result.end, ClashEnd::LengthReached);
            assert_eq!(result.state_hash, m.state_hash());
            let last = epoch == EPOCHS;
            let expected = if last {
                Phase::MatchOver
            } else {
                Phase::EpochOver
            };
            assert_eq!(m.phase(), expected);
            if !last {
                m.begin_epoch([None, None]).unwrap();
            }
        }
    }

    #[test]
    fn energy_territory_and_unreplaced_deployments_persist_across_epochs() {
        let mut m = new_match();
        m.world.set_energy(Team::A, 123).unwrap();
        let owned = Cell {
            terrain: Terrain::Open,
            owner: Team::B,
            integrity: 2,
        };
        m.world.grid.set(Pos { x: 3, y: 3 }, owned).unwrap();
        m.run_clash(&mut NullSink).unwrap();
        m.begin_epoch([Some(dep("alpha")), None]).unwrap();
        m.run_clash(&mut NullSink).unwrap();
        m.begin_epoch([None, Some(dep("bravo"))]).unwrap();

        assert_eq!(m.world().energy(Team::A), Some(123));
        assert_eq!(m.world().grid.get(Pos { x: 3, y: 3 }).unwrap(), owned);
        // A kept "alpha" through the epoch where it passed None.
        assert_eq!(m.deployment(Team::A).unwrap().library[0].name, "alpha");
        assert_eq!(m.deployment(Team::B).unwrap().library[0].name, "bravo");
        assert_eq!(m.epoch(), 3);
    }

    #[test]
    fn win_hook_ends_the_clash_early() {
        let mut m = new_match();
        m.set_win_hook(Box::new(|w| {
            (w.tick == 2).then_some(ClashEnd::CoreDestroyed(Team::B))
        }));
        let result = m.run_clash(&mut NullSink).unwrap();
        assert_eq!(
            (result.ticks_run, result.end),
            (2, ClashEnd::CoreDestroyed(Team::B))
        );
        assert_eq!(m.phase(), Phase::EpochOver);
    }
}
