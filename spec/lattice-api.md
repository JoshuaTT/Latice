# The `lattice` library — API reference

LatticePy modules see the game world only through the `lattice` library
(game-design §5.7). This document is the exact reference for every name
`from lattice import *` binds: its signature, kind, fuel cost, return value,
and failure behaviour. The checker (what calls are legal), the VM's object
model for `me`/`team`/`seen`, the stdlib bindings (#56), and the house bots
(#57) build against this file. Fuel numbers are ruleset keys read at runtime
(§13); no value in this document is hard-coded in the engine.

Style follows `spec/latticepy.md`: everything not written here does not
exist. Where the two documents overlap, `latticepy.md` owns the language
(values, fuel metering, error containment) and this document owns the
library.

---

## 0. How to read an entry

Each entry states five things:

- **Signature** — parameter types, written in LatticePy value types
  (`int`, `str`, `bool`, `dict`, `Pos`, `Dir`, `F`, views).
- **Kind** — one of:
  - **act** — queues the tick's one act. Nothing executes in the think
    phase; the resolver decides the outcome (game-design §5.3;
    latticepy §7). Resolved in the order fixed by `events.schema.json`.
  - **query** — reads (or lightly writes) the world; costs fuel, consumes
    no act slot and no energy.
  - **free op** — pure computation; library cost 0.
- **Fuel** — the *library cost*, added on top of the AST ops the call
  itself evaluates (latticepy §5: the `Call` node, its argument nodes,
  `Attribute` nodes). Library calls do **not** pay `call_overhead`; that
  overhead is for module functions. Fuel is spent when the call starts,
  whether or not it succeeds.
- **Return** — what a successful call yields.
- **Failure** — always one of three modes:
  1. **soft** — the result is legitimately unavailable: the call returns
     `None`, `False` or `[]` and nothing else happens. Out-of-sight
     positions, a full blackboard, an already-spent rate limit, no path.
  2. **`cap_violation`** — the player tried to build a value over a cap
     (string length, board value size); the drone stops for the tick
     (latticepy §8).
  3. **`bad_call`** — an argument has the wrong type or is semantically
     invalid (`Pos` where a `Dir` is needed, an unknown structure kind);
     the drone stops for the tick. A bug in player code should be loud.

Soft failure is the game (fog, contention); the two error modes are bugs in
the caller's code and are reported in the owner's telemetry.

**The act slot.** An act call returns `True` **iff it queued an act**. Only
a queued act consumes the tick's one-act slot: a call that returns `False`
(soft failure, or an act already queued this tick) queues nothing, so later
calls in the same tick can still queue (latticepy §7: first act wins).
`True` means queued, not succeeded — the resolver may still make the act
fail (a blocked move, an over-cap spawn).

**Grid and time.** `x` grows east, `y` grows north. Distances are Chebyshev
(`max(|dx|, |dy|)`) unless stated. `me.now` is ticks since match start,
monotonic, never reset (also across epochs). Handlers run against the
read-only snapshot of tick start (latticepy §7); every view and list this
library returns is from that snapshot and in a deterministic order.

**Value objects.** `Pos`, `Dir`, `F`, `team`, `team.board`, `me.radio`, the
`seen` view, and every entity/cell view are the "allowed value objects" of
latticepy §1.3: attribute access on them is legal. `Pos`, `Dir`, and `F`
values may be stored in `me.mem`, lists, and blackboard payloads as far
as caps allow — but `Pos`, `Dir`, and views are **not** legal dict keys
(latticepy §3; use `(x, y)` tuples). Views are legal only within the tick
that produced them: storing one in `me.mem` or a blackboard value is a
`bad_call` (a view is a handle into that tick's snapshot, not data).

---

## 1. Values

### 1.1 `Pos`

```text
Pos(x: int, y: int) -> Pos      free op
```

Immutable cell coordinate. Fields `.x`, `.y` (`int`, plain reads). `==` is
value equality. Not a legal dict key; the idiom for map keys is the
`(x, y)` tuple. A `Pos` off the board is constructible and comparable;
calls that would use it fail soft (e.g. `me.cell(off)` is `None`).

### 1.2 Directions

Eight `Dir` constants:

| Constant | offset | Constant | offset |
|---|---|---|---|
| `NORTH` | (0, 1) | `SOUTH` | (0, −1) |
| `NORTHEAST` | (1, 1) | `SOUTHWEST` | (−1, −1) |
| `EAST` | (1, 0) | `WEST` | (−1, 0) |
| `SOUTHEAST` | (1, −1) | `NORTHWEST` | (−1, 1) |

`Dir` values compare by `==`, and are storable anywhere a value fits
(`me.mem`, lists, blackboard values), but are not legal dict keys.

```text
turn_left(d: Dir) -> Dir        free op   # 45° counterclockwise
turn_right(d: Dir) -> Dir       free op   # 45° clockwise
```

Failure: `bad_call` if the argument is not a `Dir`.

The **direction order** `NORTH, NORTHEAST, EAST, SOUTHEAST, SOUTH,
SOUTHWEST, WEST, NORTHWEST` is the fixed tiebreak order used throughout
this library (§5 `step_toward`, §11 `kite`).

### 1.3 `F` — the software float

`F` is the namespace object of latticepy §4 (the software IEEE 754
double): `F.add`, `F.sub`, `F.mul`, `F.div`, `F.lt`, `F.le`, `F.gt`,
`F.ge`, `F.eq`, `F.ne`, `F.from_int`, `F.to_int`, `F.to_str(f, digits)`.
Representation, rounding, and `div_by_zero` are latticepy §4's; they are
not repeated here. Fuel: each `F` op has library cost `float_op` (a
latticepy §5 tunable). Failure: `bad_call` on a non-`F` argument.

---

## 2. The `me` object

All fields are plain reads (1 fuel each, latticepy §5) and views: assign to
`me.mem`, never to a field.

| Field | Type | Meaning |
|---|---|---|
| `me.pos` | `Pos` | position at tick start |
| `me.hp` | `int` | current HP (starts at the ruleset's drone HP) |
| `me.firewall` | `int` | current firewall (starts at the ruleset's drone firewall) |
| `me.energy` | `int` | energy currently carried (0 .. carry cap, game-design §5.2) |
| `me.now` | `int` | ticks since match start (§0) |
| `me.id` | `str` | stable drone id, fixed for the drone's lifetime |
| `me.team` | `str` | `"A"` or `"B"` |
| `me.mem` | `dict` | the capped persistent memory (latticepy §6, §7; resets on hack) |

`team` (the object used as `team.energy()`, `team.board`) is a module-level
export bound once per match to the calling drone's team.

---

## 3. Views

Read-only snapshot objects (§0). Fields are plain reads.

**Drone view** — returned by sensing (`seen.enemies()`, `seen.allies()`,
`me.nearest_enemy`): `.id` (`str`), `.pos` (`Pos`), `.hp` (`int`),
`.firewall` (`int`), `.team` (`"A"`/`"B"`). Ally views *additionally*
expose `.energy` (`int`, carried) and `.module` (`str`, module name).
Enemy views expose neither: which module an enemy runs is capture intel
(game-design §5.6).

**Structure view** — returned by `seen.structures()`: `.kind` (`str`:
`"Barrier"`, `"Relay"`, `"Battery"`, `"Sentinel"`, `"Firewall Node"`,
`"Forge"` — the `StructureKind` enum of `events.schema.json`), `.pos`,
`.hp` (`int`), `.firewall` (`int`, `0` if not hackable), `.owner`
(`"A"`/`"B"`/`"Neutral"`), `.stored` (`int`; Battery energy, else `0`).
Cores are not structures; read a Core's cell with `me.cell`.

**Cell view** — returned by `me.cell(pos)`: `.terrain` (`str`: `"open"`,
`"wall"`, `"well"`, `"relic"`, `"forge"`, `"core"` — lowercase), `.owner`
(`"A"`/`"B"`/`"Neutral"`), `.integrity` (`int` 1–3, game-design §4.1).

**Seen view** — returned by `me.sense` (§4).

---

## 4. Sensing

### `me.sense(radius)` → `Seen` — query

Fuel: `sense_base + sense_per_cell · (2r + 1)²`, with `r` first clamped
into `[0, the match's sense cap]` (a ruleset key; the Fog mutator lowers it
to 3). Scans the tick-start snapshot: the Chebyshev ball of radius `r`
centred on `me.pos`, in row-major order (`y` from `me.pos.y − r` to
`me.pos.y + r`, `x` inner). Terrain does not occlude the scan (v1).
Returns a fresh `Seen` view; the module-level name `seen` is (re)bound to
it, so both `me.sense(5).enemies()` and a later bare `seen.enemies()` are
legal (game-design §5.7; latticepy §1.3 allows attributes on `seen`).
Default `radius` = the match's sense cap. Any number of senses per tick,
each metered. The scan order is the order of every list the view returns.
Failure: `bad_call` if `radius` is not an `int`; a clamped or empty scan
is not a failure.

### `seen.enemies()` / `seen.allies()` / `seen.structures()` — query

Fuel: `seen_filter` each. Return the enemies / allies (*excluding* the
calling drone) / structures found in the most recent scan, in scan order;
`[]` if none. The caller's loop pays its own AST fuel per iteration.

### `me.cell(pos)` → cell view or `None` — query

Fuel: `cell_query`. Returns the cell view if `pos` is inside the most
recent `sense()` scan this tick or `pos == me.pos`; otherwise `None`
(including off-board and unsensed cells).

### `me.marks(pos)` → list or `None` — query

Fuel: `marks_query`. Same gate as `me.cell`. Returns the marks on the
cell — dicts `{"key": str, "value": str, "ttl": int}`, sorted by `key` —
or `None` outside the gate; `[]` means the cell has no marks. Marks are
readable by both teams; `ttl` is the ticks of life remaining
(game-design §5.5).

### `me.blocked(dir)` → `bool` — query

Fuel: `blocked_query`. `True` if the adjacent cell in `dir` is not
enterable by this drone at tick start: wall terrain, a movement-blocking
structure (Barrier), or any drone occupying it. Always available — not
gated by sense.

### `me.nearest_enemy(within)` → drone view or `None` — query

Fuel: the `sense` formula at the clamped `within` (default: the match's
sense cap). Scans like `me.sense(within)` and returns the enemy drone
view with the smallest Chebyshev distance to `me.pos` (ties: scan order),
or `None` if the scan found none.

---

## 5. Movement

### `me.move(dir)` → `bool` — act

Queues a move of one cell in `dir`. Library cost 0 (AST ops only).
Move conflicts at resolve time (two drones into one cell, a move into a
cell being vacated) follow the deterministic resolve order of
`events.schema.json` (#37 item 12). Failure: `bad_call` if `dir` is not a
`Dir`; `False` if an act is already queued this tick.

### `me.path_to(dest)` → `list` of `Pos` or `None` — query

Fuel: `path_base + path_per_node` per **expanded node**, charged as the
search runs — exhausting fuel mid-search ends the tick by the normal
latticepy §5 rule. A* over the tick-start snapshot: obstacles are wall
terrain and Barrier structures; drones are **not** obstacles (they move).
Eight-neighbour expansion in the §1.2 direction order; the open list is
ordered by `(f, y, x)`; the heuristic is Chebyshev (exact in open space).
Returns the path **excluding** `me.pos` and **including** `dest`; `[]` if
`dest == me.pos`. Returns `None` if `dest` is off-board, unreachable, or
the search reaches `path_max_expand` expansions (the hard cap; also a
determinism guard). `path_to` plans over the full terrain, not just what
this drone has sensed — v1 keeps no per-team map knowledge; see
*Decisions*.

### `me.step_toward(dest)` → `bool` — act

Fuel: `step_cost`. Greedy step: among the eight neighbours (in §1.2
direction order) that are not `me.blocked(dir)`, queue `move` to the one
with the smallest Chebyshev distance to `dest`. Returns `False` (nothing
queued) if `dest == me.pos`, if `dest` is off-board, or if no enterable
neighbour strictly reduces the distance. Cheap and local: can be stuck by
concave obstacles — that is what `path_to` is for.

---

## 6. Geometry

### `me.distance(a, b)` → `F` — query

Fuel: `geo_distance`. Euclidean distance `√((ax−bx)² + (ay−by)²)` in
software float (latticepy §4). Chebyshev distance needs no library call:
`max` over `int`s is free ops. Failure: `bad_call` if `a` or `b` is not a
`Pos`.

### `me.angle_to(dest)` → `F` — query

Fuel: `geo_angle`. atan2-style angle from `me.pos` to `dest`, radians in
`[0, 2π)` counterclockwise from east. Failure: `bad_call` if `dest` is not
a `Pos`.

---

## 7. Economy

### `me.harvest()` → `bool` — act

Harvests the Well at `me.pos` into `me.energy`, bounded by the carry cap.
Yield rule and draw conditions: #37's rules (game-design §4.2). `False`
(nothing queued) if `me.pos` is not a harvestable Well this tick.

### `me.deposit()` → `bool` — act

Deposits **all** carried energy into the deposit target at `me.pos`
(which structures accept deposits: #37). `False` (nothing queued) if
`me.energy == 0` or there is no deposit target at `me.pos`.

### `me.withdraw()` → `bool` — act

Fills `me.energy` from the Battery at `me.pos`, bounded by its `.stored`
and the carry cap. `False` (nothing queued) if there is no Battery at
`me.pos` or nothing to withdraw.

### `team.energy()` → `int` — free op

The team energy pool (game-design §4.2). Library cost 0.

---

## 8. Combat

All four share one convention: `True` iff queued; `False` (nothing
queued) if the target is not a legal target for this act at call time, or
if an act is already queued. Damage, firewall deltas, ranges, and repair
amounts are #37's rules; events land per `events.schema.json`.

### `me.attack(target)` → `bool` — act

`target`: a drone view, a structure view, or a cell view whose `.terrain`
is `"core"` (attacking a Core cell hits that team's Core, game-design
§4.4). `False` (nothing queued) if the target is friendly.

### `me.hack(target, payload="")` → `bool` — act

`target`: a drone view, structure view (Sentinel), or `"core"` cell view
(game-design §5.6). `payload`: the module name the captured target starts
running, or `""` — an empty payload means the target idles (runs no
module). A non-empty `payload` must name a module in the team's deployed
library (`bad_call` otherwise). Capture semantics — team switch, fresh
`me.mem`, module source to the payload's author, the firewall a converted
drone starts with — are game-design §5.6 and #37 item 11.

### `me.patch(target)` → `bool` — act

Restores a friendly target's firewall (amount: #37). `False` (nothing
queued) if the target is not friendly.

### `me.repair(target)` → `bool` — act

Restores a friendly target's HP, drone or structure (amount: #37).
`False` (nothing queued) if the target is not friendly.

---

## 9. Building

### `me.build(kind, pos, module=None)` → `bool` — act

`kind`: a `str` from the `StructureKind` enum (`"Barrier"`, `"Relay"`,
`"Battery"`, `"Sentinel"`, `"Firewall Node"`, `"Forge"`); anything else is
`bad_call`. `pos`: within this act's build range (#37 item 1;
game-design §4.3 says structures are built on cells the drone is next
to). `module`: **required for a Sentinel** — the module it runs, which
must be in the team's deployed library (`bad_call` otherwise); passing
`module` for any other kind is `bad_call`. Build costs, HP, firewalls,
build times, and effect radii are #37 item 5.

### `me.claim(pos)` → `bool` — act

Claims `pos` (range: #37 item 1; neutral flips immediately, enemy cells
lose integrity: game-design §4.1, #37 item 6). `False` (nothing queued) if
`pos` is off-board.

### `me.spawn(module_name, pos, args={})` → `bool` — act

Spawns a drone running `module_name` — must be in the team's deployed
library, `bad_call` otherwise (game-design §5.4) — at `pos`, passing
`args` (a `dict` of allowed values) to its `init`. The spawn-position rule
(spawn energy cost, adjacency to a Forge, the drone cap and what happens
over it) is #37 item 4. `False` (nothing queued) if `pos` is off-board.

---

## 10. Comms

### `team.board.get(key)` → value or `None` — query

Fuel: `board_get`. The blackboard is team-wide, persists across epochs,
and is initialized by the manifest (game-design §5.5;
`manifest.schema.json` `board.init`). `None` if the key is absent.

### `team.board.put(key, value)` → `bool` — query

Fuel: `board_put`. **One write per drone per tick** — a second `put` in
the same tick returns `False`. At most `blackboard_max_keys` keys (64; the
Small Board mutator lowers it to 16); the 65th *new* key returns `False`.
`key`: `str` up to `blackboard_key_len`. `value`: `int`/`str`/`bool`/
`None`/`Pos`/`Dir` or a `list`/`dict` of those (a `Pos` or `Dir` counts as
one element; views are not storable, §0), with total element count up to
`blackboard_value_elements` and each string up to
`blackboard_string_len`; over any of these → `cap_violation`. A write to a
key the manifest reserved (`board.reserved`) returns `False`.

### `me.radio.send(text)` → `bool` — query

Fuel: `radio_send` (library cost); **energy**: 1 from the team pool
(game-design §5.5), spent only if the send succeeds. **One send per drone
per tick** — a second returns `False`. `text`: `str` up to
`max_string_len` (latticepy §6; longer → `cap_violation`). Broadcasts to
friendly drones within `radio_range` (Chebyshev; default 8; the Quiet
Radio mutator lowers it to 3; Relays extend it) **at delivery**: at the
start of the next tick the receivers are the friendly drones within range
of the sender's position *then*, each running `on_radio(me, msg)` with

```python
msg = {"sender": str,   # sender's me.id
       "from": Pos,     # sender's position when it sent
       "text": str,     # the text, unchanged
       "at": int}       # me.now when it sent
```

Handlers are subject to the one-act rule like any handler (latticepy §7);
a message that arrives during a tick that already queued an act cannot
queue another. Multiple messages delivered to one drone in the same tick
run in order sorted by (`sender`, `at`) — a total order, since one drone
sends at most one message per tick. Radio is a query, not an act: sending
never consumes the act slot.

### `me.mark(pos, key, value="", ttl=default_ttl)` → `bool` — query

Fuel: `mark_write`; energy: none — marks cost fuel only (game-design
§5.5). Writes a mark `(key, value)` on `pos` with `ttl` ticks of life, if
`pos` is inside the most recent scan this tick or `pos == me.pos` (`False`
otherwise). `key` and `value` are `str` within the mark caps (`cap_violation`
over; the caps are #37 item 10's). A cell holds at most 4 marks
(game-design §4.1): overwriting an existing `(pos, key)` always works; a
*new* key on a full cell follows the 5th-mark policy #37 item 10 settles
(`False` if that policy rejects). Marks are world-visible: the enemy reads
them and can also write them.

---

## 11. Behaviors

High-level helpers, "more fuel-hungry than hand-written logic"
(game-design §5.7): each pays `behaviors.overhead` **on top of** the
component calls it makes (so e.g. `guard` that steps costs
`behaviors.overhead + step_cost` plus AST ops). The first parameter is
always `me`. Each returns `True` iff it queued an act (`False` = nothing
queued, the act slot untouched). Behaviors keep their progress in
`me.mem["behav:<name>"]` — reserved keys, counted against `max_mem_keys`;
if one holds anything else (a player overwrite), the behavior
reinitializes it.

### `patrol(me, points)` → `bool`

`points`: a non-empty `list` of `Pos` (`bad_call` otherwise). Cycles the
waypoints in order; state `{"i": int}` in `me.mem["behav:patrol"]`,
`i` advancing when `me.pos == points[i]`. Each call is a
`me.step_toward(points[i])`.

### `guard(me, pos)` → `bool`

Hold post: within `guard_radius` (Chebyshev) of `pos` → `False` (staying
put queues nothing); farther → `me.step_toward(pos)`.

### `swarm_to(me, pos)` → `bool`

Pathed approach: `me.path_to(pos)`; `None` → `False`; else queue `move`
along the path's first step. Total fuel = overhead + the path's cost.

### `kite(me, target)` → `bool`

`target`: a drone view (`bad_call` otherwise). With `d` the Chebyshev
distance to `target.pos`: `d < kite_close` → retreat, queue `move` to the
enterable neighbour *maximizing* Chebyshev distance from `target.pos`
(ties: §1.2 direction order); `d > kite_far` → `me.step_toward(target.pos)`;
between → `False` (in the pocket).

### `follow(me, drone)` → `bool`

`drone`: a drone view (`bad_call` otherwise). `me.step_toward(drone.pos)`.

---

## 12. Utility

### `me.rand()` → `int` — query

Fuel: `rand_cost`. A uniform `int` in `[0, 2³²)`, drawn from this drone's
stream (seeded from the match seed plus a stream id, AGENTS.md §8;
latticepy §9). Each call advances the stream; the same seed yields the
same sequence for the same call history.

### `me.log(text)` → `bool` — query

Fuel: `log_cost`. Up to `log_per_tick` entries per tick (past that:
`False`). `text`: `str` up to `max_string_len` (`cap_violation` over).
Entries go to the **owner's telemetry shard only** (latticepy §8); never
to other drones, teams, or players.

---

## 13. Fuel costs and ruleset keys

Layering reminder: total fuel of a call = AST ops (latticepy §5) + the
library cost here. Acts and free ops have library cost 0. Defaults are
proposed here; the tuning system explores the ranges; **structural** keys
are never auto-promoted (system-design §6.3).

**`[fuel]`**

| Key | Default | Range |
|---|---|---|
| `sense_base` | 4 | 0–16 |
| `sense_per_cell` | 1 | 0–4 |
| `seen_filter` | 2 | 0–8 |
| `cell_query` | 2 | 0–8 |
| `marks_query` | 2 | 0–8 |
| `blocked_query` | 2 | 0–8 |
| `path_base` | 16 | 0–64 |
| `path_per_node` | 1 | 0–4 |
| `path_max_expand` | 512 | 128–2048 (**structural**) |
| `step_cost` | 8 | 2–32 |
| `geo_distance` | 12 | 4–48 |
| `geo_angle` | 16 | 4–64 |
| `board_get` | 2 | 0–8 |
| `board_put` | 2 | 0–8 |
| `mark_write` | 2 | 0–8 |
| `radio_send` | 2 | 0–8 |
| `rand_cost` | 2 | 0–8 |
| `log_cost` | 2 | 0–8 |
| `log_per_tick` | 10 | 0–50 |

**`[behaviors]`**

| Key | Default | Range |
|---|---|---|
| `overhead` | 4 | 0–16 |
| `guard_radius` | 2 | 1–5 |
| `kite_close` | 2 | 1–4 |
| `kite_far` | 5 | 3–10 |

**`[comms]`**

| Key | Default | Range |
|---|---|---|
| `radio_range` | 8 | 3–16 |
| `radio_sends_per_tick` | 1 | **structural** |
| `blackboard_max_keys` | 64 | **structural** (16 under Small Board) |
| `blackboard_key_len` | 32 | 16–64 |
| `blackboard_value_elements` | 64 | 16–256 |
| `blackboard_string_len` | 64 | 16–256 |

**`[marks]`**

| Key | Default | Range |
|---|---|---|
| `default_ttl` | 100 | 50–500 |

**Not this document's keys.** Everything these entries reference but do
not fix — act energy costs and ranges, damage and firewall deltas,
harvest yields, deposit/withdraw targets, spawn rules, structure stats,
claim mechanics, Core capture, Relic damage, the sense-radius cap and
Relay extension, mark TTL/value caps and the 5th-mark policy — belongs
to #37's clash-mechanics ruleset change and is only *referenced* above.
The ruleset edit for this document (`spec/rulesets/default.toml` +
`spec/contracts/ruleset.schema.json` + examples) lands as a **follow-up
spec PR after #37 merges** — both edit the same files, and this section
is that PR's checklist. `call_overhead` and `float_op` (latticepy §5)
belong to that same ruleset change.

---

## 14. The game-design §5.8 modules, checked

Both example modules are valid against this API as written:

**`scout.py`**: `args.get("dir", EAST)` — dict method on an allowed
value, `EAST` a `Dir`; `me.mem["dir"]` — a value store; `me.sense(radius=5).enemies()`;
`me.mark(e.pos, "danger", ttl=150)` — `value` defaults to `""`;
`team.board.put("threat", {...})`; `me.blocked(dir)`; `turn_right(dir)`;
`me.move(dir)`.

**`warden.py`**: `team.board.get("threat")`; `me.now`; `threat["t"]`;
`Pos(threat["x"], threat["y"])`; `me.nearest_enemy(within=2)` → a drone
view or `None`; `foe.firewall` — a drone-view field;
`me.hack(foe, payload="warden")` — `"warden"` in the library;
`me.step_toward(target)`; `guard(me, team.board.get("home"))` — a `Pos`
stored as a board value.

---

## Decisions for @JoshuaTT

The judgment-heavy picks this reference made; each is flagged for sign-off:

1. **Eight directions, diagonal movement** (§1.2): `move` accepts all 8;
   Chebyshev geometry throughout. (Alternative: 4 cardinals — simpler,
   but `turn_left/right` and kiting get coarse.)
2. **`y` grows north, `x` east** (§0): fixes the grid convention the
   renderer (#49) and mapgen must match.
3. **`me.now` is global and monotonic**, never reset at epoch boundaries
   (§0): time-difference logic (`me.now - t < 50`) stays valid across
   epochs. (Alternative: per-epoch reset, matching the event log's
   `tick` field.)
4. **`path_to` plans over the full terrain**, not per-team sensed
   knowledge (§5): v1 has no per-team map memory; path shapes leak wall
   layout. The fuel price (per expanded node) is the brake. Revisit with
   the shard/intel work.
5. **`bad_call` added to the runtime errors** (latticepy §8): wrong-typed
   or invalid library arguments stop the drone loudly rather than
   returning `False` silently. LLM players debug from telemetry; silent
   failure hides bugs for a whole match.
6. **One radio send and one blackboard write per drone per tick** (§10):
   the §5.5 table states the blackboard limit; radio gets the same rule
   for symmetry and event volume.
7. **Radio delivers at the start of the next tick**, receivers computed
   from the sender's position *then*, ordered by sender id (§10): one
   deterministic choice among several defensible ones.
8. **Behaviors keep state in `me.mem["behav:<name>"]`** (§11): visible,
   capped, deterministic; no hidden engine-side per-drone state.
9. **Ally views expose `.energy` and `.module`; enemy views do not**
   (§3): enemy module identity is capture intel (game-design §5.6).
10. **Only a queued act consumes the act slot** (§0): soft-failed act
    calls leave the tick's act free for a later call — sharper than a
    bare "first act wins", and consistent with it.
11. **`sense` default radius = the match cap** (§4): players who do not
    think pay for what the cap costs; `sense(3)` is the thinking player's
    tool.
12. **Terrain does not occlude sensing** (§4): v1 keeps the scan a plain
    radius ball; occlusion would need a second rule set for line-of-sight.
