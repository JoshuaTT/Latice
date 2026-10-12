# LATTICE — Clash Mechanics
*Companion to `game-design.md` §4, §5, §13 and `rulesets/default.toml`*

Every numerical value below is a tuning parameter lives in the ruleset (`spec/rulesets/*.toml`) and is read at runtime (AGENTS.md §2.4). These are v1 defaults; the tuning system (§14) explores them once play data exists.

---

## 1. Per-act costs and ranges

Each drone performs **one act per tick** (game-design §5.3). Acts cost energy from the team pool unless noted. Range uses **4-neighbour** adjacency (up, down, left, right; no diagonals). The `harvest` act is self-targeting on the drone's current cell.

Costs come from the ruleset section `[acts]`.

| Act | Energy cost | Range | Target | Notes |
|---|---|---|---|---|
| `move` | 0 | adjacent (4-n) | empty walkable cell | Consumes the act; no energy cost |
| `claim` | 1 | adjacent (4-n) | any claimable cell | Neutral cell flips immediately; enemy cell loses 1 integrity, flips at 0 |
| `harvest` | 0 | current cell | self (Must be on a Well) | Adds yield to team pool; Well must be owned by team (or most 8-n neighbours) |
| `attack` | 2 | adjacent (4-n) | any entity | Deals damage per §2 |
| `hack` | 3 | adjacent (4-n) | hackable entity | Reduces target firewall per §2; capture at 0 |
| `patch` | 1 | adjacent (4-n) | friendly entity | Restores firewall per §2 |
| `build` | per structure | current cell *or* adjacent (4-n) | empty walkable cell | Costs varies by structure kind (§5); instant |
| `spawn` | 20 | adjacent (4-n) to owned Forge | empty walkable cell | Creates drone from deployed library module |
| `repair` | 2 | adjacent (4-n) | friendly entity with HP | Restores HP per §2 |
| `deposit` | 0 | adjacent (4-n) | Battery (own team) | Transfers carried energy; up to 20 per act |
| `withdraw` | 0 | adjacent (4-n) | Battery (own team) | Withdraws stored energy; up to 20 per act |

---

## 2. Amounts

| Parameter | Value | Ruleset key |
|---|---|---|
| Attack damage | 3 HP | `acts.attack_damage` |
| Hack firewall reduction | 1 per hack act | `acts.hack_firewall_reduction` |
| Patch firewall restore | 2 per patch act | `acts.patch_firewall_restore` |
| Repair HP restore | 2 per repair act | `acts.repair_hp_restore` |
| Harvest yield | 5 energy per harvest act | `acts.harvest_yield` |
| Deposit/withdraw per act | up to 20 (drone carry cap) | `drone.carry_cap` |
| Drone carry cap | 20 energy | `drone.carry_cap` |
| Well passive yield | 2 energy per tick per Well | `economy.well_yield_per_tick` |

A drone standing on a Well it owns may use `harvest` to collect the yield in addition to the passive tick income. The team that owns the most of a Well's 8-neighbour cells receives the passive yield each tick (game-design §4.1).

---

## 3. Upkeep

v1 has **no upkeep**. No energy is deducted per drone per tick. The only ongoing costs are acts the drones choose to perform.

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Drone upkeep per tick | 0 | `economy.drone_upkeep_per_tick` |

---

## 4. Spawning

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Spawn energy cost | 20 (from team pool) | `acts.spawn_cost` |
| Spawn position | Adjacent (4-n) to an **owned Forge**, on an empty walkable cell | structural rule |
| Drone cap enforcement | Spawn fails if team is at drone cap | structural rule |
| Spawn over-cap | Act fails; energy is **refunded** | structural rule |
| Spawn fuel cost | Target is the spawner's remaining fuel for the tick | structural rule |

A drone may spawn any module from the team's deployed library, with arguments (game-design §5.4). The spawned drone begins with full HP, firewall 3, 0 carried energy, and empty `me.mem`.

---

## 5. Structures

All structures are built instantly (one act). Cost is in addition to the `build` act's energy cost. Each structure occupies one cell and blocks movement onto that cell by enemy drones.

