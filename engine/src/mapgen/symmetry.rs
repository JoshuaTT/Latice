//! Map symmetry utilities for fair map generation.
//!
//! These utilities ensure maps are provably symmetric, satisfying game-design
//! pillar 6: "Maps are symmetric; matches are played as side-swapped pairs".

use crate::world::{Entity, EntityId, Team, World};
use crate::world::grid::Pos;

use crate::world::grid::{Grid, Terrain, Cell};

/// Symmetry transforms the map generator may apply (game-design §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Symmetry {
    /// 180-degree rotation around the board center.
    Rotate180,
    /// Mirror across the vertical center line (x -> side - 1 - x).
    MirrorX,
    /// Mirror across the horizontal center line (y -> side - 1 - y).
    #[default]
    MirrorY,
}

impl Symmetry {
    /// All symmetry variants.
    pub fn all() -> &'static [Symmetry] {
        &[Symmetry::Rotate180, Symmetry::MirrorX, Symmetry::MirrorY]
    }
}

/// Returns the partner position: the cell that corresponds to `pos` for the
/// opposing team under this symmetry. An involution:
/// `partner(partner(p)) == p`.
pub fn partner(sym: Symmetry, pos: Pos, side: u32) -> Pos {
    let s = side - 1;
    match sym {
        Symmetry::Rotate180 => Pos {
            x: (s - pos.x),
            y: (s - pos.y),
        },
        Symmetry::MirrorX => Pos {
            x: (s - pos.x),
            y: pos.y,
        },
        Symmetry::MirrorY => Pos {
            x: pos.x,
            y: (s - pos.y),
        },
    }
}

/// Returns true if a cell maps to itself under this symmetry (e.g., the center
/// cell on odd-sized boards, or cells on the mirror axis).
pub fn is_self_partner(sym: Symmetry, pos: Pos, side: u32) -> bool {
    partner(sym, pos, side) == pos
}

/// Swaps team A and B, leaving Neutral unchanged.
fn swap_team(team: Team) -> Team {
    match team {
        Team::A => Team::B,
        Team::B => Team::A,
        Team::Neutral => Team::Neutral,
    }
}

