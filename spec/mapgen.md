# Map generation, fairness and mutators

**Status:** proposed for M2. **Implements:** the map generator and fairness validator (system-design §2.1), the mutator draw (game-design §6). **Complements:** game-design §4.0 (scale table), §4.1, §4.5, §6.

This file settles the rules the M2 generator issues need and that §4.0 and §6 leave open. Every number below is a tunable and lives in the ruleset: `[mapgen]` and `[mutators.params]` in `spec/rulesets/*.toml`, defined by `ruleset.schema.json`. The values shown are the defaults. Until the ruleset loader reads those tables, the defaults here are normative and the tables are optional in the schema.

Everything in this file is deterministic (AGENTS.md §8): integer math only, and the only randomness is the `Mapgen` stream of the match seed (`rng.rs`), drawn in the order given.

## 1. Board sizes and scaling

**Legal side lengths** are `64 · 2^k` for `k = 0..=4`: 64, 128, 256, 512 and 1024. So `s = side / 64` is 1, 2, 4, 8 or 16. (Not every multiple of 64: odd multiples would make `s` a non-power-of-two and gain nothing.)

**Rounding.** Every scaled value is the floor of its exact value, in integer math:

| Formula | Integer form | s = 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|---|
| `base * sqrt(s)` | `isqrt(base² · side) / 8` | base | 1.414·base | 2·base | 2.828·base | 4·base |
| `base * s` | `base · side / 64` | base | 2·base | 4·base | 8·base | 16·base |
| `base + log2(s)` | `base + ilog2(side) − 6` | base | base+1 | base+2 | base+3 | base+4 |

