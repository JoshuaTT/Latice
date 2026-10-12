//! Clash events, in the shape `spec/contracts/events.schema.json` defines, and
//! the sink API from `spec/contracts/engine-api.md` §4.
//!
//! Payload ids are the schema's free-form strings; whoever emits an event
//! (the resolvers, #53–#55) chooses how an `EntityId` is rendered. A sink gets
//! `&Event`, so it cannot change an event or its `seq`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::world::StructureKind as WorldStructureKind;
use crate::world::grid::{Pos, Team as WorldTeam};

/// A cell coordinate as the schema writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: u16,
    pub y: u16,
}

impl From<Pos> for Position {
    fn from(p: Pos) -> Self {
        Position { x: p.x, y: p.y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Team {
    A,
    B,
    Neutral,
}

impl From<WorldTeam> for Team {
    fn from(t: WorldTeam) -> Self {
        match t {
            WorldTeam::A => Team::A,
            WorldTeam::B => Team::B,
            WorldTeam::Neutral => Team::Neutral,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureKind {
    Barrier,
    Relay,
    Battery,
    Sentinel,
    #[serde(rename = "Firewall Node")]
    FirewallNode,
    Forge,
}

impl From<WorldStructureKind> for StructureKind {
    fn from(k: WorldStructureKind) -> Self {
        match k {
            WorldStructureKind::Barrier => StructureKind::Barrier,
            WorldStructureKind::Relay => StructureKind::Relay,
            WorldStructureKind::Battery => StructureKind::Battery,
            WorldStructureKind::Sentinel => StructureKind::Sentinel,
            WorldStructureKind::FirewallNode => StructureKind::FirewallNode,
            WorldStructureKind::Forge => StructureKind::Forge,
        }
    }
}

macro_rules! payloads {
    ($($(#[$m:meta])* $name:ident { $($field:ident: $ty:ty),* $(,)? })*) => {
        $(
            $(#[$m])*
            #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
            #[serde(deny_unknown_fields)]
            pub struct $name { $(pub $field: $ty),* }
        )*
    };
}

payloads! {
    MovePayload { drone_id: String, from: Position, to: Position }
    ClaimPayload { drone_id: String, position: Position, new_owner: Team }
    HarvestPayload { drone_id: String, position: Position, amount: u32 }
    AttackPayload { attacker_id: String, target_id: String, damage: u32, target_hp_after: u32 }
    HackPayload {
        attacker_id: String,
        target_id: String,
        target_firewall_before: u32,
        target_firewall_after: u32,
        payload_module: String,
    }
    PatchPayload { drone_id: String, target_id: String, firewall_restored: u32 }
    BuildPayload { drone_id: String, structure_kind: StructureKind, position: Position }
    SpawnPayload {
        spawner_id: String,
        new_drone_id: String,
        module_name: String,
        position: Position,
        args: Map<String, Value>,
    }
    RepairPayload { drone_id: String, target_id: String, hp_restored: u32, target_hp_after: u32 }
    DepositPayload { drone_id: String, structure_id: String, amount: u32 }
    WithdrawPayload { drone_id: String, structure_id: String, amount: u32 }
    CapturePayload { target_id: String, capturer_id: String, new_owner: Team, module_source: String }
    CoreDamagePayload { core_team: Team, damage: u32, hp_after: u32 }
    StructureDestroyedPayload {
        structure_id: String,
        structure_kind: StructureKind,
        position: Position,
        owner: Team,
    }
    CellFlippedPayload { position: Position, old_owner: Team, new_owner: Team }
    MarkWrittenPayload { drone_id: String, position: Position, key: String, value: String, ttl: u32 }
    RadioSentPayload { sender_id: String, position: Position, content: String, range: u32 }
}

/// One variant per `event_type` in the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type", content = "payload", rename_all = "snake_case")]
pub enum EventKind {
    Move(MovePayload),
    Claim(ClaimPayload),
    Harvest(HarvestPayload),
    Attack(AttackPayload),
    Hack(HackPayload),
    Patch(PatchPayload),
    Build(BuildPayload),
    Spawn(SpawnPayload),
    Repair(RepairPayload),
    Deposit(DepositPayload),
    Withdraw(WithdrawPayload),
    Capture(CapturePayload),
    CoreDamage(CoreDamagePayload),
    StructureDestroyed(StructureDestroyedPayload),
    CellFlipped(CellFlippedPayload),
    MarkWritten(MarkWrittenPayload),
    RadioSent(RadioSentPayload),
}

/// The event envelope (`events.schema.json`). `seq` is per match, assigned by
/// the [`Emitter`] in emission order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub match_id: String,
    /// 1-based.
    pub epoch: u32,
    pub tick: u64,
    pub seq: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    /// Exactly the schema's JSON shape.
    pub fn to_json(&self) -> Value {
        // Serializing these types cannot fail (string keys, plain scalars).
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Receives events as they are emitted. Infallible, and must not influence
/// the simulation.
pub trait EventSink {
    fn emit(&mut self, event: &Event);
}

/// Discards events.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&mut self, _: &Event) {}
}

/// Collects events.
#[derive(Debug, Default)]
pub struct VecSink(pub Vec<Event>);

impl EventSink for VecSink {
    fn emit(&mut self, event: &Event) {
        self.0.push(event.clone());
    }
}

/// Stamps events with the match id and a gap-free `seq`, then hands them to a
/// sink. Owned by the match; `seq` is part of the state hash.
#[derive(Debug, Clone, Default)]
pub struct Emitter {
    match_id: String,
    next_seq: u64,
}

impl Emitter {
    pub fn new(match_id: String) -> Self {
        Emitter {
            match_id,
            next_seq: 0,
        }
    }

    pub fn set_match_id(&mut self, match_id: String) {
        self.match_id = match_id;
    }

    /// The `seq` the next event will get.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Emits `kind` at `(epoch, tick)` with the next `seq`.
    pub fn emit(&mut self, epoch: u32, tick: u64, kind: EventKind, sink: &mut dyn EventSink) {
        let event = Event {
            match_id: self.match_id.clone(),
            epoch,
            tick,
            seq: self.next_seq,
            kind,
        };
        self.next_seq += 1;
        sink.emit(&event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = include_str!("../../spec/contracts/events.schema.json");

    const VALID: [&str; 7] = [
        include_str!("../../spec/contracts/examples/events/valid/capture-chain-1-hack.json"),
        include_str!("../../spec/contracts/examples/events/valid/capture-chain-2-hack.json"),
        include_str!("../../spec/contracts/examples/events/valid/capture-chain-3-capture.json"),
        include_str!("../../spec/contracts/examples/events/valid/full-tick-1-spawn.json"),
        include_str!("../../spec/contracts/examples/events/valid/full-tick-2-build.json"),
        include_str!("../../spec/contracts/examples/events/valid/full-tick-3-attack.json"),
        include_str!("../../spec/contracts/examples/events/valid/full-tick-event-1.json"),
    ];

    const INVALID: [&str; 3] = [
        include_str!("../../spec/contracts/examples/events/invalid/invalid-missing-seq.json"),
        include_str!("../../spec/contracts/examples/events/invalid/invalid-negative-damage.json"),
        include_str!(
            "../../spec/contracts/examples/events/invalid/invalid-unknown-event-type.json"
        ),
    ];

    fn validator() -> jsonschema::Validator {
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    }

    fn pos(x: u16, y: u16) -> Position {
        Position { x, y }
    }

    fn s(v: &str) -> String {
        v.to_owned()
    }

    /// One event of every type, in the schema's priority order.
    fn one_of_each() -> Vec<EventKind> {
        vec![
            EventKind::Build(BuildPayload {
                drone_id: s("d1"),
                structure_kind: StructureKind::FirewallNode,
                position: pos(1, 2),
            }),
            EventKind::Spawn(SpawnPayload {
                spawner_id: s("f1"),
                new_drone_id: s("d2"),
                module_name: s("scout"),
                position: pos(3, 4),
                args: Map::from_iter([(s("k"), Value::from(1))]),
            }),
            EventKind::Repair(RepairPayload {
                drone_id: s("d1"),
                target_id: s("s1"),
                hp_restored: 5,
                target_hp_after: 20,
            }),
            EventKind::Patch(PatchPayload {
                drone_id: s("d1"),
                target_id: s("d2"),
                firewall_restored: 2,
            }),
            EventKind::Deposit(DepositPayload {
                drone_id: s("d1"),
                structure_id: s("b1"),
                amount: 9,
            }),
            EventKind::Withdraw(WithdrawPayload {
                drone_id: s("d1"),
                structure_id: s("b1"),
                amount: 4,
            }),
            EventKind::Harvest(HarvestPayload {
                drone_id: s("d1"),
                position: pos(5, 5),
                amount: 7,
            }),
            EventKind::Claim(ClaimPayload {
                drone_id: s("d1"),
                position: pos(6, 6),
                new_owner: Team::A,
            }),
            EventKind::Move(MovePayload {
                drone_id: s("d1"),
                from: pos(0, 0),
                to: pos(0, 1),
            }),
            EventKind::Attack(AttackPayload {
                attacker_id: s("d1"),
                target_id: s("d9"),
                damage: 3,
                target_hp_after: 17,
            }),
            EventKind::Hack(HackPayload {
                attacker_id: s("d1"),
                target_id: s("d9"),
                target_firewall_before: 10,
                target_firewall_after: 4,
                payload_module: s("worm"),
            }),
            EventKind::Capture(CapturePayload {
                target_id: s("d9"),
                capturer_id: s("d1"),
                new_owner: Team::B,
                module_source: s("p3"),
            }),
            EventKind::RadioSent(RadioSentPayload {
                sender_id: s("d1"),
                position: pos(8, 8),
                content: s("hi"),
                range: 8,
            }),
            EventKind::MarkWritten(MarkWrittenPayload {
                drone_id: s("d1"),
                position: pos(9, 9),
                key: s("k"),
                value: s("v"),
                ttl: 50,
            }),
            EventKind::CoreDamage(CoreDamagePayload {
                core_team: Team::B,
                damage: 12,
                hp_after: 188,
            }),
            EventKind::StructureDestroyed(StructureDestroyedPayload {
                structure_id: s("b1"),
                structure_kind: StructureKind::Barrier,
                position: pos(7, 7),
                owner: Team::Neutral,
            }),
            EventKind::CellFlipped(CellFlippedPayload {
                position: pos(2, 2),
                old_owner: Team::A,
                new_owner: Team::B,
            }),
        ]
    }

    #[test]
    fn every_event_type_validates_against_the_schema_and_round_trips() {
        let v = validator();
        let kinds = one_of_each();
        assert_eq!(kinds.len(), 17);
        let mut seen = Vec::new();
        for (seq, kind) in kinds.into_iter().enumerate() {
            let event = Event {
                match_id: s("m"),
                epoch: 2,
                tick: 10,
                seq: seq as u64,
                kind,
            };
            let json = event.to_json();
            assert!(v.is_valid(&json), "invalid: {json}");
            assert_eq!(
                serde_json::from_value::<Event>(json.clone()).unwrap(),
                event
            );
            seen.push(json["event_type"].as_str().unwrap().to_owned());
        }
        // Every enum value the schema allows has a variant.
        let schema: Value = serde_json::from_str(SCHEMA).unwrap();
        let mut allowed: Vec<String> = schema["properties"]["event_type"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e.as_str().unwrap().to_owned())
            .collect();
        allowed.sort();
        seen.sort();
        assert_eq!(seen, allowed);
    }

    #[test]
    fn schema_valid_examples_deserialize_and_re_serialize_equal() {
        let v = validator();
        for text in VALID {
            let json: Value = serde_json::from_str(text).unwrap();
            assert!(v.is_valid(&json));
            let event: Event = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(event.to_json(), json);
        }
    }

    #[test]
    fn schema_invalid_examples_are_rejected_by_schema_and_types() {
        let v = validator();
        for text in INVALID {
            let json: Value = serde_json::from_str(text).unwrap();
            assert!(!v.is_valid(&json), "{json}");
            assert!(serde_json::from_value::<Event>(json).is_err());
        }
    }

    #[test]
    fn emitter_assigns_gap_free_seq_in_emission_order() {
        let mut emitter = Emitter::new(s("m"));
        let mut sink = VecSink::default();
        for kind in one_of_each() {
            emitter.emit(1, 0, kind, &mut sink);
        }
        let seqs: Vec<u64> = sink.0.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (0..17).collect::<Vec<_>>());
        assert_eq!(emitter.next_seq(), 17);
        // Emission order is preserved, not re-sorted by type.
        assert!(matches!(sink.0[0].kind, EventKind::Build(_)));
        assert!(matches!(sink.0[16].kind, EventKind::CellFlipped(_)));
        assert!(sink.0.iter().all(|e| e.match_id == "m"));
    }

    #[test]
    fn sink_cannot_change_seq_and_null_sink_still_advances_it() {
        // A sink only ever sees `&Event`; this one tries to be unhelpful and
        // the emitter's numbering is unaffected.
        struct Counting(usize);
        impl EventSink for Counting {
            fn emit(&mut self, _: &Event) {
                self.0 += 1;
            }
        }
        let mut emitter = Emitter::new(s("m"));
        let mut counting = Counting(0);
        let kind = || {
            EventKind::CoreDamage(CoreDamagePayload {
                core_team: Team::A,
                damage: 1,
                hp_after: 1,
            })
        };
        emitter.emit(1, 0, kind(), &mut counting);
        emitter.emit(1, 0, kind(), &mut NullSink);
        let mut vec = VecSink::default();
        emitter.emit(1, 1, kind(), &mut vec);
        assert_eq!((counting.0, vec.0[0].seq), (1, 2));
    }

    #[test]
    fn event_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Event>();
    }

    #[test]
    fn conversions_from_world_types() {
        assert_eq!(
            StructureKind::from(WorldStructureKind::FirewallNode),
            StructureKind::FirewallNode
        );
        assert_eq!(
            serde_json::to_value(StructureKind::FirewallNode).unwrap(),
            "Firewall Node"
        );
        assert_eq!(Team::from(WorldTeam::Neutral), Team::Neutral);
        assert_eq!(Position::from(Pos { x: 3, y: 4 }), pos(3, 4));
    }
}
