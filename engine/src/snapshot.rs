//! In-memory snapshot and restore (`spec/contracts/engine-api.md` §4).
//!
//! The obligation: restoring `m.snapshot()` and applying the same steps as
//! `m` gives byte-identical results. A [`Snapshot`] holds everything that is
//! state, which is at least everything `state-hash.md` hashes: the world
//! (grid chunks shared copy-on-write), RNG stream positions, the scheduler's
//! pending trigger queue, the phase, deployments and the event emitter.
//!
//! What is not state is supplied again on restore: drone programs (their
//! persistent state is `me.mem`, which lives in the world; the VM, #47, adds
//! anything else it needs), the resolver, the win hook and the options.
//! Byte encoding is out of scope.

use crate::error::EngineError;
use crate::events::Emitter;
use crate::game::{ClashEnd, Match, TeamDeployment};
use crate::rng::{MatchRng, RngState, StreamId};
use crate::ruleset::Ruleset;
use crate::tick::{DroneProgram, EngineOptions, Resolver, Trigger};
use crate::world::hash::Phase;
use crate::world::{EntityId, World};

/// An immutable capture of a [`Match`]. `Clone + Send + Sync`; cloning shares
/// grid chunks.
#[derive(Debug, Clone)]
pub struct Snapshot {
    ruleset_version: String,
    seed: u64,
    phase: Phase,
    ended: Option<ClashEnd>,
    world: World,
    deployments: [TeamDeployment; 2],
    rng: Vec<(StreamId, RngState)>,
    pending: Vec<(EntityId, Trigger)>,
    emitter: Emitter,
}

impl Snapshot {
    pub fn ruleset_version(&self) -> &str {
        &self.ruleset_version
    }
}