This is what `ruleset.rs` already does. Worked values at 128 and 512 (the sizes §4.0's table omits):

| Parameter | 128 | 512 |
|---|---|---|
| Clash length | 2,828 | 5,656 |
| Drone cap | 84 | 169 |
| Wells | 12 | 48 |
| Neutral Forges | 4 | 16 |
| Relics (formula) | 3 | 5 |
| Starting energy | 565 | 1,131 |

**Relics are placed in pairs.** Both symmetries (§2) pair every object with its image and have no fixed cell on an even-sided board, so an odd Relic count cannot be placed fairly. The generator therefore places `2 · floor(relics / 2)` Relics: 2, 2, 4, 4, 6 at 64, 128, 256, 512, 1024. Wells (6s) and neutral Forges (2s) are always even. The ruleset formula and the §4.0 table are unchanged; only the placed count is rounded down to even.

## 2. Symmetry

Every map has a symmetry `T`, an involution on cells that maps Team A's half onto Team B's half. Walls, Wells, Relics, Forges and Cores are generated on A's half and copied through `T`.

- **Choice.** The first draw from the `Mapgen` stream is `below(2)`: `0` is mirror, `1` is rotational. The choice is part of the Briefing (game-design §6).
- **Mirror:** `T(x, y) = (side − 1 − x, y)`. Team A holds the west half (`x < side/2`).
- **Rotational:** `T(x, y) = (side − 1 − x, side − 1 − y)` (180°). Team A holds the north half (`y < side/2`).

Sides are never swapped inside the generator: a side-swapped series game (game-design §13) uses the same map with the teams' seats exchanged, so the Team A half is fixed.

## 3. Terrain constraints

Terrain is generated on A's half and mirrored. The generator may use any method (it is not specified here) provided the result satisfies all of these; a map that fails any of them is rejected and retried (§4).

1. **Wall density.** Wall cells are between `wall_density_min_percent` (8) and `wall_density_max_percent` (18) of all cells.
2. **No enclosed regions.** All non-wall cells form one 4-connected component. Pockets are filled with wall before the density check.
3. **Minimum corridor width.** Opening the non-wall area with a `min_corridor_width`-square (2×2) structuring element must keep every Core, Forge, Well and Relic cell in one connected component. No objective sits behind a 1-cell choke.
4. **Core clearance.** Each Core centre has an open square of Chebyshev radius `core_clearance_radius` (2, a 5×5 square holding the 3×3 Core and a ring).
5. **Core separation.** The two Core centres are at least `core_min_separation_percent` (50) of the side apart in Chebyshev distance (game-design §4.0).
6. **Starting Forges.** Each team has 2 starting Forges, each between `start_forge_min_distance` (3) and `start_forge_max_distance` (6) from its own Core centre (Chebyshev), on open cells.
7. **Spacing.** Any two of {Wells, Relics, neutral Forges, starting Forges} are at least `object_min_spacing` (6) apart, and every Well, Relic and neutral Forge is at least `object_core_min_distance` (8) from both Core centres.

Distances in §3 and §4 are Chebyshev for placement and **4-connected** (orthogonal) steps for paths. If the movement rule in the clash mechanics (#37) allows diagonal steps, path length switches to 8-connected everywhere in this file at once.

## 4. Fairness validator

By construction `T` is an isometry of the grid, so fairness is **exact equality, no tolerance**. A difference means a generator bug. The validator recomputes everything from the finished map, never trusting the construction:

1. **Image check.** `T(T(c)) = c` and `T(cell)` has the same terrain and the images of objects are the same kind, for every cell.
2. **Path lengths.** For every Well, Relic and neutral Forge `o`: `dist(Core A, o) = dist(Core B, T(o))`, where `dist` is the length of the shortest 4-connected path over non-wall cells. Equivalently the sorted list of distances from Core A to all objects equals the one from Core B.
3. **Well access.** For every Well `w`: the number of non-wall cells among its 8 neighbours equals that of `T(w)` (Wells are claimed by owning most of those neighbours, game-design §4.1).
4. **Relic distances.** For each pair of Relics the Core A to Relic distances, sorted, equal the Core B to Relic distances, sorted (this also follows from check 2 and is reported separately for diagnostics).
5. **Reachability.** Every object is reachable from both Cores.

**On failure** the generator retries. Attempt 0 draws from the `Mapgen` stream of the match seed. Attempt `k ≥ 1` draws from the `Mapgen` stream of `seed_k`, where `seed_k` is the first 8 bytes, little-endian, of `SHA-256(seed as 8 bytes LE ‖ "mapgen-retry" ‖ k as 4 bytes LE)`. After `max_attempts` (16) failures generation fails with a `MapError` (engine-api §5); a map that failed validation is never returned. The symmetry choice and mutator draw (§5, §6) are redone for each attempt from that attempt's stream, so a final map and its mutators always come from one stream.

## 5. Intel shard regions

There are 5 regions (game-design §4.0), 5 horizontal bands. Band `i` (0..=4) covers rows `floor(i·side/5) ..= floor((i+1)·side/5) − 1`, full width. Team A's seat `k` (0..=4) gets band `k`. Team B's seat `k` gets the band that contains `T` of seat `k`'s band: band `k` under mirror, band `4 − k` under rotation, so the two seats' views are images of each other and equally fair. Region membership is a function of the cell only, so mapgen emits one band index per row.

## 6. Mutators

### 6.1 Draw

The mutator draw happens after the symmetry choice and before terrain, on the `Mapgen` stream.

1. **Count.** `mutators.draw_count` (default 2; the schema allows 2 or 3). It is a league setting, not drawn per match.
2. **Candidate pool.** The ruleset's `mutators.pool`, in its listed order, minus: *Long Night* if the scaled clash length is already at least `long_night_ticks` (s ≥ 4), and *Banned Call* if `banned_call_candidates` is empty. These are the only incompatible cases; every other pair may be drawn together.
3. **Draw without replacement.** For `i` in `0..count`: `j = i + below(n − i)`, swap pool entries `i` and `j`. The mutators are `pool[..count]` in that order. If the pool has fewer entries than `count`, all are taken.
4. If *Banned Call* was drawn, one more draw, `below(len)`, picks the disabled function from `banned_call_candidates`.

### 6.2 Semantics

All parameters are in `[mutators.params]`.

| Mutator | Effect, precisely |
|---|---|
| **Fog** | The final sensing radius of every sensing call is `min(radius, fog_sense_cap)` (3). The cap is applied **after** the Relay extension, so Relays cannot lift it. |
| **Costly Spawn** | Spawn energy cost is multiplied by `costly_spawn_multiplier` (3). |
| **Quiet Radio** | The **base** radio range is `quiet_radio_range` (3) instead of 8. Relay extension still adds to it, which is why Relays matter more. |
| **Small Board** | The blackboard holds at most `small_board_keys` (16) keys instead of 64. The manifest check in `manifest.schema.json` uses the same value. |
| **Glass Core** | Each Core's starting firewall is `glass_core_firewall` (15) instead of `core_firewall`. |
| **Plague** | A hack's energy cost is `ceil(cost · plague_hack_cost_percent / 100)` (half, rounded up, so a hack is never free). |
| **Thin Air** | Fuel per tick is `floor(fuel_per_tick · thin_air_fuel_percent / 100)` (500), for drones and Sentinels alike. |
| **Banned Call** | The drawn library function is disabled for the whole match. Calling it is a runtime error (`fn_disabled`, in the `latticepy.md` §8 sense: it stops that drone for that tick). It is not silently `None`. The Briefing names the function. `lattice check` still accepts code that calls it, because the mutator is not known at check time. |
| **Drift** | Each **accepted** move act has a `drift_percent` (1) percent chance of going in a random direction. Resolve order is serial, so draws are in the resolver's fixed order, on the `TieBreak` stream (never a drone's `me.rand()` stream, so drift cannot perturb player code). Per move: draw `below(100)`; if it is below `drift_percent`, draw `below(4)` to pick north, east, south or west uniformly (it may equal the intended direction). If the drifted target is blocked, off the board or occupied, the move fails exactly like any blocked move: the drone stays put, with no re-roll. |
| **Long Night** | Clash length is `max(scaled clash length, long_night_ticks)` (4,000). At s ≥ 4 the scaled length is already 4,000 or more, which is why §6.1 removes it from the pool there. |

Mutator effects are applied once, when the match is built, to a copy of the ruleset-derived values; the ruleset itself is never changed, so a match's ruleset version plus its mutator list reproduces it.

## 7. For Josh to look at most closely

1. **Legal sizes are `64·2^k`, and Relics round down to even (§1).** This changes the placed Relic count at 128 (3 → 2) and 512 (5 → 4) but not at the §4.0 table's sizes. The alternative is allowing only `64·4^k`, which needs no rounding rule at all.
2. **Exact-equality fairness with a 16-attempt retry (§4).** The alternative is a tolerance, which only makes sense if some generator ever breaks exact symmetry.
3. **Symmetry transforms and fixed team halves (§2).** Team A is west under mirror and north under rotation.
4. **Fog caps after Relays, Quiet Radio sets the base (§6.2).** Read from the wording of §6 (Fog "capped", Quiet Radio "Relays matter much more"), but it is a game-rule call.
5. **Banned Call is a runtime error, not `None` (§6.2)**, and the candidate list is empty until the library API (#38) names functions, which keeps the mutator out of the pool until then.
6. **Mutator count is a league setting (§6.1)**, not drawn per match.
7. **The numeric defaults in `[mapgen]`** (8 to 18 % walls, spacing 6, Core distance 8, clearance 2) are proposals chosen to give room on a 64² board; the tuning system can adjust them.
8. **Path metric is 4-connected (§3)** pending the movement rule in #37.

## 8. Ruleset files

`spec/rulesets/default.toml` is not changed by this PR: the engine's ruleset loader rejects unknown tables, so adding `[mapgen]` and `[mutators.params]` there has to land together with the loader change (a code issue in `engine/src/ruleset.rs`, to be filed). `ruleset.schema.json` accepts both tables as optional, and a valid example with every key is in `spec/contracts/examples/ruleset/valid/with-mapgen.json`.
