//! Canonical serialization and SHA-256 state hash, byte for byte as
//! `spec/contracts/state-hash.md` (format version 1) specifies.
//!
//! The hash covers more than [`World`] holds (seed, blackboards, radio and
//! handler queues, deployed libraries, RNG positions), so the caller passes
//! those as [`Extras`]; the match layer will own them later.
//!
//! LatticePy values (`me.mem`, mark values, blackboard values) are stored as
//! opaque `Vec<u8>` until the VM lands (#47). Here they are taken to be the
//! contract's canonical value encoding already, with an empty vector meaning
//! `None` (`0x00`).
// TODO(#47): encode real LatticePy values instead of taking pre-encoded bytes.

use sha2::{Digest, Sha256};

use super::World;
use super::entity::{Entity, EntityId, RelicKind, StructureKind};
use super::grid::{Pos, Team, Terrain};
use crate::rng::{RngState, StreamId};

/// Version byte at the start of the stream.
pub const FORMAT_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Deployed,
    InClash,
    EpochOver,
    MatchOver,
}

/// A team blackboard: entries in insertion order, values pre-encoded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Board {
    pub entries: Vec<(String, Vec<u8>)>,
    pub reserved: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadioMsg {
    pub sender: EntityId,
    pub pos: Pos,
    pub content: String,
    pub range: u32,
    pub deliver_tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    OnHit,
    OnHacked,
}

/// A pending `on_hit`/`on_hacked` delivery, in emission order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingHandler {
    pub target: EntityId,
    pub handler: HandlerKind,
    pub attacker: EntityId,
}

/// State the hash covers that lives outside [`World`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extras {
    pub seed: u64,
    pub phase: Phase,
    pub next_event_seq: u64,
    /// Team A, then team B.
    pub boards: [Board; 2],
    pub radio: Vec<RadioMsg>,
    pub handlers: Vec<PendingHandler>,
    /// Per team: module name and SHA-256 of its source. Order is irrelevant;
    /// the encoder sorts by name.
    pub libraries: [Vec<(String, [u8; 32])>; 2],
    /// Order is irrelevant; the encoder sorts by (kind, drone id).
    pub rng: Vec<(StreamId, RngState)>,
}

/// Lowercase hex, as in `expected/hashes.json`.
pub fn to_hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

struct Enc(Vec<u8>);

