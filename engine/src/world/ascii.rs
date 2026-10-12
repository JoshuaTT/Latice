//! ASCII rendering of a [`World`] for sample replays (M1's human checkpoint).
//!
//! One character per cell. What is drawn, highest priority first: the entity on
//! the cell, then the cell's terrain, then who owns the cell. Output depends
//! only on the world, so it is safe to use in golden text tests.

use std::fmt::Write;

use super::{Entity, Pos, RelicKind, StructureKind, Team, Terrain, World};

/// Every symbol the renderer can draw, in legend order.
const LEGEND: [(char, &str); 17] = [
    ('A', "drone, team A"),
    ('B', "drone, team B"),
    ('N', "drone, no team"),
    ('C', "Core cell"),
    ('F', "Forge"),
    ('R', "Relay"),
    ('T', "Battery"),
    ('S', "Sentinel"),
    ('W', "Firewall Node"),
    ('X', "Barrier"),
    ('*', "Relic"),
    ('~', "Well"),
    ('#', "wall"),
    ('a', "open, owned by A"),
    ('b', "open, owned by B"),
    ('.', "open, neutral"),
    ('?', "terrain without an entity"),
];

/// A rectangle of cells to draw; clipped to the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// What to draw. The default is the whole board, with no header or legend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AsciiOpts {
    pub viewport: Option<Viewport>,
    /// A `tick N epoch E | A: ... | B: ...` line above the map.
    pub header: bool,
    /// A line per symbol that appears in the drawn cells.
    pub legend: bool,
}

/// Renders `world`. Rows run from `y` ascending (top) and columns from `x`
/// ascending; every line ends in `\n`.
pub fn render_ascii(world: &World, opts: &AsciiOpts) -> String {
    let side = world.grid.side();
    let v = opts.viewport.unwrap_or(Viewport {
        x: 0,
        y: 0,
        w: side,
        h: side,
    });
    let (x1, y1) = (
        v.x.saturating_add(v.w).min(side),
        v.y.saturating_add(v.h).min(side),
    );
    let mut out = String::new();
    if opts.header {
        header(world, &mut out);
    }
    let mut used = [false; LEGEND.len()];
    for y in v.y.min(side)..y1 {
        for x in v.x.min(side)..x1 {
            let c = glyph(
                world,
                Pos {
                    x: x as u16,
                    y: y as u16,
                },
            );
            if let Some(i) = LEGEND.iter().position(|&(s, _)| s == c) {
                used[i] = true;
            }
            out.push(c);
        }
        out.push('\n');
    }
    if opts.legend {
        for (i, (c, what)) in LEGEND.iter().enumerate() {
            if used[i] {
                let _ = writeln!(out, "{c} {what}");
            }
        }
    }
    out
}

fn header(world: &World, out: &mut String) {
    let drones = |team| {
        world
            .entities()
            .filter(|(_, e)| matches!(e, Entity::Drone(d) if d.team == team))
            .count()
    };
    let _ = write!(out, "tick {} epoch {}", world.tick, world.epoch);
    for (name, team) in [("A", Team::A), ("B", Team::B)] {
        let energy = world.energy(team).unwrap_or(0);
        let _ = write!(out, " | {name}: energy {energy}, drones {}", drones(team));
    }
    out.push('\n');
}