/// Checks that `world` is symmetric under the given symmetry: every cell's
/// terrain and integrity equals its partner's; owner is swapped (A↔B,
/// Neutral↔Neutral); marks are mirrored; every entity has a partner entity
/// of the same kind and stats with team swapped at the partner position.
///
/// The first mismatch is reported with both positions.
pub fn symmetry_check(world: &World, sym: Symmetry) -> Result<(), String> {
    let side = world.grid.side();

    // Check grid cells.
    for (pos, cell) in world.grid.iter() {
        let partner_pos = partner(sym, pos, side);
        let partner_cell = world.grid.get(partner_pos).map_err(|e| e.to_string())?;

        // Terrain must match exactly.
        if cell.terrain != partner_cell.terrain {
            return Err(format!(
                "terrain mismatch at ({},{}) partner ({},{}): {:?} != {:?}",
                pos.x, pos.y, partner_pos.x, partner_pos.y, cell.terrain, partner_cell.terrain
            ));
        }
        // Integrity must match.
        if cell.integrity != partner_cell.integrity {
            return Err(format!(
                "integrity mismatch at ({},{}) partner ({},{}): {} != {}",
                pos.x, pos.y, partner_pos.x, partner_pos.y, cell.integrity, partner_cell.integrity
            ));
        }
        // Owner: A↔B, Neutral↔Neutral.
        let expected_owner = swap_team(cell.owner);
        if partner_cell.owner != expected_owner {
            return Err(format!(
                "owner mismatch at ({},{}) partner ({},{}): {:?} != {:?}",
                pos.x, pos.y, partner_pos.x, partner_pos.y, cell.owner, partner_cell.owner
            ));
        }
        // Marks must mirror (same keys, values, expiration, but writer is swapped).
        let marks = world.grid.marks(pos).map_err(|e| e.to_string())?;
        let partner_marks = world.grid.marks(partner_pos).map_err(|e| e.to_string())?;
        if marks.len() != partner_marks.len() {
            return Err(format!(
                "marks count mismatch at ({},{}) partner ({}): {} != {}",
                pos.x, pos.y, partner_pos.x, marks.len(), partner_marks.len()
            ));
        }
        for (m, pm) in marks.iter().zip(partner_marks.iter()) {
            if m.key != pm.key || m.value != pm.value || m.expires_tick != pm.expires_tick {
                return Err(format!(
                    "mark content mismatch at ({},{}) partner ({},{})",
                    pos.x, pos.y, partner_pos.x, partner_pos.y
                ));
            }
            if m.writer != swap_team(pm.writer) {
                return Err(format!(
                    "mark writer not swapped at ({},{}) partner ({},{})",
                    pos.x, pos.y, partner_pos.x, partner_pos.y
                ));
            }
        }
    }

    // Check entities: build map of position -> (entity_id, entity).
    let mut entity_at: std::collections::BTreeMap<Pos, (EntityId, &Entity)> = std::collections::BTreeMap::new();
    for (id, entity) in world.entities() {
        entity_at.insert(entity.pos(), (id, entity));
    }

    // Verify each entity has a properly partnered entity.
    for (pos, (_, entity)) in &entity_at {
        let partner_pos = partner(sym, *pos, side);
        
        match entity_at.get(&partner_pos) {
            Some((_, partner_entity)) => {
                // Same variant required.
                match (entity, partner_entity) {
                    (Entity::Drone(e), Entity::Drone(pe)) => {
                        if e.team != swap_team(pe.team) {
                            return Err(format!(
                                "drone team not swapped at ({},{}) partner ({},{}): {:?} != {:?}",
                                pos.x, pos.y, partner_pos.x, partner_pos.y, e.team, pe.team
                            ));
                        }
                        if e.hp != pe.hp || e.firewall != pe.firewall || e.energy != pe.energy || e.module != pe.module {
                            return Err(format!("drone stats mismatch at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                    }
                    (Entity::Structure(e), Entity::Structure(pe)) => {
                        if e.kind != pe.kind {
                            return Err(format!("structure kind mismatch at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                        if e.team != swap_team(pe.team) {
                            return Err(format!("structure team not swapped at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                        if e.hp != pe.hp || e.firewall != pe.firewall {
                            return Err(format!("structure stats mismatch at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                    }
                    (Entity::Core(e), Entity::Core(pe)) => {
                        if e.team != swap_team(pe.team) {
                            return Err(format!("core team not swapped at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                        if e.hp != pe.hp || e.firewall != pe.firewall {
                            return Err(format!("core stats mismatch at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                    }
                    (Entity::Relic(e), Entity::Relic(pe)) => {
                        if e.kind != pe.kind {
                            return Err(format!("relic kind mismatch at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                        if e.hp != pe.hp {
                            return Err(format!("relic hp mismatch at ({},{}) partner ({},{})", pos.x, pos.y, partner_pos.x, partner_pos.y));
                        }
                    }
                    _ => {
                        return Err(format!(
                            "entity type mismatch at ({},{}) partner ({},{})",
                            pos.x, pos.y, partner_pos.x, partner_pos.y
                        ));
                    }
                }
            }
            None => {
                return Err(format!(
                    "no partner entity at ({},{}) partner ({},{})",
                    pos.x, pos.y, partner_pos.x, partner_pos.y
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ruleset::Ruleset;
    use crate::world::{Drone, Structure, StructureKind, Core, Relic, RelicKind};
    use crate::world::grid::Terrain;

    fn ruleset() -> Ruleset {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/rulesets/default.toml");
        Ruleset::from_path(&path).expect("default ruleset loads")
    }

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    proptest::proptest! {
        #[test]
        fn partner_is_involution(sym in proptest::strategy::Just(Symmetry::Rotate180)
            .or(proptest::strategy::Just(Symmetry::MirrorX))
            .or(proptest::strategy::Just(Symmetry::MirrorY))) {
            for side in [64u32, 65, 128, 256, 1023, 1024] {
                for x in 0..side {
                    for y in 0..side {
                        let pos = p(x as u16, y as u16);
                        let p1 = partner(sym, pos, side);
                        let p2 = partner(sym, p1, side);
                        prop_assert_eq!(pos, p2, "partner is involution for {:?} at ({},{}) side {}", sym, x, y, side);
                    }
                }
            }
        }

        #[test]
        fn partner_stays_in_bounds(sym in proptest::strategy::Just(Symmetry::Rotate180)
            .or(proptest::strategy::Just(Symmetry::MirrorX))
            .or(proptest::strategy::Just(Symmetry::MirrorY))) {
            for side in [64u32, 65, 128, 1024] {
                for x in 0..side {
                    for y in 0..side {
                        let pos = p(x as u16, y as u16);
                        let p1 = partner(sym, pos, side);
                        prop_assert!(p1.x < side as u16 && p1.y < side as u16, "partner out of bounds for {:?} at ({},{}) side {}", sym, x, y, side);
                    }
                }
            }
        }
    }

    #[test]
    fn is_self_partner_center_on_odd() {
        // 65x65 board has a true center cell.
        let center = p(32, 32);
        assert!(is_self_partner(Symmetry::Rotate180, center, 65));
        assert!(is_self_partner(Symmetry::MirrorX, center, 65));
        assert!(is_self_partner(Symmetry::MirrorY, center, 65));
    }

    #[test]
    fn is_self_partner_axis_on_mirror() {
        // MirrorX: column 32 on 65-wide board.
        assert!(is_self_partner(Symmetry::MirrorX, p(32, 0), 65));
        // MirrorY: row 32 on 65-tall board.
        assert!(is_self_partner(Symmetry::MirrorY, p(0, 32), 65));
        // Rotate180 has no self-partners on odd boards (all cells rotate away).
        assert!(!is_self_partner(Symmetry::Rotate180, p(32, 32), 65));
    }

    #[test]
    fn is_self_partner_even_no_self() {
        // Even-sized boards have no self-partners.
        for sym in Symmetry::all() {
            assert!(!is_self_partner(sym, p(0, 0), 64));
            assert!(!is_self_partner(sym, p(31, 31), 64));
            assert!(!is_self_partner(sym, p(32, 32), 64));
        }
    }

    #[test]
    fn symmetric_world_passes_check() {
        let r = ruleset();
        let mut w = World::new(64, &r).unwrap();

        // Place mirrored entities.
        let _d_a = w.add_drone(Team::A, p(10, 10), "bot").unwrap();
        let _d_b = w.add_drone(Team::B, p(53, 53), "bot").unwrap();

        // Set mirrored cell ownership.
        w.grid.set(p(10, 10), Cell {
            terrain: Terrain::Open,
            owner: Team::A,
            integrity: 2,
        }).unwrap();
        w.grid.set(p(53, 53), Cell {
            terrain: Terrain::Open,
            owner: Team::B,
            integrity: 2,
        }).unwrap();

        // Should pass symmetry check for Rotate180 (the board is symmetric).
        assert!(symmetry_check(&w, Symmetry::Rotate180).is_ok(), "{:?}", symmetry_check(&w, Symmetry::Rotate180));
    }

    #[test]
    fn asymmetry_detected() {
        let r = ruleset();
        let mut w = World::new(64, &r).unwrap();

        // Place only one drone, no partner.
        w.add_drone(Team::A, p(10, 10), "bot").unwrap();

        let result = symmetry_check(&w, Symmetry::Rotate180);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no partner entity"));
    }

    #[test]
    fn core_3x3_partner() {
        let r = ruleset();
        let mut w = World::new(64, &r).unwrap();

        // Place Core for team A at (5,5) - its 3x3 covers (4-6, 4-6).
        let _core_a = w.add_core(Team::A, p(5, 5)).unwrap();

        // Partner of (5,5) under MirrorX on 64-wide: (64-1-5, 5) = (58, 5).
        let _core_b = w.add_core(Team::B, p(58, 5)).unwrap();

        // Set mirrored ownership.
        for dx in 0u16..3 {
            for dy in 0u16..3 {
                w.grid.set(p(4 + dx, 4 + dy), Cell {
                    terrain: Terrain::Core,
                    owner: Team::A,
                    integrity: 3,
                }).unwrap();
                w.grid.set(p(56 + dx, 4 + dy), Cell {
                    terrain: Terrain::Core,
                    owner: Team::B,
                    integrity: 3,
                }).unwrap();
            }
        }

        assert!(symmetry_check(&w, Symmetry::MirrorX).is_ok(), "{:?}", symmetry_check(&w, Symmetry::MirrorX));
    }
}