| Structure | Build cost (energy) | HP | Firewall | Effect radius | Effect |
|---|---|---|---|---|---|
| **Barrier** | 8 | 20 | 0 | 0 (self) | Blocks all movement onto this cell |
| **Relay** | 12 | 10 | 0 | 6 | Extends radio and sense range by +3 for friendly drones within the radius |
| **Battery** | 15 | 10 | 0 | 0 (self) | Stores up to 100 energy; drones deposit/withdraw when adjacent |
| **Sentinel** | 20 | 10 | 3 | 1 | Runs a single module at 30 fuel/tick; can attack/hack adjacent targets |
| **Firewall Node** | 15 | 10 | 0 | 3 | +2 firewall to friendly entities within the radius |
| **Forge** | 50 | 15 | 0 | 0 (self) | New spawn point for the owning team |

Ruleset keys: `structures.barrier`, `structures.relay`, `structures.battery`, `structures.sentinel`, `structures.firewall_node`, `structures.forge`. Each sub-table holds `build_cost`, `hp`, `firewall`, `effect_radius`, and kind-specific fields.

**Stacking rules (v1):**
- Firewall Nodes stack additively: a drone in range of two Firewall Nodes gets +4 firewall.
- Relays do **not** stack: a drone in range of multiple Relays gets the extension once.
- Barriers occupy the cell and block movement; friendly drones can move through cells occupied by friendly structures they own, except Barriers.

---

## 6. Cells and territory

### 6.1 Integrity

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Starting integrity (neutral cell) | 1 | `cell.default_integrity` |
| Max integrity | 3 | `cell.max_integrity` |
| Integrity gain per friendly claim | +1 (costs 1 energy per claim act) | structural rule |
| Integrity loss per enemy claim | −1 | structural rule |
| Flip threshold | integrity reaches 0 (for enemy-owned cells) | structural rule |

A cell freshly captured (flipped) starts at integrity 1. A drone on a friendly cell can use `claim` to raise the cell's integrity by 1 up to the maximum (3), at the normal claim cost (1 energy). This acts as reinforcement.

### 6.2 Terrains

| Terrain | Walkable | Claimable | Notes |
|---|---|---|---|
| Open | ✔ | ✔ | Standard terrain |
| Wall | ✘ | ✘ | Impassable |
| Well | ✔ | ✔ | Generates energy passive for the team owning most 8-neighbours |
| Relic | ✔ | ✔ | Neutral objective; see §8 |
| Forge | ✔ | ✔ | Spawn point; captured by owning most 8-neighbours at epoch end |
| Core | ✔ (friendly only) | ✔ | 3×3 structure; see §7 |

`claimable = [open, well, relic, forge, core]`
`walkable = [open, well, relic, forge, core]`

Terrain walkability and claimability are stored in the ruleset at `terrains.<kind>.walkable` and `terrains.<kind>.claimable`.

---

## 7. The Core

Each team's Core is a 3×3 structure at a fixed position on the map (game-design §4.4).

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Core HP | 200 | `board.core_hp` |
| Core firewall | 40 | `board.core_firewall` |
| Core size | 3×3 cells | `board.core_size` |

### 7.1 Walkability

- **Friendly** drones can move through cells occupied by the friendly Core.
- **Enemy** drones cannot enter Core cells; they must attack or hack from an adjacent (4-n) cell.

### 7.2 Destruction and capture

- **Destruction:** Core HP reaches 0 from attack damage. Match ends immediately.
- **Capture:** Core firewall reaches 0 from hack acts. The **next** hack act by an enemy drone on the Core (targeting it) captures it, flipping it to the attacking team. Match ends immediately.
- A captured Core's HP is unchanged; only the firewall is 0 and the ownership has flipped.

---

## 8. Relics

Relics are neutral objectives placed by the map generator (game-design §4.5).

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Relic HP | 30 | `relics.hp` |
| Hackable | No | structural rule |
| Repairable | No | structural rule |
| Attacked damage | Normal (3 HP per attack act) | follows attack rules |

### 8.1 Holding

