//! World model: chunked grid plus the entities on it.
//!
//! Everything iterates in a deterministic order (AGENTS.md §8): cells
//! row-major, entities by ascending [`EntityId`]; no `HashMap` anywhere.
//!
//! # One source of truth per fact
//!
//! Three views say where things are: each entity's `pos`, the `occupancy`
//! index, and grid terrain. They are kept in step by construction:
//!
//! - **Entities are authoritative for Forge, Relic and Core cells.** Adding a
//!   Core sets its nine cells to [`Terrain::Core`], a Forge structure its cell
//!   to [`Terrain::Forge`] and a Relic its cell to [`Terrain::Relic`]; removing
//!   the entity sets them back to [`Terrain::Open`]. Those terrains therefore
//!   exist only under a matching entity, and the entity must be placed on
//!   `Open` terrain. Wells and walls are pure terrain.
//! - **`occupancy` is derived** from entity footprints and is private.
//! - **Position changes only through [`World::move_entity`].** There is no
//!   `&mut Entity` accessor; the mutators ([`World::drone_mut`],
//!   [`World::hp_mut`], [`World::firewall_mut`]) cannot reach `pos`, so code
//!   that moves an entity another way does not compile.
//!
//! [`World::check_invariants`] verifies all of this and is for tests and the
//! scenario harness.

pub mod ascii;
pub mod entity;
pub mod grid;
pub mod hash;

use std::collections::BTreeMap;

pub use entity::{Core, Drone, Entity, EntityId, Relic, RelicKind, Structure, StructureKind};
pub use grid::{Cell, Grid, GridError, Mark, Pos, Team, Terrain};

use crate::ruleset::{Ruleset, RulesetError};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorldError {
    #[error(transparent)]
    Grid(#[from] GridError),
    #[error("cell ({}, {}) is already occupied by entity {}", pos.x, pos.y, by.0)]
    Occupied { pos: Pos, by: EntityId },
    #[error("no entity with id {}", .0.0)]
    NoSuchEntity(EntityId),
    #[error("entity ids exhausted")]
    IdsExhausted,
    #[error("entity {} is not a drone and cannot move", .0.0)]
    NotADrone(EntityId),
    #[error("cell ({}, {}) is {terrain:?}; it must be Open to place a Core, Forge or Relic", pos.x, pos.y)]
    BadTerrain { pos: Pos, terrain: Terrain },
    #[error("Neutral has no energy pool")]
    NeutralHasNoPool,
}

/// Unit stats copied from the ruleset's `[board]` table at construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnitStats {
    drone_hp: u32,
    drone_firewall: u32,
    core_hp: u32,
    core_firewall: u32,
}

/// The full simulation state. `Clone` copies the entity maps (the grid part
/// is copy-on-write, see [`Grid::snapshot`]).
#[derive(Debug, Clone)]
pub struct World {
    pub grid: Grid,
    pub tick: u64,
    pub epoch: u32,
    entities: BTreeMap<EntityId, Entity>,
    occupancy: BTreeMap<Pos, EntityId>,
    next_id: u32,
    energy: [u64; 2],
    stats: UnitStats,
}

impl World {
    /// An empty world of `side`×`side` cells; both teams start with the
    /// board-scaled starting energy (game-design §4.0).
    pub fn new(side: u32, ruleset: &Ruleset) -> Result<World, RulesetError> {
        let derived = ruleset.derived(side)?;
        let b = &ruleset.board;
        Ok(World {
            grid: Grid::new(side),
            tick: 0,
            // 1-based, as in events.schema.json and engine-api.md.
            epoch: 1,
            entities: BTreeMap::new(),
            occupancy: BTreeMap::new(),
            next_id: 0,
            energy: [derived.starting_energy; 2],
            stats: UnitStats {
                drone_hp: b.drone_hp,
                drone_firewall: b.drone_firewall,
                core_hp: b.core_hp,
                core_firewall: b.core_firewall,
            },
        })
    }

    pub fn energy(&self, team: Team) -> Option<u64> {
        pool(team).map(|i| self.energy[i])
    }

    pub fn set_energy(&mut self, team: Team, amount: u64) -> Result<(), WorldError> {
        self.energy[pool(team).ok_or(WorldError::NeutralHasNoPool)?] = amount;
        Ok(())
    }

