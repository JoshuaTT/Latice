//! Tick scheduler: each tick runs a **think** phase (every drone program runs
//! against the read-only start-of-tick world, optionally in parallel) and then
//! a **resolve** phase (the collected acts are applied serially).
//!
//! Determinism (AGENTS.md §8, `spec/contracts/engine-api.md` §3 and §6):
//! think results are sorted by entity id before resolve, so thread scheduling
//! never reaches game state, and `think_threads` changes timing only.
//! Snapshot isolation falls out of ownership: think borrows the world shared,
//! resolve borrows it mutably only after every think has finished.

use std::collections::BTreeMap;

use rayon::prelude::*;

use crate::ruleset::{Ruleset, RulesetError};
use crate::world::{Entity, EntityId, Pos, StructureKind, World};

/// An act a program asks for. Placeholder set: #53–#55 own act semantics and
/// extend this enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Move { to: Pos },
    Claim,
    Attack { target: EntityId },
}

/// An event delivered to a drone's handler on the tick after it happened
/// (game-design §5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    OnHit { attacker: EntityId },
    OnHacked { attacker: EntityId },
    OnRadio { content: String },
}

/// Why a program's tick produced nothing (`spec/latticepy.md` §8: errors are
/// contained to the drone's tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    OutOfFuel,
    Raised,
    CapExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Acts(Vec<Act>),
    Failed(Failure),
}

/// What runs a drone's (or Sentinel's) code. The VM (#47, #56) implements it.
/// `Send` so instances can be moved to think workers; each owns its own state.
pub trait DroneProgram: Send {
    /// `view` is the world at the start of this tick. `triggers` are the
    /// events from the previous tick's resolve, in the order the resolver
    /// emitted them; handlers run before `tick`.
    fn think(&mut self, view: &World, id: EntityId, triggers: &[Trigger], fuel: u32) -> RunOutcome;
}

/// Applies a tick's acts to the world (#53–#55).
pub trait Resolver {
    /// `acts` is ordered by ascending entity id, then program order within a
    /// drone. A resolver imposes the act-type priority and seat rotation from
    /// `events.schema.json` itself. Returns triggers for the next tick.
    fn resolve(&mut self, world: &mut World, acts: &[(EntityId, Act)]) -> Vec<(EntityId, Trigger)>;
}

/// Resolver that applies nothing; for tests and scheduler-only runs.
pub struct NoopResolver;

impl Resolver for NoopResolver {
    fn resolve(&mut self, _: &mut World, _: &[(EntityId, Act)]) -> Vec<(EntityId, Trigger)> {
        Vec::new()
    }
}