impl Match {
    /// Captures the match. Cheap for the grid (chunks are shared, not
    /// copied); the entity tables are cloned.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            ruleset_version: self.ruleset.version().to_owned(),
            seed: self.seed,
            phase: self.phase,
            ended: self.ended,
            world: self.world.clone(),
            deployments: self.deployments.clone(),
            rng: self.rng.iter().map(|(s, r)| (*s, r.state())).collect(),
            pending: self
                .scheduler
                .pending_triggers()
                .map(|(id, t)| (id, t.clone()))
                .collect(),
            emitter: self.emitter.clone(),
        }
    }

    /// Triggers waiting for the next tick, in delivery order.
    #[doc(hidden)]
    pub fn pending_triggers(&self) -> Vec<(EntityId, Trigger)> {
        self.scheduler
            .pending_triggers()
            .map(|(id, t)| (id, t.clone()))
            .collect()
    }

    /// Rebuilds a match from `snapshot`. `programs`, `resolver` and `options`
    /// are not state, so the caller supplies them again; the win hook, if
    /// any, must be set again too. Fails with
    /// [`EngineError::RulesetMismatch`] if `ruleset`'s version differs from
    /// the one the snapshot was taken under.
    pub fn restore(
        snapshot: &Snapshot,
        ruleset: Ruleset,
        programs: Vec<(EntityId, Box<dyn DroneProgram>)>,
        resolver: Box<dyn Resolver + Send>,
        options: EngineOptions,
    ) -> Result<Match, EngineError> {
        if ruleset.version() != snapshot.ruleset_version {
            return Err(EngineError::RulesetMismatch {
                expected: snapshot.ruleset_version.clone(),
                found: ruleset.version().to_owned(),
            });
        }
        let mut m = Match::from_parts(
            snapshot.world.clone(),
            programs,
            resolver,
            ruleset,
            snapshot.seed,
            options,
        )?;
        m.phase = snapshot.phase;
        m.ended = snapshot.ended;
        m.deployments = snapshot.deployments.clone();
        m.rng = snapshot
            .rng
            .iter()
            .map(|(s, st)| (*s, MatchRng::from_state(snapshot.seed, *s, *st)))
            .collect();
        m.emitter = snapshot.emitter.clone();
        m.scheduler.set_pending(snapshot.pending.clone());
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NullSink;
    use crate::tick::{Act, RunOutcome};
    use crate::world::grid::{Cell, Pos, Team, Terrain};
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(u64, EntityId, Act)>>>;

    fn ruleset() -> Ruleset {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
        Ruleset::from_path(&path).expect("default ruleset loads")
    }

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    /// Stateless on purpose (programs are not snapshotted): the direction
    /// depends on the view, the tick and the triggers received, so losing the
    /// pending queue or any world state changes the walk.
    struct Walker;

    impl DroneProgram for Walker {
        fn think(
            &mut self,
            view: &World,
            id: EntityId,
            triggers: &[Trigger],
            _: u32,
        ) -> RunOutcome {
            let seen: u64 = view
                .entities()
                .map(|(i, e)| u64::from(i.0) * u64::from(e.pos().x + 3 * e.pos().y + 1))
                .sum();
            let me = e_pos(view, id);
            let dir = (seen + view.tick + u64::from(id.0) + 7 * triggers.len() as u64) % 4;
            let to = match dir {
                0 => p((me.x + 1).min(63), me.y),
                1 => p(me.x.saturating_sub(1), me.y),
                2 => p(me.x, (me.y + 1).min(63)),
                _ => p(me.x, me.y.saturating_sub(1)),
            };
            RunOutcome::Acts(vec![Act::Move { to }])
        }
    }

    fn e_pos(view: &World, id: EntityId) -> Pos {
        view.entity(id).map_or(p(0, 0), |e| e.pos())
    }

    /// Applies moves and emits an `on_hit` to every drone on every third tick.
    struct Mover {
        log: Log,
    }

    impl Resolver for Mover {
        fn resolve(
            &mut self,
            world: &mut World,
            acts: &[(EntityId, Act)],
        ) -> Vec<(EntityId, Trigger)> {
            let mut out = Vec::new();
            for (id, act) in acts {
                self.log
                    .lock()
                    .unwrap()
                    .push((world.tick, *id, act.clone()));
                if let Act::Move { to } = act {
                    let _ = world.move_entity(*id, *to);
                }
                if world.tick.is_multiple_of(3) {
                    out.push((
                        *id,
                        Trigger::OnHit {
                            attacker: EntityId(0),
                        },
                    ));
                }
            }
            out
        }
    }

    fn programs(ids: &[EntityId]) -> Vec<(EntityId, Box<dyn DroneProgram>)> {
        ids.iter()
            .map(|&i| (i, Box::new(Walker) as Box<dyn DroneProgram>))
            .collect()
    }

    fn fresh(threads: usize, log: &Log) -> (Match, Vec<EntityId>) {
        let r = ruleset();
        let mut world = World::new(64, &r).unwrap();
        let ids: Vec<EntityId> = (0..12u16)
            .map(|i| world.add_drone(Team::A, p(i * 3, i * 2), "m").unwrap())
            .collect();
        let m = Match::from_parts(
            world,
            programs(&ids),
            Box::new(Mover { log: log.clone() }),
            r,
            9,
            EngineOptions {
                think_threads: threads,
            },
        )
        .unwrap();
        (m, ids)
    }

    fn run(m: &mut Match, ticks: u64) {
        for _ in 0..ticks {
            m.step_tick(&mut NullSink).unwrap();
        }
    }

    fn positions(m: &Match, ids: &[EntityId]) -> Vec<Pos> {
        ids.iter().map(|&i| e_pos(m.world(), i)).collect()
    }

    #[test]
    fn restore_then_continue_equals_the_uninterrupted_run() {
        for threads in [1, 8] {
            let full_log: Log = Arc::default();
            let (mut full, ids) = fresh(threads, &full_log);
            run(&mut full, 100);

            let first_log: Log = Arc::default();
            let (mut first, _) = fresh(threads, &first_log);
            run(&mut first, 40);
            let snap = first.snapshot();
            // The live match may carry on or be dropped; the snapshot is
            // unaffected either way.
            run(&mut first, 5);

            let second_log: Log = Arc::default();
            let mut restored = Match::restore(
                &snap,
                ruleset(),
                programs(&ids),
                Box::new(Mover {
                    log: second_log.clone(),
                }),
                EngineOptions {
                    think_threads: threads,
                },
            )
            .unwrap();
            assert_eq!(restored.tick(), 40);
            run(&mut restored, 60);

            assert_eq!(
                positions(&restored, &ids),
                positions(&full, &ids),
                "threads={threads}"
            );
            let tail: Vec<_> = full_log
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.0 >= 40)
                .cloned()
                .collect();
            assert_eq!(*second_log.lock().unwrap(), tail);
            assert_eq!(restored.pending_triggers(), full.pending_triggers());
            assert!(!restored.pending_triggers().is_empty());
            assert_eq!(restored.state_hash(), full.state_hash());
            restored.world().check_invariants().unwrap();
        }
    }

    #[test]
    fn snapshot_shares_unchanged_chunks_and_is_isolated_from_the_live_world() {
        let r = ruleset();
        let mut world = World::new(128, &r).unwrap();
        let owned = Cell {
            terrain: Terrain::Open,
            owner: Team::A,
            integrity: 2,
        };
        world.grid.set(p(1, 1), owned).unwrap();
        world.grid.set(p(100, 100), owned).unwrap();
        let mut m = Match::idle(world, r, 1).unwrap();
        let snap = m.snapshot();

        let flipped = Cell {
            owner: Team::B,
            ..owned
        };
        m.world.grid.set(p(1, 1), flipped).unwrap();
        assert_eq!(snap.world.grid.get(p(1, 1)).unwrap(), owned);
        assert!(!snap.world.grid.shares_chunk_with(&m.world.grid, p(1, 1)));
        assert!(
            snap.world
                .grid
                .shares_chunk_with(&m.world.grid, p(100, 100))
        );
        assert!(snap.world.grid.shares_chunk_with(&m.world.grid, p(100, 1)));
    }

    #[test]
    fn restoring_under_a_different_ruleset_version_is_an_error() {
        let (m, _) = fresh(1, &Arc::default());
        let snap = m.snapshot();
        let mut other = ruleset();
        other.version = "9.9.9".into();
        let err = Match::restore(
            &snap,
            other,
            Vec::new(),
            Box::new(Mover {
                log: Arc::default(),
            }),
            EngineOptions::default(),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(matches!(
            err,
            EngineError::RulesetMismatch { ref expected, ref found }
                if *expected == snap.ruleset_version() && found == "9.9.9"
        ));
    }

    #[test]
    fn snapshot_is_send_sync_and_clone() {
        fn check<T: Send + Sync + Clone>() {}
        check::<Snapshot>();
    }
}