    /// Adds a drone with ruleset HP and firewall.
    pub fn add_drone(
        &mut self,
        team: Team,
        pos: Pos,
        module: &str,
    ) -> Result<EntityId, WorldError> {
        self.insert(Entity::Drone(Drone {
            team,
            pos,
            hp: self.stats.drone_hp,
            firewall: self.stats.drone_firewall,
            energy: 0,
            mem: Vec::new(),
            module: module.to_owned(),
        }))
    }

    /// Adds a Core centred on `pos` with ruleset HP and firewall.
    pub fn add_core(&mut self, team: Team, pos: Pos) -> Result<EntityId, WorldError> {
        self.insert(Entity::Core(Core {
            team,
            pos,
            hp: self.stats.core_hp,
            firewall: self.stats.core_firewall,
        }))
    }

    /// Adds a structure. HP and firewall are the caller's until the clash
    /// parameters land in the ruleset.
    // TODO(#37): read structure HP/firewall from the ruleset.
    pub fn add_structure(&mut self, s: Structure) -> Result<EntityId, WorldError> {
        self.insert(Entity::Structure(s))
    }

    // TODO(#37): read relic HP (30) from the ruleset.
    pub fn add_relic(&mut self, r: Relic) -> Result<EntityId, WorldError> {
        self.insert(Entity::Relic(r))
    }