A team holds a Relic by **owning the cell the Relic occupies at the end of an epoch**. The cell must be claimed—ownership of the 8-neighbours alone is not enough. The team receives the Relic's bonus for the next epoch.

### 8.2 Destruction

When a Relic's HP reaches 0, it is **destroyed** and removed from the game for the remainder of the match. The cell returns to open terrain. Destroyed Relics do not count toward the Relics-held score.

---

## 9. Sensing

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Default sense radius | 5 | `sensing.default_radius` |
| Maximum sense radius | 10 | `sensing.max_radius` |
| Relay sense extension | +3 | `sensing.relay_extension` |
| Relay effect radius | 6 (drones within this range get the extension) | `structures.relay.effect_radius` |

`me.sense(radius)` costs fuel proportional to the number of cells scanned. HUD and digest modes (game-design §3.4) reduce the cost by returning summarized data instead of raw cells.

A drone within range 6 of a Relay can sense up to radius 10 (default 5 + relay extension 3 = 8, capped at max 10). Multiple Relays do not stack.

---

## 10. Marks

Marks are tags written on cells for stigmergic communication (game-design §5.5).

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Max TTL | 500 ticks | `marks.max_ttl` |
| Value size cap | 64 bytes | `marks.value_size_bytes` |
| Max marks per cell | 4 | `marks.max_per_cell` |
| Eviction policy | oldest mark evicted (FIFO) when 5th mark is written | structural rule |

Marks are **readable by both teams** (and the enemy can overwrite them). This is intentional: marks are stigmergy, and deception through mark spoofing is expected (game-design §5.5).

The `mark` act itself costs fuel only (the write is a free operation from an energy perspective; the cost is in fuel used by the `me.mark()` function call).

---

## 11. Hack conversion

| Parameter | Value | Ruleset key |
|---|---|---|---|
| Converted drone starting firewall | 2 | `hack.converted_firewall` |

When a target's firewall reaches 0 and a subsequent `hack` act applies a payload module, the target:
1. Switches teams to the attacker's team.
2. Begins running the payload module (calling `init(me, args)` with the payload's args).
3. Starts with the converted firewall value (2), not its original firewall.
4. Retains its current HP and carried energy.

Converted drones running a payload that itself hacks are **worms** (game-design §5.6). Self-propagating code is possible.

---

## 12. Resolve order and conflict rules

The engine resolves acts **serially** in a fixed order each tick (game-design / system-design §10.1). The events schema defines the priority:

```
build < spawn < repair < patch < deposit < withdraw < harvest < claim
< move < attack < hack < capture < radio_sent < mark_written
< core_damage < structure_destroyed < cell_flipped
```

Within each act type, priority goes to the seat that comes first in the **rotating per-epoch seat order** (the order advances each epoch so no seat is always first).

### 12.1 Conflict rules

| Situation | Resolution |
|---|---|
| **Two drones move into the same cell** | Seat priority wins: the drone whose seat comes first in the current epoch's order gets the cell; the other's move fails and it stays in place. Both drones consume their act. |
| **Move into a cell being vacated** | Valid. All moves resolve atomically in seat order: the vacating drone leaves first (earlier seat), vacating the cell, which is then available for later-seat moves. |
| **Target moves away before attack** | Moves resolve before attacks. If the attack target has moved to a position still adjacent to the attacker, the attack applies at the new position. If the target is no longer adjacent, the attack fails (energy is still spent). |
| **Attack destroys a target mid-tick** | The destroyed entity's subsequent scheduled acts are skipped. |
| **Structure destroyed mid-tick** | Acts targeting a destroyed structure fail (e.g., a hack targeting a Barrier that was destroyed earlier in the same tick). |
| **Drone destroyed by attack while carrying energy** | Carried energy is lost (not transferred). |
| **Simultaneous claims to same cell** | Seat priority wins: the earlier seat's claim resolves first. For enemy cells, the first claim reduces integrity by 1; the second resolves against the updated integrity. |

These rules are enforced by the resolve engine executing acts in the deterministic priority order listed above. No act can "look ahead" at pending resolutions; each sees the state left by all previously resolved acts.