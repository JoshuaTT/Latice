//! Entities that live on the grid (game-design §4.3–§4.5, §5.2).

use super::grid::{Pos, Team};

/// Stable entity handle, allocated monotonically and never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u32);

/// Structure kinds; names match `events.schema.json` `StructureKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructureKind {
    Barrier,
    Relay,
    Battery,
    Sentinel,
    FirewallNode,
    Forge,
}

/// The five relic kinds (game-design §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelicKind {
    Oracle,
    Surplus,
    ComputeNode,
    Lens,
    Wellspring,
}

/// A drone (game-design §5.2). `mem` is `me.mem`, opaque until the VM defines
/// LatticePy values; `module` names the module it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drone {
    pub team: Team,
    pub pos: Pos,
    pub hp: u32,
    pub firewall: u32,
    pub energy: u32,
    pub mem: Vec<u8>,
    pub module: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    pub kind: StructureKind,
    pub team: Team,
    pub pos: Pos,
    pub hp: u32,
    pub firewall: u32,
}

/// A team's Core: 3×3 around `pos` (its centre cell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core {
    pub team: Team,
    pub pos: Pos,
    pub hp: u32,
    pub firewall: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relic {
    pub kind: RelicKind,
    pub pos: Pos,
    pub hp: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entity {
    Drone(Drone),
    Structure(Structure),
    Core(Core),
    Relic(Relic),
}

impl Entity {
    /// Anchor position (the centre cell for a Core).
    pub fn pos(&self) -> Pos {
        match self {
            Entity::Drone(e) => e.pos,
            Entity::Structure(e) => e.pos,
            Entity::Core(e) => e.pos,
            Entity::Relic(e) => e.pos,
        }
    }

    /// Cells this entity occupies: one, or the 3×3 footprint for a Core.
    /// Clipped to `u16` range; callers bounds-check against the grid.
    pub fn footprint(&self) -> Vec<Pos> {
        match self {
            Entity::Core(c) => {
                let mut cells = Vec::with_capacity(9);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (x, y) = (i32::from(c.pos.x) + dx, i32::from(c.pos.y) + dy);
                        if let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) {
                            cells.push(Pos { x, y });
                        }
                    }
                }
                cells
            }
            other => vec![other.pos()],
        }
    }
}