    fn insert(&mut self, entity: Entity) -> Result<EntityId, WorldError> {
        let cells = entity.footprint();
        // footprint() drops cells below 0; a Core must fit entirely.
        if matches!(entity, Entity::Core(_)) && cells.len() != 9 {
            return Err(GridError::OutOfBounds {
                pos: entity.pos(),
                side: self.grid.side(),
            }
            .into());
        }
        let owns = owned_terrain(&entity);
        for &pos in &cells {
            let cell = self.grid.get(pos)?;
            if let Some(&by) = self.occupancy.get(&pos) {
                return Err(WorldError::Occupied { pos, by });
            }
            if owns.is_some() && cell.terrain != Terrain::Open {
                return Err(WorldError::BadTerrain {
                    pos,
                    terrain: cell.terrain,
                });
            }
        }
        let id = EntityId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted)?;
        for pos in cells {
            self.occupancy.insert(pos, id);
            if let Some(terrain) = owns {
                self.set_terrain(pos, terrain);
            }
        }
        self.entities.insert(id, entity);
        Ok(id)
    }

    /// Removes an entity and frees its cells. Its id is never reissued.
    pub fn remove(&mut self, id: EntityId) -> Result<Entity, WorldError> {
        let entity = self
            .entities
            .remove(&id)
            .ok_or(WorldError::NoSuchEntity(id))?;
        let owns = owned_terrain(&entity).is_some();
        for pos in entity.footprint() {
            self.occupancy.remove(&pos);
            if owns {
                self.set_terrain(pos, Terrain::Open);
            }
        }
        Ok(entity)
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    /// The mutable parts of a drone: everything except its position, which only
    /// [`World::move_entity`] may change.
    pub fn drone_mut(&mut self, id: EntityId) -> Option<DroneMut<'_>> {
        match self.entities.get_mut(&id)? {
            Entity::Drone(d) => Some(DroneMut {
                team: &mut d.team,
                hp: &mut d.hp,
                firewall: &mut d.firewall,
                energy: &mut d.energy,
                mem: &mut d.mem,
                module: &mut d.module,
            }),
            _ => None,
        }
    }

    /// Hit points of any entity.
    pub fn hp_mut(&mut self, id: EntityId) -> Option<&mut u32> {
        Some(match self.entities.get_mut(&id)? {
            Entity::Drone(e) => &mut e.hp,
            Entity::Structure(e) => &mut e.hp,
            Entity::Core(e) => &mut e.hp,
            Entity::Relic(e) => &mut e.hp,
        })
    }

    /// Firewall of a drone, structure or Core (Relics have none).
    pub fn firewall_mut(&mut self, id: EntityId) -> Option<&mut u32> {
        match self.entities.get_mut(&id)? {
            Entity::Drone(e) => Some(&mut e.firewall),
            Entity::Structure(e) => Some(&mut e.firewall),
            Entity::Core(e) => Some(&mut e.firewall),
            Entity::Relic(_) => None,
        }
    }

    /// Moves a drone to `to`, updating its position and the occupancy index
    /// together. Fails, leaving the world unchanged, if `to` is off the board
    /// or held by another entity. Movement rules (adjacency, simultaneous
    /// moves) are the resolver's.
    pub fn move_entity(&mut self, id: EntityId, to: Pos) -> Result<(), WorldError> {
        let from = match self.entities.get(&id) {
            Some(Entity::Drone(d)) => d.pos,
            Some(_) => return Err(WorldError::NotADrone(id)),
            None => return Err(WorldError::NoSuchEntity(id)),
        };
        self.grid.get(to)?;
        match self.occupancy.get(&to) {
            Some(&by) if by != id => return Err(WorldError::Occupied { pos: to, by }),
            Some(_) => return Ok(()),
            None => {}
        }
        self.occupancy.remove(&from);
        self.occupancy.insert(to, id);
        if let Some(Entity::Drone(d)) = self.entities.get_mut(&id) {
            d.pos = to;
        }
        Ok(())
    }

    /// Verifies the single-source-of-truth rules in the module docs: occupancy
    /// equals the union of entity footprints with no overlaps, every entity is
    /// on the board, and Core/Forge/Relic terrain exists exactly under the
    /// matching entities. Cost is linear in board size; use it in tests.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut cells = 0usize;
        for (id, e) in self.entities() {
            let footprint = e.footprint();
            let want = if matches!(e, Entity::Core(_)) { 9 } else { 1 };
            if footprint.len() != want {
                return Err(format!("entity {} is partly off the board", id.0));
            }
            for pos in footprint {
                let cell = self.grid.get(pos).map_err(|_| {
                    format!("entity {} at ({}, {}) is off the board", id.0, pos.x, pos.y)
                })?;
                match self.occupancy.get(&pos) {
                    Some(&by) if by == id => {}
                    other => {
                        return Err(format!(
                            "cell ({}, {}) belongs to entity {} but occupancy says {other:?}",
                            pos.x, pos.y, id.0
                        ));
                    }
                }
                let terrain = owned_terrain(e).unwrap_or(cell.terrain);
                if cell.terrain != terrain {
                    return Err(format!(
                        "cell ({}, {}) is {:?} but entity {} needs {terrain:?}",
                        pos.x, pos.y, cell.terrain, id.0
                    ));
                }
                cells += 1;
            }
        }
        if self.occupancy.len() != cells {
            return Err(format!(
                "occupancy has {} cells but entities cover {cells}",
                self.occupancy.len()
            ));
        }
        for (pos, cell) in self.grid.iter() {
            let owned = matches!(
                cell.terrain,
                Terrain::Core | Terrain::Forge | Terrain::Relic
            );
            let covered = self
                .occupancy
                .get(&pos)
                .and_then(|id| self.entities.get(id))
                .and_then(owned_terrain);
            if owned && covered != Some(cell.terrain) {
                return Err(format!(
                    "cell ({}, {}) is {:?} with no matching entity",
                    pos.x, pos.y, cell.terrain
                ));
            }
        }
        Ok(())
    }

    fn set_terrain(&mut self, pos: Pos, terrain: Terrain) {
        // Callers have bounds-checked `pos`.
        if let Ok(cell) = self.grid.get(pos) {
            let _ = self.grid.set(pos, Cell { terrain, ..cell });
        }
    }

    /// The entity covering `pos`, if any.
    pub fn entity_at(&self, pos: Pos) -> Option<EntityId> {
        self.occupancy.get(&pos).copied()
    }

    /// All entities in ascending id order.
    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &Entity)> {
        self.entities.iter().map(|(&id, e)| (id, e))
    }
}

/// The mutable parts of a drone; see [`World::drone_mut`].
pub struct DroneMut<'a> {
    pub team: &'a mut Team,
    pub hp: &'a mut u32,
    pub firewall: &'a mut u32,
    pub energy: &'a mut u32,
    pub mem: &'a mut Vec<u8>,
    pub module: &'a mut String,
}