fn glyph(world: &World, pos: Pos) -> char {
    if let Some(e) = world.entity_at(pos).and_then(|id| world.entity(id)) {
        return match e {
            Entity::Drone(d) => match d.team {
                Team::A => 'A',
                Team::B => 'B',
                Team::Neutral => 'N',
            },
            Entity::Core(_) => 'C',
            Entity::Relic(r) => match r.kind {
                RelicKind::Oracle
                | RelicKind::Surplus
                | RelicKind::ComputeNode
                | RelicKind::Lens
                | RelicKind::Wellspring => '*',
            },
            Entity::Structure(s) => match s.kind {
                StructureKind::Barrier => 'X',
                StructureKind::Relay => 'R',
                StructureKind::Battery => 'T',
                StructureKind::Sentinel => 'S',
                StructureKind::FirewallNode => 'W',
                StructureKind::Forge => 'F',
            },
        };
    }
    // Positions outside the board never reach here; viewports are clipped.
    let Ok(cell) = world.grid.get(pos) else {
        return '?';
    };
    match cell.terrain {
        Terrain::Wall => '#',
        Terrain::Well => '~',
        // Entities own these terrains, so a bare one means a broken world.
        Terrain::Relic | Terrain::Forge | Terrain::Core => '?',
        Terrain::Open => match cell.owner {
            Team::A => 'a',
            Team::B => 'b',
            Team::Neutral => '.',
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ruleset::Ruleset;
    use crate::world::{Cell, Relic, Structure};

    fn world() -> World {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rulesets/default.toml");
        let ruleset = Ruleset::from_path(&path).unwrap();
        World::new(64, &ruleset).unwrap()
    }

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    fn set(w: &mut World, pos: Pos, terrain: Terrain, owner: Team) {
        let cell = Cell {
            terrain,
            owner,
            ..Cell::default()
        };
        w.grid.set(pos, cell).unwrap();
    }

    fn structure(kind: StructureKind, pos: Pos) -> Structure {
        Structure {
            kind,
            team: Team::A,
            pos,
            hp: 20,
            firewall: 0,
            stored_energy: 0,
            module: None,
            mem: Vec::new(),
        }
    }

    /// A 64×64 world with something of everything near the top-left corner.
    fn scene() -> World {
        let mut w = world();
        set(&mut w, p(0, 0), Terrain::Wall, Team::Neutral);
        set(&mut w, p(1, 0), Terrain::Well, Team::Neutral);
        set(&mut w, p(2, 0), Terrain::Open, Team::A);
        set(&mut w, p(3, 0), Terrain::Open, Team::B);
        w.add_core(Team::A, p(2, 3)).unwrap();
        w.add_structure(structure(StructureKind::Forge, p(6, 0)))
            .unwrap();
        w.add_structure(structure(StructureKind::Relay, p(7, 0)))
            .unwrap();
        w.add_structure(structure(StructureKind::Barrier, p(8, 0)))
            .unwrap();
        w.add_relic(Relic {
            kind: RelicKind::Lens,
            pos: p(9, 0),
            hp: 30,
        })
        .unwrap();
        w.add_drone(Team::A, p(4, 1), "m").unwrap();
        w.add_drone(Team::B, p(5, 1), "m").unwrap();
        w
    }

    #[test]
    fn small_world_renders_to_the_exact_string() {
        let opts = AsciiOpts {
            viewport: Some(Viewport {
                x: 0,
                y: 0,
                w: 10,
                h: 5,
            }),
            ..AsciiOpts::default()
        };
        let expected = "\
#~ab..FRX*
....AB....
.CCC......
.CCC......
.CCC......
";
        assert_eq!(render_ascii(&scene(), &opts), expected);
    }

    #[test]
    fn header_and_legend_list_what_is_drawn() {
        let mut w = scene();
        w.tick = 7;
        w.set_energy(Team::A, 120).unwrap();
        let opts = AsciiOpts {
            viewport: Some(Viewport {
                x: 0,
                y: 0,
                w: 4,
                h: 1,
            }),
            header: true,
            legend: true,
        };
        let out = render_ascii(&w, &opts);
        // Legend order follows LEGEND, not first appearance.
        let expected = format!(
            "tick 7 epoch 1 | A: energy 120, drones 1 | B: energy {}, drones 1\n#~ab\n\
             ~ Well\n# wall\na open, owned by A\nb open, owned by B\n",
            w.energy(Team::B).unwrap()
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn viewport_crops_at_board_edges() {
        let mut w = world();
        w.add_drone(Team::B, p(63, 63), "m").unwrap();
        let opts = AsciiOpts {
            viewport: Some(Viewport {
                x: 62,
                y: 62,
                w: 10,
                h: 10,
            }),
            ..AsciiOpts::default()
        };
        assert_eq!(render_ascii(&w, &opts), "..\n.B\n");
        let off = AsciiOpts {
            viewport: Some(Viewport {
                x: 100,
                y: 100,
                w: 5,
                h: 5,
            }),
            ..AsciiOpts::default()
        };
        assert_eq!(render_ascii(&w, &off), "");
    }

    #[test]
    fn whole_board_is_side_by_side_and_deterministic() {
        let w = scene();
        let out = render_ascii(&w, &AsciiOpts::default());
        assert_eq!(out.lines().count(), 64);
        assert!(out.lines().all(|l| l.chars().count() == 64));
        assert_eq!(out, render_ascii(&w.clone(), &AsciiOpts::default()));
    }

    #[test]
    fn every_symbol_is_in_the_legend() {
        let opts = AsciiOpts {
            legend: true,
            ..AsciiOpts::default()
        };
        let out = render_ascii(&scene(), &opts);
        let (map, legend) = out.split_at(out.find("A drone").unwrap());
        for c in map.chars().filter(|c| *c != '\n') {
            assert!(
                legend.lines().any(|l| l.starts_with(c)),
                "`{c}` is drawn but not in the legend"
            );
        }
    }
}
