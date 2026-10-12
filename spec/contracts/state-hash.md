# State hash: canonical serialization (format version 1)

**Status:** proposed for M1. **Implemented by:** `lattice-engine` state hash (#51). **Consumed by:** `Match::state_hash` and `Snapshot` ([`engine-api.md`](engine-api.md) §4), `lattice replay --verify`, and `expected/hashes.json` in `tests/golden/`.

The state hash is `SHA-256(canonical_bytes(state))`. `canonical_bytes` is defined here byte for byte, so two independent implementations (native, WASM, a Python checker) produce the same hash for the same state. The format must not depend on the host: no host floats, no hash-map order, no pointer or allocation detail, no platform endianness.

## 1. What is hashed

Exactly the following state, in exactly this order (§4 gives the section layout). Anything that can change a future tick's behaviour or a future event must be hashed; anything that cannot must not be.

**Hashed**

- Match seed, board side length, epoch (1-based), tick, phase, next entity id, next event `seq`.
- Team energy pools (A, B).
- Every grid cell: terrain, owner, integrity, and its marks (key, value, expiry tick, writer team) by slot.
- Every entity, by ascending id: drones (id, team, position, hp, firewall, carried energy, module name, `me.mem`), structures (kind, team, position, hp, firewall, stored energy, and for Sentinels the module and `mem`), Cores, Relics.
- Each team's blackboard (entries in insertion order, plus its reserved keys).
- Pending radio messages (sent, not yet delivered).
- Pending `on_hit` and `on_hacked` handler events (caused in tick N, delivered in tick N+1). `on_radio` deliveries are not hashed separately: they are derived from the pending radio messages above.
- Each team's deployed library: module names with the SHA-256 of each module's source.
- RNG stream positions: `Mapgen`, `TieBreak`, and one stream per live drone.

**Not hashed**

- `match_id`, `EngineOptions` (thread count), and anything else the caller supplies that has no simulation meaning.
- Telemetry, event logs, digests, chat, and any per-seat views.
- Fuel used (it resets each tick), compiled bytecode and other caches, interpreter scratch state between ticks.
- Chunk allocation and `Arc` sharing: an absent chunk hashes the same as an allocated chunk of default cells.
- Ruleset values (a mismatched ruleset is caught by version; see `engine-api.md` §4).

If a world-model field is added that affects behaviour, it must be added here and the version byte bumped (§5).

## 2. Primitive encoding

All multi-byte integers are **little-endian**. There is no padding or alignment.

| Primitive | Encoding |
|---|---|
| `u8`, `u16`, `u32`, `u64` | 1, 2, 4, 8 bytes |
| `u128` | 16 bytes |
| `i64` | 8 bytes, two's complement |
| `bool` | 1 byte: `0x00` false, `0x01` true |
| `str` | `u32` byte length, then the UTF-8 bytes. No terminator; no normalization. |
| sequence | `u32` element count, then the elements in the stated order |
| `sha256` | the 32 raw digest bytes |

Enum codes (a single `u8`):

| Enum | Codes |
|---|---|
| Team | `A`=0, `B`=1, `Neutral`=2 |
| Terrain | `Open`=0, `Wall`=1, `Well`=2, `Relic`=3, `Forge`=4, `Core`=5 |
| Phase | `Deployed`=0, `InClash`=1, `EpochOver`=2, `MatchOver`=3 |
| Entity kind | `Drone`=0, `Structure`=1, `Core`=2, `Relic`=3 |
| Structure kind | `Barrier`=0, `Relay`=1, `Battery`=2, `Sentinel`=3, `FirewallNode`=4, `Forge`=5 |
| Relic kind | `Oracle`=0, `Surplus`=1, `ComputeNode`=2, `Lens`=3, `Wellspring`=4 |
| RNG stream kind | `Mapgen`=0, `TieBreak`=1, `Drone`=2 |

### LatticePy values (`me.mem`, marks, blackboard)

A value is a one-byte tag followed by its payload, recursively (`spec/latticepy.md` §3):

| Tag | Type | Payload |
|---|---|---|
| `0x00` | `None` | none |
| `0x01` | `bool` | `u8` 0 or 1 |
| `0x02` | `int` | `i64` |
| `0x03` | software float `F` | `u64`: the IEEE 754 binary64 bit pattern (sign, 11-bit exponent, 52-bit fraction), exactly as the VM holds it. `-0.0` and `+0.0` are distinct. NaN and Inf cannot occur (`latticepy.md` §4). |
| `0x04` | `str` | `str` |
| `0x05` | `list` | sequence of values |
| `0x06` | `tuple` | sequence of values |
| `0x07` | `dict` | `u32` pair count, then each pair as `value key`, `value value`, **in insertion order** |

Mutable containers are hashed by content; aliasing between values is not preserved or represented.

## 3. Ordering

- Cells: row-major from (0, 0): `y` ascending, then `x` ascending, covering every cell of the board including defaults.
- Marks within a cell: in the order the engine holds them (write order, at most 4), so the count is the number of marks present. That order is part of the state.
- Entities: ascending `EntityId`.
- Dicts: insertion order. Blackboard reserved keys: bytewise ascending (they are a set).
- Pending radio messages: queue order, i.e. the order the resolve phase emitted them. A message whose sender is destroyed before delivery is still hashed, by sender id (ids are never reused).
- Pending handler events: ascending target entity id, then the order the resolve phase emitted them for that target (the order handlers run in).
- Libraries: modules ascending by name (bytewise).
- RNG streams: by (stream kind, drone id) ascending, which is `Mapgen`, `TieBreak`, then drones by ascending id.

No encoder may iterate a `HashMap` or `HashSet`, or any container whose order depends on addresses, hashes or thread scheduling.

## 4. Layout

```
u8   format version                  (= 1)
u64  match seed
u32  board side
u32  epoch                           (1-based)
u64  tick
u8   phase
u32  next entity id
u64  next event seq
u64  energy pool A
u64  energy pool B
cells        side*side times:
    u8 terrain, u8 owner, u8 integrity, u8 mark count,
    then per mark: str key, value value, u64 expires_tick, u8 writer team
entities     u32 count, then per entity (ascending id):
    u32 id, u8 entity kind, then by kind:
      Drone:     u8 team, u16 x, u16 y, u32 hp, u32 firewall, u32 energy, str module, value mem
      Structure: u8 structure kind, u8 team, u16 x, u16 y, u32 hp, u32 firewall,
                 u32 stored energy,
                 u8 has_module (0|1), and if 1: str module, value mem
      Core:      u8 team, u16 x, u16 y, u32 hp, u32 firewall        (x, y = centre cell)
      Relic:     u8 relic kind, u16 x, u16 y, u32 hp
boards       for team A then team B:
    u32 entry count, per entry: str key, value value   (insertion order)
    u32 reserved count, per key: str                   (ascending)
radio        u32 count, per message (queue order):
    u32 sender id, u16 x, u16 y, str content, u32 range, u64 deliver_tick
handlers     u32 count, per pending event (ascending target id, then emission order):
    u32 target id, u8 handler (OnHit=0, OnHacked=1), u32 attacker id
libraries    for team A then team B:
    u32 module count, per module (ascending name): str name, 32 bytes sha256(source UTF-8)
rng          u32 count, per stream (ascending (kind, id)):
    u8 kind, u64 drone id (0 for Mapgen and TieBreak), u128 word position
```

Notes:

- A drone's RNG stream keys on its drone id here, not on the combined integer stream value, so a change in how `rng.rs` derives that integer cannot silently alias two streams in the hash (see #83). The stream *position* is the ChaCha word position from `RngState`.
- `Structure.stored energy` is non-zero only for Batteries. The `has_module` branch is for Sentinels. The world-model structs do not carry these fields yet; the hash implementation PR (#51) adds them, with the gameplay PRs that own them.
- The mark `value` is a LatticePy value (§2). The world model currently stores marks as opaque bytes; the VM PR replaces that with the value type before the hash is implemented.

## 5. Versioning

The first byte is the format version. Version 1 is this document. Any change to what is hashed or how it is encoded gets a new version number and a new section in this file; old sections stay so old golden fixtures remain checkable. A hash produced under one version is never comparable to one from another; `expected/hashes.json` records the version in `serialization_contract` (for example `"spec/contracts/state-hash.md#v1"`). Changing a golden hash is a behaviour commitment and needs human approval (AGENTS.md §8).

## 6. Worked example

A deliberately tiny state, with illustrative numbers (not ruleset values): seed 42, a 2×2 board, epoch 1, tick 0, phase `Deployed`, energy 400 for both teams, empty blackboards, radio queue, handler queue and libraries.

- Cell (1, 0): `Wall`, owner `A`, integrity 2, one mark: key `"k"`, value `int 7`, expiry tick 10, writer `B`.
- The other three cells are defaults: `Open`, `Neutral`, integrity 1, no marks.
- One drone, id 0: team `A`, at (0, 1), hp 10, firewall 3, carried energy 0, module `"scout"`, `me.mem = None`.
- RNG streams: `Mapgen` at word position 0, `TieBreak` at 5, and `Drone 0` at 0.

Canonical bytes, by section (spaces are for reading only; the real stream has none):

```
version          01
seed             2a00000000000000
side             02000000
epoch            01000000
tick             0000000000000000
phase            00
next entity id   01000000
next event seq   0000000000000000
energy A         9001000000000000
energy B         9001000000000000
cell (0,0)       00 02 01 00
cell (1,0)       01 00 02 01  01000000 6b  02 0700000000000000  0a00000000000000  01
cell (0,1)       00 02 01 00
cell (1,1)       00 02 01 00
entity count     01000000
entity 0         00000000 00 00 0000 0100 0a000000 03000000 00000000 05000000 73636f7574 00
board A          00000000 00000000
board B          00000000 00000000
radio            00000000
handlers         00000000
library A        00000000
library B        00000000
rng count        03000000
rng Mapgen       00 0000000000000000 00000000000000000000000000000000
rng TieBreak     01 0000000000000000 05000000000000000000000000000000
rng Drone 0      02 0000000000000000 00000000000000000000000000000000
```

Total length: 240 bytes. SHA-256 of the concatenated bytes:

```
7fe1d33cd90de2299ac1a756d3776669d1a347fbb4f2e376d6bc797e81c1f4b6
```

The bytes above were generated by an independent reference encoder written from this document (not from the engine), so the hash implementation PR (#51) must reproduce this hash in a unit test.