/// The terrain an entity owns under the authority rule in the module docs.
fn owned_terrain(e: &Entity) -> Option<Terrain> {
    match e {
        Entity::Core(_) => Some(Terrain::Core),
        Entity::Structure(Structure {
            kind: StructureKind::Forge,
            ..
        }) => Some(Terrain::Forge),
        Entity::Relic(_) => Some(Terrain::Relic),
        _ => None,
    }
}

fn pool(team: Team) -> Option<usize> {
    match team {
        Team::A => Some(0),
        Team::B => Some(1),
        Team::Neutral => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ruleset() -> Ruleset {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
        Ruleset::from_path(&path).expect("default ruleset loads")
    }

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    fn barrier(pos: Pos) -> Structure {
        Structure {
            kind: StructureKind::Barrier,
            team: Team::A,
            pos,
            hp: 20,
            firewall: 0,
            stored_energy: 0,
            module: None,
            mem: Vec::new(),
        }
    }

    #[test]
    fn creates_64_and_1024_worlds_with_scaled_energy() {
        let r = ruleset();
        let small = World::new(64, &r).unwrap();
        let big = World::new(1024, &r).unwrap();
        assert_eq!(
            small.energy(Team::A),
            Some(r.derived(64).unwrap().starting_energy)
        );
        assert_eq!(
            big.energy(Team::B),
            Some(r.derived(1024).unwrap().starting_energy)
        );
        assert_eq!(big.energy(Team::Neutral), None);
        assert_eq!(big.grid.side(), 1024);
        assert!(World::new(32, &r).is_err());
    }

    #[test]
    fn ids_are_monotonic_never_reused_and_ordered() {
        let mut w = World::new(64, &ruleset()).unwrap();
        // Insert out of positional order; iteration still follows ids.
        let a = w.add_drone(Team::A, p(60, 60), "m").unwrap();
        let b = w.add_drone(Team::B, p(0, 0), "m").unwrap();
        let c = w.add_structure(barrier(p(10, 10))).unwrap();
        assert!(a < b && b < c);
        w.remove(b).unwrap();
        let d = w.add_drone(Team::B, p(0, 0), "m").unwrap();
        assert!(d > c);
        let ids: Vec<_> = w.entities().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![a, c, d]);
        assert_eq!(w.remove(b), Err(WorldError::NoSuchEntity(b)));
    }

    #[test]
    fn occupancy_tracks_add_and_remove() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let id = w.add_drone(Team::A, p(64 - 1, 5), "m").unwrap();
        assert_eq!(w.entity_at(p(63, 5)), Some(id));
        assert_eq!(
            w.add_structure(barrier(p(63, 5))),
            Err(WorldError::Occupied {
                pos: p(63, 5),
                by: id
            })
        );
        w.remove(id).unwrap();
        assert_eq!(w.entity_at(p(63, 5)), None);
        assert!(matches!(
            w.add_drone(Team::A, p(64, 0), "m"),
            Err(WorldError::Grid(GridError::OutOfBounds { .. }))
        ));
    }

    #[test]
    fn core_occupies_3x3_and_uses_ruleset_stats() {
        let r = ruleset();
        let mut w = World::new(64, &r).unwrap();
        let id = w.add_core(Team::A, p(5, 5)).unwrap();
        for (x, y) in [(4, 4), (5, 5), (6, 6), (4, 6)] {
            assert_eq!(w.entity_at(p(x, y)), Some(id));
        }
        assert_eq!(w.entity_at(p(7, 5)), None);
        let Some(Entity::Core(core)) = w.entity(id) else {
            panic!("expected a core");
        };
        assert_eq!(
            (core.hp, core.firewall),
            (r.board.core_hp, r.board.core_firewall)
        );
        // A drone can't stand inside the Core footprint.
        assert!(w.add_drone(Team::B, p(6, 4), "m").is_err());
        // A Core hanging off the board is rejected, and nothing is left behind.
        assert!(w.add_core(Team::B, p(0, 0)).is_err());
        assert_eq!(w.entity_at(p(1, 1)), None);
        w.remove(id).unwrap();
        assert_eq!(w.entity_at(p(5, 5)), None);
    }

    #[test]
    fn move_entity_updates_position_and_occupancy() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let d = w.add_drone(Team::A, p(3, 3), "m").unwrap();
        w.move_entity(d, p(4, 3)).unwrap();
        assert_eq!(w.entity_at(p(3, 3)), None);
        assert_eq!(w.entity_at(p(4, 3)), Some(d));
        assert_eq!(w.entity(d).map(Entity::pos), Some(p(4, 3)));
        w.check_invariants().unwrap();
        // Moving in place is a no-op.
        w.move_entity(d, p(4, 3)).unwrap();
        w.check_invariants().unwrap();
    }

    #[test]
    fn failed_moves_leave_the_world_unchanged() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let a = w.add_drone(Team::A, p(0, 0), "m").unwrap();
        let b = w.add_drone(Team::B, p(1, 0), "m").unwrap();
        let wall = w.add_structure(barrier(p(5, 5))).unwrap();
        let core = w.add_core(Team::A, p(20, 20)).unwrap();
        let before = format!("{:?}", w.entities().collect::<Vec<_>>());
        assert_eq!(
            w.move_entity(a, p(1, 0)),
            Err(WorldError::Occupied {
                pos: p(1, 0),
                by: b
            })
        );
        assert!(matches!(
            w.move_entity(a, p(64, 0)),
            Err(WorldError::Grid(GridError::OutOfBounds { .. }))
        ));
        assert_eq!(
            w.move_entity(wall, p(6, 5)),
            Err(WorldError::NotADrone(wall))
        );
        assert_eq!(
            w.move_entity(core, p(30, 30)),
            Err(WorldError::NotADrone(core))
        );
        assert_eq!(
            w.move_entity(EntityId(99), p(1, 1)),
            Err(WorldError::NoSuchEntity(EntityId(99)))
        );
        assert_eq!(before, format!("{:?}", w.entities().collect::<Vec<_>>()));
        assert_eq!(w.entity_at(p(0, 0)), Some(a));
        w.check_invariants().unwrap();
    }

    #[test]
    fn entities_own_forge_relic_and_core_terrain() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let core = w.add_core(Team::A, p(5, 5)).unwrap();
        assert_eq!(w.grid.get(p(4, 6)).unwrap().terrain, Terrain::Core);
        let forge = w
            .add_structure(Structure {
                kind: StructureKind::Forge,
                ..barrier(p(10, 10))
            })
            .unwrap();
        assert_eq!(w.grid.get(p(10, 10)).unwrap().terrain, Terrain::Forge);
        let relic = w
            .add_relic(Relic {
                kind: RelicKind::Lens,
                pos: p(12, 12),
                hp: 30,
            })
            .unwrap();
        assert_eq!(w.grid.get(p(12, 12)).unwrap().terrain, Terrain::Relic);
        // A barrier does not own terrain.
        w.add_structure(barrier(p(14, 14))).unwrap();
        assert_eq!(w.grid.get(p(14, 14)).unwrap().terrain, Terrain::Open);
        w.check_invariants().unwrap();
        for id in [core, forge, relic] {
            w.remove(id).unwrap();
        }
        assert_eq!(w.grid.get(p(5, 5)).unwrap().terrain, Terrain::Open);
        assert_eq!(w.grid.get(p(10, 10)).unwrap().terrain, Terrain::Open);
        assert_eq!(w.grid.get(p(12, 12)).unwrap().terrain, Terrain::Open);
        w.check_invariants().unwrap();
    }

    #[test]
    fn owned_terrain_needs_open_ground() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let wall = Cell {
            terrain: Terrain::Wall,
            ..Cell::default()
        };
        w.grid.set(p(7, 7), wall).unwrap();
        let relic = Relic {
            kind: RelicKind::Oracle,
            pos: p(7, 7),
            hp: 30,
        };
        assert!(matches!(
            w.add_relic(relic),
            Err(WorldError::BadTerrain { .. })
        ));
        assert!(w.add_core(Team::B, p(7, 8)).is_err());
        assert_eq!(w.entity_at(p(7, 8)), None);
        w.check_invariants().unwrap();
    }

    #[test]
    fn invariants_catch_a_stray_terrain_cell() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let forge = Cell {
            terrain: Terrain::Forge,
            ..Cell::default()
        };
        w.grid.set(p(2, 2), forge).unwrap();
        assert!(w.check_invariants().is_err());
    }

    #[test]
    fn mutators_cannot_reach_position() {
        let mut w = World::new(64, &ruleset()).unwrap();
        let d = w.add_drone(Team::A, p(1, 1), "m").unwrap();
        let s = w.add_structure(barrier(p(2, 2))).unwrap();
        {
            let m = w.drone_mut(d).unwrap();
            *m.hp -= 1;
            *m.energy = 9;
        }
        *w.hp_mut(s).unwrap() = 1;
        *w.firewall_mut(s).unwrap() = 2;
        assert!(w.drone_mut(s).is_none());
        w.check_invariants().unwrap();
    }

    #[test]
    fn drone_uses_ruleset_stats_and_relic_roundtrips() {
        let r = ruleset();
        let mut w = World::new(64, &r).unwrap();
        let d = w.add_drone(Team::A, p(1, 1), "scout").unwrap();
        let Some(Entity::Drone(drone)) = w.entity(d) else {
            panic!("expected a drone");
        };
        assert_eq!(
            (drone.hp, drone.firewall),
            (r.board.drone_hp, r.board.drone_firewall)
        );
        let rel = w
            .add_relic(Relic {
                kind: RelicKind::Lens,
                pos: p(9, 9),
                hp: 30,
            })
            .unwrap();
        assert!(matches!(w.entity(rel), Some(Entity::Relic(_))));
        w.set_energy(Team::A, 7).unwrap();
        assert_eq!(w.energy(Team::A), Some(7));
        assert_eq!(
            w.set_energy(Team::Neutral, 1),
            Err(WorldError::NeutralHasNoPool)
        );
    }
}

