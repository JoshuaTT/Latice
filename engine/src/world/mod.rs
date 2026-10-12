//! World model: chunked grid plus the entities on it.
//!
//! Everything iterates in a deterministic order (AGENTS.md §8): cells
//! row-major, entities by ascending [`EntityId`]; no `HashMap` anywhere.

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
        for &pos in &cells {
            self.grid.get(pos)?;
            if let Some(&by) = self.occupancy.get(&pos) {
                return Err(WorldError::Occupied { pos, by });
            }
        }
        let id = EntityId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(WorldError::IdsExhausted)?;
        for pos in cells {
            self.occupancy.insert(pos, id);
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
        for pos in entity.footprint() {
            self.occupancy.remove(&pos);
        }
        Ok(entity)
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn entity_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(&id)
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