impl Enc {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u128(&mut self, v: u128) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// Sequence length prefix. Every sequence is capped far below `u32::MAX`
    /// by the ruleset, so saturating here is unreachable in practice.
    fn count(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn bytes(&mut self, b: &[u8]) {
        self.count(b.len());
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn value(&mut self, v: &[u8]) {
        if v.is_empty() {
            self.u8(0x00);
        } else {
            self.0.extend_from_slice(v);
        }
    }
    fn pos(&mut self, p: Pos) {
        self.u16(p.x);
        self.u16(p.y);
    }
}

fn team(t: Team) -> u8 {
    match t {
        Team::A => 0,
        Team::B => 1,
        Team::Neutral => 2,
    }
}

fn terrain(t: Terrain) -> u8 {
    match t {
        Terrain::Open => 0,
        Terrain::Wall => 1,
        Terrain::Well => 2,
        Terrain::Relic => 3,
        Terrain::Forge => 4,
        Terrain::Core => 5,
    }
}

fn structure_kind(k: StructureKind) -> u8 {
    match k {
        StructureKind::Barrier => 0,
        StructureKind::Relay => 1,
        StructureKind::Battery => 2,
        StructureKind::Sentinel => 3,
        StructureKind::FirewallNode => 4,
        StructureKind::Forge => 5,
    }
}

fn relic_kind(k: RelicKind) -> u8 {
    match k {
        RelicKind::Oracle => 0,
        RelicKind::Surplus => 1,
        RelicKind::ComputeNode => 2,
        RelicKind::Lens => 3,
        RelicKind::Wellspring => 4,
    }
}

fn phase(p: Phase) -> u8 {
    match p {
        Phase::Deployed => 0,
        Phase::InClash => 1,
        Phase::EpochOver => 2,
        Phase::MatchOver => 3,
    }
}

/// (kind, drone id) sort key and encoding; Mapgen < TieBreak < Drone(id).
fn stream_key(s: StreamId) -> (u8, u64) {
    match s {
        StreamId::Mapgen => (0, 0),
        StreamId::TieBreak => (1, 0),
        StreamId::Drone(id) => (2, id),
    }
}

impl World {
    /// The canonical bytes the state hash is computed over.
    pub fn canonical_bytes(&self, x: &Extras) -> Vec<u8> {
        let mut e = Enc(Vec::new());
        e.u8(FORMAT_VERSION);
        e.u64(x.seed);
        e.u32(self.grid.side());
        e.u32(self.epoch);
        e.u64(self.tick);
        e.u8(phase(x.phase));
        e.u32(self.next_id);
        e.u64(x.next_event_seq);
        e.u64(self.energy[0]);
        e.u64(self.energy[1]);

        for (pos, cell) in self.grid.iter() {
            e.u8(terrain(cell.terrain));
            e.u8(team(cell.owner));
            e.u8(cell.integrity);
            // In bounds by construction of `iter`.
            let marks = self.grid.marks(pos).unwrap_or(&[]);
            e.u8(u8::try_from(marks.len()).unwrap_or(u8::MAX));
            for m in marks {
                e.bytes(&m.key);
                e.value(&m.value);
                e.u64(m.expires_tick);
                e.u8(team(m.writer));
            }
        }

        e.count(self.entities.len());
        for (id, entity) in &self.entities {
            e.u32(id.0);
            match entity {
                Entity::Drone(d) => {
                    e.u8(0);
                    e.u8(team(d.team));
                    e.pos(d.pos);
                    e.u32(d.hp);
                    e.u32(d.firewall);
                    e.u32(d.energy);
                    e.str(&d.module);
                    e.value(&d.mem);
                }
                Entity::Structure(s) => {
                    e.u8(1);
                    e.u8(structure_kind(s.kind));
                    e.u8(team(s.team));
                    e.pos(s.pos);
                    e.u32(s.hp);
                    e.u32(s.firewall);
                    e.u32(s.stored_energy);
                    match &s.module {
                        Some(m) => {
                            e.u8(1);
                            e.str(m);
                            e.value(&s.mem);
                        }
                        None => e.u8(0),
                    }
                }
                Entity::Core(c) => {
                    e.u8(2);
                    e.u8(team(c.team));
                    e.pos(c.pos);
                    e.u32(c.hp);
                    e.u32(c.firewall);
                }
                Entity::Relic(r) => {
                    e.u8(3);
                    e.u8(relic_kind(r.kind));
                    e.pos(r.pos);
                    e.u32(r.hp);
                }
            }
        }

        for board in &x.boards {
            e.count(board.entries.len());
            for (k, v) in &board.entries {
                e.str(k);
                e.value(v);
            }
            let mut reserved: Vec<&String> = board.reserved.iter().collect();
            reserved.sort();
            e.count(reserved.len());
            for k in reserved {
                e.str(k);
            }
        }

        e.count(x.radio.len());
        for m in &x.radio {
            e.u32(m.sender.0);
            e.pos(m.pos);
            e.str(&m.content);
            e.u32(m.range);
            e.u64(m.deliver_tick);
        }

        // Stable sort: ascending target, emission order within a target.
        let mut handlers = x.handlers.clone();
        handlers.sort_by_key(|h| h.target);
        e.count(handlers.len());
        for h in handlers {
            e.u32(h.target.0);
            e.u8(match h.handler {
                HandlerKind::OnHit => 0,
                HandlerKind::OnHacked => 1,
            });
            e.u32(h.attacker.0);
        }

        for lib in &x.libraries {
            let mut mods: Vec<&(String, [u8; 32])> = lib.iter().collect();
            mods.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            e.count(mods.len());
            for (name, digest) in mods {
                e.str(name);
                e.0.extend_from_slice(digest);
            }
        }

        let mut rng = x.rng.clone();
        rng.sort_by_key(|(s, _)| stream_key(*s));
        e.count(rng.len());
        for (stream, state) in rng {
            let (kind, id) = stream_key(stream);
            e.u8(kind);
            e.u64(id);
            e.u128(state.word_pos);
        }
        e.0
    }

    /// SHA-256 of [`World::canonical_bytes`].
    pub fn state_hash(&self, extras: &Extras) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes(extras)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::entity::{Core, Drone, Relic, Structure};
    use crate::world::grid::{Cell, Grid, Mark};
    use std::collections::BTreeMap;

    fn p(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }

    /// The contract's section 6 example: 2x2 board, one drone, one mark.
    fn example() -> (World, Extras) {
        let mut grid = Grid::new(2);
        grid.set(
            p(1, 0),
            Cell {
                terrain: Terrain::Wall,
                owner: Team::A,
                integrity: 2,
            },
        )
        .unwrap();
        let mut int7 = vec![0x02];
        int7.extend_from_slice(&7i64.to_le_bytes());
        grid.set_marks(
            p(1, 0),
            vec![Mark {
                key: b"k".to_vec(),
                value: int7,
                expires_tick: 10,
                writer: Team::B,
            }],
        )
        .unwrap();
        let mut entities = BTreeMap::new();
        entities.insert(
            EntityId(0),
            Entity::Drone(Drone {
                team: Team::A,
                pos: p(0, 1),
                hp: 10,
                firewall: 3,
                energy: 0,
                mem: Vec::new(),
                module: "scout".into(),
            }),
        );
        let world = World {
            grid,
            tick: 0,
            epoch: 1,
            entities,
            occupancy: BTreeMap::new(),
            next_id: 1,
            energy: [400, 400],
            stats: crate::world::UnitStats {
                drone_hp: 0,
                drone_firewall: 0,
                core_hp: 0,
                core_firewall: 0,
            },
        };
        let extras = Extras {
            seed: 42,
            rng: vec![
                (StreamId::Drone(0), RngState { word_pos: 0 }),
                (StreamId::TieBreak, RngState { word_pos: 5 }),
                (StreamId::Mapgen, RngState { word_pos: 0 }),
            ],
            ..Extras::default()
        };
        (world, extras)
    }

    // The hex lines from spec/contracts/state-hash.md section 6, in order.
    const EXAMPLE_HEX: &[&str] = &[
        "01",
        "2a00000000000000",
        "02000000",
        "01000000",
        "0000000000000000",
        "00",
        "01000000",
        "0000000000000000",
        "9001000000000000",
        "9001000000000000",
        "00020100",
        "01000201",
        "010000006b",
        "020700000000000000",
        "0a00000000000000",
        "01",
        "00020100",
        "00020100",
        "01000000",
        "00000000000000000100",
        "0a000000030000000000000005000000",
        "73636f757400",
        "0000000000000000",
        "0000000000000000",
        "00000000",
        "00000000",
        "00000000",
        "00000000",
        "03000000",
        "00000000000000000000000000000000000000000000000000",
        "01000000000000000005000000000000000000000000000000",
        "02000000000000000000000000000000000000000000000000",
    ];

    #[test]
    fn contract_worked_example_matches_bytes_and_hash() {
        let (w, x) = example();
        let expected: Vec<u8> = EXAMPLE_HEX
            .iter()
            .flat_map(|h| {
                (0..h.len() / 2).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
            })
            .collect();
        assert_eq!(expected.len(), 240);
        assert_eq!(w.canonical_bytes(&x), expected);
        assert_eq!(
            to_hex(&w.state_hash(&x)),
            "7fe1d33cd90de2299ac1a756d3776669d1a347fbb4f2e376d6bc797e81c1f4b6"
        );
    }

    #[test]
    fn changing_any_hashed_field_changes_the_hash() {
        let (w0, x0) = example();
        let base = w0.state_hash(&x0);
        type Mutation = Box<dyn Fn(&mut World, &mut Extras)>;
        let drone = |w: &mut World, f: &dyn Fn(&mut Drone)| {
            if let Some(Entity::Drone(d)) = w.entities.get_mut(&EntityId(0)) {
                f(d);
            }
        };
        let mutations: Vec<(&str, Mutation)> = vec![
            ("seed", Box::new(|_, x| x.seed = 43)),
            ("phase", Box::new(|_, x| x.phase = Phase::InClash)),
            ("next_event_seq", Box::new(|_, x| x.next_event_seq = 1)),
            ("epoch", Box::new(|w, _| w.epoch = 2)),
            ("tick", Box::new(|w, _| w.tick = 1)),
            ("next_id", Box::new(|w, _| w.next_id = 2)),
            ("energy A", Box::new(|w, _| w.energy[0] = 399)),
            ("energy B", Box::new(|w, _| w.energy[1] = 399)),
            (
                "cell owner",
                Box::new(|w, _| {
                    let mut c = w.grid.get(p(0, 0)).unwrap();
                    c.owner = Team::B;
                    w.grid.set(p(0, 0), c).unwrap();
                }),
            ),
            (
                "cell integrity",
                Box::new(|w, _| {
                    let mut c = w.grid.get(p(1, 0)).unwrap();
                    c.integrity = 3;
                    w.grid.set(p(1, 0), c).unwrap();
                }),
            ),
            (
                "mark expiry",
                Box::new(|w, _| {
                    let mut m = w.grid.marks(p(1, 0)).unwrap().to_vec();
                    m[0].expires_tick = 11;
                    w.grid.set_marks(p(1, 0), m).unwrap();
                }),
            ),
            ("drone hp", Box::new(move |w, _| drone(w, &|d| d.hp = 9))),
            (
                "drone module",
                Box::new(move |w, _| drone(w, &|d| d.module = "warden".into())),
            ),
            (
                "drone mem",
                Box::new(move |w, _| drone(w, &|d| d.mem = vec![0x01, 1])),
            ),
            (
                "extra core",
                Box::new(|w, _| {
                    w.entities.insert(
                        EntityId(1),
                        Entity::Core(Core {
                            team: Team::B,
                            pos: p(1, 1),
                            hp: 1,
                            firewall: 1,
                        }),
                    );
                }),
            ),
            (
                "extra relic",
                Box::new(|w, _| {
                    w.entities.insert(
                        EntityId(1),
                        Entity::Relic(Relic {
                            kind: RelicKind::Lens,
                            pos: p(1, 1),
                            hp: 30,
                        }),
                    );
                }),
            ),
            (
                "extra structure",
                Box::new(|w, _| {
                    w.entities.insert(
                        EntityId(1),
                        Entity::Structure(Structure {
                            kind: StructureKind::Battery,
                            team: Team::A,
                            pos: p(1, 1),
                            hp: 5,
                            firewall: 0,
                            stored_energy: 9,
                            module: None,
                            mem: Vec::new(),
                        }),
                    );
                }),
            ),
            (
                "board entry",
                Box::new(|_, x| {
                    x.boards[1]
                        .entries
                        .push(("k".into(), vec![0x02, 1, 0, 0, 0, 0, 0, 0, 0]))
                }),
            ),
            (
                "board reserved",
                Box::new(|_, x| x.boards[0].reserved.push("home".into())),
            ),
            (
                "radio",
                Box::new(|_, x| {
                    x.radio.push(RadioMsg {
                        sender: EntityId(0),
                        pos: p(0, 1),
                        content: "hi".into(),
                        range: 8,
                        deliver_tick: 1,
                    })
                }),
            ),
            (
                "handler",
                Box::new(|_, x| {
                    x.handlers.push(PendingHandler {
                        target: EntityId(0),
                        handler: HandlerKind::OnHit,
                        attacker: EntityId(1),
                    })
                }),
            ),
            (
                "library",
                Box::new(|_, x| x.libraries[0].push(("scout".into(), [7; 32]))),
            ),
            ("rng position", Box::new(|_, x| x.rng[1].1.word_pos = 6)),
        ];
        for (name, mutate) in mutations {
            let (mut w, mut x) = example();
            mutate(&mut w, &mut x);
            assert_ne!(w.state_hash(&x), base, "{name} did not change the hash");
        }
    }

    #[test]
    fn unhashed_differences_do_not_change_the_hash() {
        let (w0, x0) = example();
        let base = w0.state_hash(&x0);

        // Chunk allocation and Arc sharing are not state: writing then
        // clearing a cell in another chunk, or snapshotting, changes nothing.
        let mut big = example().0;
        big.grid = {
            let mut g = Grid::new(2);
            g.set(
                p(1, 1),
                Cell {
                    terrain: Terrain::Wall,
                    ..Cell::default()
                },
            )
            .unwrap();
            g.set(p(1, 1), Cell::default()).unwrap();
            g.set(p(1, 0), w0.grid.get(p(1, 0)).unwrap()).unwrap();
            g.set_marks(p(1, 0), w0.grid.marks(p(1, 0)).unwrap().to_vec())
                .unwrap();
            g
        };
        assert_eq!(big.state_hash(&x0), base);
        assert_eq!(w0.clone().state_hash(&x0), base);
        assert_eq!(w0.grid.snapshot().iter().count(), 4);

        // The per-world unit-stat cache is ruleset-derived, not state.
        let mut w = example().0;
        w.stats.drone_hp = 99;
        assert_eq!(w.state_hash(&x0), base);
    }

    #[test]
    fn equal_state_hashes_equally_regardless_of_construction_order() {
        let mk = |ids: &[u32]| {
            let (mut w, mut x) = example();
            for &i in ids {
                w.entities.insert(
                    EntityId(i),
                    Entity::Relic(Relic {
                        kind: RelicKind::Oracle,
                        pos: p(1, 1),
                        hp: i,
                    }),
                );
            }
            w.next_id = 10;
            x.libraries[1] = vec![("b".into(), [2; 32]), ("a".into(), [1; 32])];
            x.boards[0].reserved = vec!["y".into(), "x".into()];
            x.handlers = vec![
                PendingHandler {
                    target: EntityId(3),
                    handler: HandlerKind::OnHit,
                    attacker: EntityId(0),
                },
                PendingHandler {
                    target: EntityId(1),
                    handler: HandlerKind::OnHacked,
                    attacker: EntityId(0),
                },
            ];
            (w, x)
        };
        let (w1, x1) = mk(&[1, 2, 3]);
        let (w2, mut x2) = mk(&[3, 1, 2]);
        x2.libraries[1].reverse();
        x2.boards[0].reserved.reverse();
        x2.rng.reverse();
        x2.handlers.reverse();
        // Handlers reversed across different targets is the same set; only
        // same-target order matters, and these two have different targets.
        assert_eq!(w1.state_hash(&x1), w2.state_hash(&x2));
        // Same-target emission order is part of the state.
        let mut x3 = x1.clone();
        x3.handlers = vec![
            PendingHandler {
                target: EntityId(1),
                handler: HandlerKind::OnHit,
                attacker: EntityId(0),
            },
            PendingHandler {
                target: EntityId(1),
                handler: HandlerKind::OnHacked,
                attacker: EntityId(0),
            },
        ];
        let mut x4 = x3.clone();
        x4.handlers.reverse();
        assert_ne!(w1.state_hash(&x3), w1.state_hash(&x4));
    }
}