/// Engine options (`engine-api.md` §2). Never part of any hash.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// 0 = rayon's default; 1 = no pool at all. Must not change any result.
    pub think_threads: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum TickError {
    #[error(transparent)]
    Ruleset(#[from] RulesetError),
    #[error("cannot build think thread pool: {0}")]
    Pool(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClashStop {
    /// The win-condition hook returned true.
    Won,
    /// The scaled clash length elapsed.
    LengthReached,
}

pub struct Scheduler {
    fuel_per_tick: u32,
    clash_length: u64,
    pool: Option<rayon::ThreadPool>,
    programs: BTreeMap<EntityId, Box<dyn DroneProgram>>,
    pending: BTreeMap<EntityId, Vec<Trigger>>,
}

impl Scheduler {
    /// Fuel and clash length come from the ruleset, scaled to `side`.
    pub fn new(ruleset: &Ruleset, side: u32, options: EngineOptions) -> Result<Self, TickError> {
        let derived = ruleset.derived(side)?;
        let pool = if options.think_threads == 1 {
            None
        } else {
            Some(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(options.think_threads)
                    .build()
                    .map_err(|e| TickError::Pool(e.to_string()))?,
            )
        };
        Ok(Scheduler {
            fuel_per_tick: ruleset.scaling.fuel_per_tick,
            clash_length: derived.clash_length_ticks,
            pool,
            programs: BTreeMap::new(),
            pending: BTreeMap::new(),
        })
    }

    /// Attaches the program that runs `id`'s code.
    pub fn set_program(&mut self, id: EntityId, program: Box<dyn DroneProgram>) {
        self.programs.insert(id, program);
    }

    /// One think + resolve cycle; advances `world.tick`.
    // TODO(#55): scale fuel by the Compute Node relic bonus.
    pub fn step_tick(&mut self, world: &mut World, resolver: &mut dyn Resolver) {
        // Programs of removed entities are dropped, and so are their triggers.
        self.programs
            .retain(|id, _| world.entity(*id).is_some_and(thinks));
        self.pending.retain(|id, _| world.entity(*id).is_some());

        let fuel = self.fuel_per_tick;
        let (view, triggers): (&World, &BTreeMap<EntityId, Vec<Trigger>>) = (world, &self.pending);
        let jobs: Vec<(EntityId, &mut Box<dyn DroneProgram>)> =
            self.programs.iter_mut().map(|(&id, p)| (id, p)).collect();
        let run = |(id, program): (EntityId, &mut Box<dyn DroneProgram>)| {
            let own = triggers.get(&id).map_or(&[][..], Vec::as_slice);
            (id, program.think(view, id, own, fuel))
        };
        let mut results: Vec<(EntityId, RunOutcome)> = match &self.pool {
            Some(pool) => pool.install(|| jobs.into_par_iter().map(run).collect()),
            None => jobs.into_iter().map(run).collect(),
        };
        // Worker completion order must not leak into resolve.
        results.sort_by_key(|(id, _)| *id);

        let mut acts = Vec::new();
        for (id, outcome) in results {
            if let RunOutcome::Acts(list) = outcome {
                acts.extend(list.into_iter().map(|a| (id, a)));
            }
        }
        let emitted = resolver.resolve(world, &acts);
        self.pending.clear();
        for (id, trigger) in emitted {
            self.pending.entry(id).or_default().push(trigger);
        }
        world.tick += 1;
    }

    /// Runs ticks until `won` returns true (checked after each tick) or the
    /// scaled clash length has elapsed.
    pub fn run_clash(
        &mut self,
        world: &mut World,
        resolver: &mut dyn Resolver,
        won: &mut dyn FnMut(&World) -> bool,
    ) -> (ClashStop, u64) {
        for ran in 1..=self.clash_length {
            self.step_tick(world, resolver);
            if won(world) {
                return (ClashStop::Won, ran);
            }
        }
        (ClashStop::LengthReached, self.clash_length)
    }

    /// Returns pending handler events in emission order, for state hashing.
    pub fn pending_handlers(&self) -> impl Iterator<Item = (&EntityId, &Vec<Trigger>)> {
        self.pending.iter()
    }
}

/// Only drones and Sentinels run code (game-design §4.3, §5.2).
fn thinks(entity: &Entity) -> bool {
    matches!(entity, Entity::Drone(_))
        || matches!(entity, Entity::Structure(s) if s.kind == StructureKind::Sentinel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::Team;
    use std::sync::{Arc, Mutex};

    fn ruleset() -> Ruleset {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
        Ruleset::from_path(&path).expect("default ruleset loads")
    }

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    fn drone_pos(world: &World, id: EntityId) -> Pos {
        match world.entity(id) {
            Some(Entity::Drone(d)) => d.pos,
            other => panic!("expected drone, got {other:?}"),
        }
    }

    /// Reads the view (all drone positions) and walks in a direction that
    /// depends on it, so any view or ordering difference changes the outcome.
    struct Walker {
        steps: u64,
    }

    impl DroneProgram for Walker {
        fn think(&mut self, view: &World, id: EntityId, _: &[Trigger], _: u32) -> RunOutcome {
            self.steps += 1;
            let seen: u64 = view
                .entities()
                .map(|(i, e)| u64::from(i.0) * u64::from(e.pos().x + 2 * e.pos().y + 1))
                .sum();
            let me = drone_pos(view, id);
            let to = match (seen + self.steps + u64::from(id.0)) % 4 {
                0 => p(me.x.saturating_add(1).min(63), me.y),
                1 => p(me.x.saturating_sub(1), me.y),
                2 => p(me.x, me.y.saturating_add(1).min(63)),
                _ => p(me.x, me.y.saturating_sub(1)),
            };
            RunOutcome::Acts(vec![Act::Move { to }])
        }
    }

    /// Moves entities by editing `pos` directly. Occupancy is not maintained
    /// (that is #89's `move_entity`); these tests compare positions only.
    #[derive(Default)]
    struct MoveResolver {
        log: Vec<(u64, EntityId, Act)>,
    }

    impl Resolver for MoveResolver {
        fn resolve(
            &mut self,
            world: &mut World,
            acts: &[(EntityId, Act)],
        ) -> Vec<(EntityId, Trigger)> {
            for (id, act) in acts {
                self.log.push((world.tick, *id, act.clone()));
                if let (Act::Move { to }, Some(Entity::Drone(d))) = (act, world.entity_mut(*id)) {
                    d.pos = *to;
                }
            }
            Vec::new()
        }
    }

    fn setup(threads: usize, drones: u16) -> (World, Scheduler, Vec<EntityId>) {
        let r = ruleset();
        let mut world = World::new(64, &r).unwrap();
        let mut sched = Scheduler::new(
            &r,
            64,
            EngineOptions {
                think_threads: threads,
            },
        )
        .unwrap();
        let ids = (0..drones)
            .map(|i| {
                let id = world.add_drone(Team::A, p(i, i), "m").unwrap();
                sched.set_program(id, Box::new(Walker { steps: 0 }));
                id
            })
            .collect();
        (world, sched, ids)
    }

    fn run(threads: usize) -> (Vec<Pos>, Vec<(u64, EntityId, Act)>) {
        let (mut world, mut sched, ids) = setup(threads, 24);
        let mut resolver = MoveResolver::default();
        for _ in 0..200 {
            sched.step_tick(&mut world, &mut resolver);
        }
        assert_eq!(world.tick, 200);
        (
            ids.iter().map(|&i| drone_pos(&world, i)).collect(),
            resolver.log,
        )
    }

    #[test]
    fn results_do_not_depend_on_thread_count() {
        let baseline = run(1);
        assert_eq!(baseline.1.len(), 24 * 200);
        assert_eq!(run(2), baseline);
        assert_eq!(run(8), baseline);
    }

    #[test]
    fn acts_reach_resolver_in_ascending_id_order() {
        let (mut world, mut sched, _) = setup(4, 10);
        let mut resolver = MoveResolver::default();
        sched.step_tick(&mut world, &mut resolver);
        assert!(resolver.log.windows(2).all(|w| w[0].1 < w[1].1));
    }

    struct Failing(Failure);

    impl DroneProgram for Failing {
        fn think(&mut self, _: &World, _: EntityId, _: &[Trigger], _: u32) -> RunOutcome {
            RunOutcome::Failed(self.0)
        }
    }

    #[test]
    fn failed_drone_yields_no_act_and_does_not_affect_others() {
        let (mut world, mut sched, ids) = setup(2, 3);
        // Replace the middle drone's program with failing ones, one failure kind each tick pair.
        sched.set_program(ids[1], Box::new(Failing(Failure::OutOfFuel)));
        let mut resolver = MoveResolver::default();
        sched.step_tick(&mut world, &mut resolver);
        sched.set_program(ids[1], Box::new(Failing(Failure::Raised)));
        sched.step_tick(&mut world, &mut resolver);
        sched.set_program(ids[1], Box::new(Failing(Failure::CapExceeded)));
        sched.step_tick(&mut world, &mut resolver);
        assert_eq!(resolver.log.len(), 6);
        assert!(resolver.log.iter().all(|(_, id, _)| *id != ids[1]));
        assert_eq!(drone_pos(&world, ids[1]), p(1, 1));
    }

    /// Records what drone `watched` looked like to the reader's think.
    struct Reader {
        watched: EntityId,
        seen: Arc<Mutex<Vec<Pos>>>,
    }

    impl DroneProgram for Reader {
        fn think(&mut self, view: &World, _: EntityId, _: &[Trigger], _: u32) -> RunOutcome {
            self.seen
                .lock()
                .unwrap()
                .push(drone_pos(view, self.watched));
            RunOutcome::Acts(vec![])
        }
    }

    struct StepRight {
        target: EntityId,
    }

    impl Resolver for StepRight {
        fn resolve(
            &mut self,
            world: &mut World,
            _: &[(EntityId, Act)],
        ) -> Vec<(EntityId, Trigger)> {
            if let Some(Entity::Drone(d)) = world.entity_mut(self.target) {
                d.pos.x += 1;
            }
            Vec::new()
        }
    }

    #[test]
    fn resolve_changes_are_invisible_until_next_tick() {
        let r = ruleset();
        let mut world = World::new(64, &r).unwrap();
        let mut sched = Scheduler::new(&r, 64, EngineOptions { think_threads: 4 }).unwrap();
        let mover = world.add_drone(Team::A, p(0, 0), "m").unwrap();
        let reader = world.add_drone(Team::B, p(5, 5), "m").unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        sched.set_program(
            reader,
            Box::new(Reader {
                watched: mover,
                seen: seen.clone(),
            }),
        );
        let mut resolver = StepRight { target: mover };
        for _ in 0..4 {
            sched.step_tick(&mut world, &mut resolver);
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec![p(0, 0), p(1, 0), p(2, 0), p(3, 0)]
        );
    }

    type Seen = Vec<(u64, Vec<Trigger>)>;

    struct TriggerProbe {
        got: Arc<Mutex<Seen>>,
        tick: u64,
    }

    impl DroneProgram for TriggerProbe {
        fn think(&mut self, _: &World, _: EntityId, t: &[Trigger], _: u32) -> RunOutcome {
            self.got.lock().unwrap().push((self.tick, t.to_vec()));
            self.tick += 1;
            RunOutcome::Acts(vec![])
        }
    }

    struct EmitOnce {
        to: EntityId,
        done: bool,
    }

    impl Resolver for EmitOnce {
        fn resolve(&mut self, _: &mut World, _: &[(EntityId, Act)]) -> Vec<(EntityId, Trigger)> {
            if self.done {
                return Vec::new();
            }
            self.done = true;
            vec![
                (
                    self.to,
                    Trigger::OnRadio {
                        content: "a".into(),
                    },
                ),
                (
                    self.to,
                    Trigger::OnHit {
                        attacker: EntityId(99),
                    },
                ),
            ]
        }
    }

    #[test]
    fn triggers_arrive_next_tick_in_emission_order_once() {
        let r = ruleset();
        let mut world = World::new(64, &r).unwrap();
        let mut sched = Scheduler::new(&r, 64, EngineOptions { think_threads: 1 }).unwrap();
        let id = world.add_drone(Team::A, p(0, 0), "m").unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        sched.set_program(
            id,
            Box::new(TriggerProbe {
                got: got.clone(),
                tick: 0,
            }),
        );
        let mut resolver = EmitOnce {
            to: id,
            done: false,
        };
        for _ in 0..3 {
            sched.step_tick(&mut world, &mut resolver);
        }
        let expected = vec![
            Trigger::OnRadio {
                content: "a".into(),
            },
            Trigger::OnHit {
                attacker: EntityId(99),
            },
        ];
        assert_eq!(
            *got.lock().unwrap(),
            vec![(0, vec![]), (1, expected), (2, vec![])]
        );
    }

    #[test]
    fn epoch_is_one_based() {
        assert_eq!(World::new(64, &ruleset()).unwrap().epoch, 1);
    }

    #[test]
    fn run_clash_stops_at_win_or_scaled_length() {
        let r = ruleset();
        let length = r.derived(64).unwrap().clash_length_ticks;
        let mut sched = Scheduler::new(&r, 64, EngineOptions::default()).unwrap();
        let mut world = World::new(64, &r).unwrap();
        let (stop, ran) = sched.run_clash(&mut world, &mut NoopResolver, &mut |_| false);
        assert_eq!(
            (stop, ran, world.tick),
            (ClashStop::LengthReached, length, length)
        );
        let mut world = World::new(64, &r).unwrap();
        let (stop, ran) = sched.run_clash(&mut world, &mut NoopResolver, &mut |w| w.tick == 7);
        assert_eq!((stop, ran), (ClashStop::Won, 7));
    }
}