#[cfg(test)]
mod prop {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        Drone(u16, u16),
        Core(u16, u16),
        Forge(u16, u16),
        Relic(u16, u16),
        Barrier(u16, u16),
        Move(usize, u16, u16),
        Remove(usize),
    }

    fn op() -> impl Strategy<Value = Op> {
        // A small corner of the board makes collisions frequent.
        let c = 0u16..12;
        prop_oneof![
            (c.clone(), c.clone()).prop_map(|(x, y)| Op::Drone(x, y)),
            (c.clone(), c.clone()).prop_map(|(x, y)| Op::Core(x, y)),
            (c.clone(), c.clone()).prop_map(|(x, y)| Op::Forge(x, y)),
            (c.clone(), c.clone()).prop_map(|(x, y)| Op::Relic(x, y)),
            (c.clone(), c.clone()).prop_map(|(x, y)| Op::Barrier(x, y)),
            (0usize..16, c.clone(), c.clone()).prop_map(|(i, x, y)| Op::Move(i, x, y)),
            (0usize..16).prop_map(Op::Remove),
        ]
    }

    proptest! {
        #[test]
        fn random_operations_keep_every_view_in_step(ops in proptest::collection::vec(op(), 0..60)) {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
            let mut w = World::new(64, &Ruleset::from_path(&path).unwrap()).unwrap();
            let mut ids: Vec<EntityId> = Vec::new();
            let at = |x, y| Pos { x, y };
            let structure = |kind, x, y| Structure {
                kind,
                team: Team::A,
                pos: at(x, y),
                hp: 20,
                firewall: 0,
                stored_energy: 0,
                module: None,
                mem: Vec::new(),
            };
            for op in ops {
                // Failed operations must leave the world valid too.
                match op {
                    Op::Drone(x, y) => ids.extend(w.add_drone(Team::A, at(x, y), "m").ok()),
                    Op::Core(x, y) => ids.extend(w.add_core(Team::B, at(x, y)).ok()),
                    Op::Forge(x, y) => ids.extend(w.add_structure(structure(StructureKind::Forge, x, y)).ok()),
                    Op::Barrier(x, y) => ids.extend(w.add_structure(structure(StructureKind::Barrier, x, y)).ok()),
                    Op::Relic(x, y) => ids.extend(w.add_relic(Relic { kind: RelicKind::Oracle, pos: at(x, y), hp: 30 }).ok()),
                    Op::Move(i, x, y) => {
                        if let Some(&id) = ids.get(i % ids.len().max(1)) {
                            let _ = w.move_entity(id, at(x, y));
                        }
                    }
                    Op::Remove(i) => {
                        if !ids.is_empty() {
                            let id = ids.remove(i % ids.len());
                            w.remove(id).unwrap();
                        }
                    }
                }
                prop_assert_eq!(w.check_invariants(), Ok(()));
            }
        }
    }
}
