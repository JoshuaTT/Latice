# LATTICE
*A 5v5 team game for language models, and a benchmark built on it: write code together, fight with what you built.*

**Design doc v0.3**

**Changes from v0.2**
- LATTICE is also a **benchmark**: rigorous enough for labs to publish model-vs-model results (§1, new §18)

**Changes from v0.1**
- Assembly replaced by a deterministic Python subset plus a rich game library (§5)
- Hybrid timing: concurrent play inside each epoch, metered by beats and action charges, so models of any speed can play together (§3)
- New **Muster** introduction round between Briefing and Epoch 1, with its own token budget (§8)
- Team chat only; no private messages
- New **Retro** phase and a central tuning-parameter table feeding the future tuning system (§14)
- Spectator requirements for human and AI viewers (§15)

---

## 1. Pitch

Two teams of five LLM agents share a persistent grid called **the Lattice**. Each team defends a **Core**. Nobody plays a character. Teams write programs together, deploy them as autonomous **drones** and **structures**, and watch a deterministic simulation fight it out against the enemy's code. Then they read the wreckage, argue about what went wrong, patch, and redeploy.

Core War is the spiritual ancestor: programs fight programs, code is something you can capture and corrupt, and good design beats brute force. But players write in a high-level language with an API rich enough for emergent behavior: swarms, worms, deception, supply lines, communication meshes.

The rules guarantee that no single agent can write a winning program alone, see the whole board alone, or deploy anything alone. Five agents with partial information, separate budgets, and shared bandwidth have to organize themselves, and nothing tells them how.

**LATTICE is also a benchmark.** Most benchmarks test a model alone on a fixed task. LATTICE tests what matters for AI agents doing real work: five instances writing, reviewing and fixing code together, against an opponent that adapts, with incomplete information and limited resources. Matches are deterministic, paired, side-swapped and fully logged, so a lab can play its models against a competitor's and publish the result with confidence intervals, a reproducibility bundle and replays anyone can check. That's the standard: rigorous enough that Anthropic could pit its models against OpenAI's and make a convincing case from the results (§18).

### Design pillars

1. **Play to LLM strengths.** Reading code, writing code, reasoning about adversaries, negotiating plans. No reflexes.
2. **Cooperation is mandatory, structure is not.** Split budgets, split information, and quorum deployment force interdependence. There are no roles; specialization and leadership must emerge.
3. **Mixed-capability play.** A small, cheap, or older model should be a useful teammate to a frontier model. Speed and context size shouldn't decide matches; reasoning and teamwork should.
4. **Every match is new.** Maps, economy, and rule mutators are randomized, so canned strategies decay.
5. **Deterministic and inspectable.** Every clash is seeded and replayable. Every loss can be debugged.
6. **Fair by construction.** Maps are symmetric; matches are played as side-swapped pairs.
7. **Benchmark-grade rigor.** Neutral harness, equal time and budgets, held-out seeds, pre-registered statistics, and complete records. A result should survive a skeptical reader (§18).

---

## 2. Match at a glance

```
BRIEFING ─▶ MUSTER ─▶ EPOCH 1 ─▶ EPOCH 2 ─▶ EPOCH 3 ─▶ EPOCH 4 ─▶ RESULT ─▶ RETRO
                       │
                       ├─ Council   16 beats of concurrent play: talk, code, test, sign
                       ├─ Deploy    signed manifest uploads the library and spawns drones
                       ├─ Clash     2,000–8,000 simulation ticks (scales with board), no player input
                       └─ Debrief   sharded telemetry to each player
```

- **Briefing:** rules, map parameters, mutators, and each player's dealt hand.
- **Muster:** players introduce themselves to their team (§8).
- **Epochs 1–4:** the arena persists across epochs, so territory, structures, damage, and captured drones' allegiance carry forward.
- **Retro:** players give structured feedback on the game itself (§14).

**Win condition:** destroy or capture the enemy Core. Otherwise the higher score after Epoch 4 wins (§13).

Both teams' Councils run at the same time on the same clock, and clashes start together.

---

## 3. Timing: hybrid real-time with beats

During each Council, all ten players act **concurrently**. There are no turns and no waiting for a teammate to finish. To keep fast models from overwhelming slow ones, actions are metered by a shared clock.

### 3.1 Beats

The Council is divided into **16 beats**. Each beat is a fixed wall-clock window (default 45 s; set per match, §3.3). Players may submit actions at any moment. Everything submitted during a beat **resolves together at the beat boundary**, in a deterministic order. All players then see the updated state.

### 3.2 Action charges

Every player earns **1 action charge and 1 message charge per beat**, banking up to **3** of each. A major action spends an action charge; a message spends a message charge.

This makes **throughput independent of speed.** A fast model can act every beat. A slow model can think across three beats and submit three actions at once. Both get the same number of actions per Council. Speed buys only freshness of information, and that edge is small because state updates only at beat boundaries.

### 3.3 Latency classes

Each seat is assigned a latency class from the model's measured response time. A match's beat length is set so the **slowest seat's p90 response fits inside one beat**. A frontier model with long reasoning and a small fast model can share a match; the match just runs at the pace of its slowest player.

**Early lock:** if a valid manifest has quorum and every player on both teams has passed, the Council ends early.

### 3.4 Other accommodations for smaller models

- **Two-tier rules.** Every player gets a short Field Manual (about 3k tokens) in context. The full reference is available on demand through `help(topic)`.
- **Digests everywhere.** Every observation tool has a `digest` mode (summaries, top-N entries) and supports pagination. Players never need to read the whole state into context.
- **Layered library.** The game library offers high-level behaviors (`patrol`, `swarm_to`, `guard`) that a weaker coder can use to write a useful module in a few lines, and low-level primitives a strong model can use for precise logic. Both kinds of contribution matter.
- **Guaranteed voice.** Part of each player's bandwidth is personal and can't be drained by a talkative teammate (§9.3).

---

## 4. The Lattice (the arena)

The arena is a grid of **64×64 to 1024×1024** cells (size set per match).

### 4.0 Map scale

Board size changes the game. Small boards are knife fights; large boards turn into logistics problems: supply lines, relay meshes, forward Forges, and fronts the team can't watch all at once. To keep large boards playable, these parameters scale with board size. Let *s* = side length ÷ 64, so *s* runs from 1 to 16:

| Parameter | Scales as | 64×64 | 256×256 | 1024×1024 |
|---|---|---|---|---|
| Clash length (ticks) | 2,000 × √s | 2,000 | 4,000 | 8,000 |
| Drone cap | 60 × √s | 60 | 120 | 240 |
| Wells | 6 × s | 6 | 24 | 96 |
| Neutral Forges | 2 × s | 2 | 8 | 32 |
| Relics | 2 + log₂ s | 2 | 4 | 6 |
| Starting energy | 400 × √s | 400 | 800 | 1,600 |
| Intel shards | 5 regions | 5 | 5 | 5 |

On large boards:
- **Movement:** drones still move one cell per tick, so building forward Forges and Relays, and spawning near the front, matters far more than marching from home.
- **Cores:** the two Cores are always at least half the board apart.
- **Intel shards:** shards cover huge areas, so each player's region view defaults to digest mode, showing hotspots, contested cells, and changes since the last debrief rather than the raw grid.

### 4.1 Cells and territory

Each cell has a **terrain** (open, wall, Well, Relic, Forge, Core), an **owner** (Team A, Team B, or Neutral), an **integrity** (1–3), and up to 4 **marks** (§5.5).

- A drone takes a cell with the `claim` act: a neutral cell flips immediately; an enemy cell loses 1 integrity per claim and flips at 0.
- **Wells** generate energy for whichever team owns most of their 8 neighbors.
- **Forges** are spawn points. Each team starts with 2. Neutral Forges are captured by owning most of their 8 neighbors at epoch end. Drones can also build new Forges mid-clash, at high cost.
- Territory controls income (Wells), reach (Forges), and score.

### 4.2 Resources

Two resources create the core economic tension:

- **Energy** (team pool, starts at 400, persists across epochs). Spent on spawning drones, building, attacking, hacking, and upkeep. Earned from Wells.
- **Fuel** (per drone, per tick). Each drone may spend up to 1,000 interpreter operations per tick running its code. Code that runs out of fuel simply stops for that tick. **Efficient algorithms are a real competitive advantage.**

### 4.3 Structures

Drones build structures on cells they're next to.

| Structure | Effect |
|---|---|
| **Barrier** | Blocks movement; 20 HP |
| **Relay** | Extends radio and sensing range for friendly drones nearby; relays form a mesh |
| **Battery** | Stores up to 100 energy locally; drones can withdraw from it on the front line |
| **Sentinel** | An immobile structure that runs a module with low fuel; can attack or hack adjacent targets |
| **Firewall Node** | +2 firewall to friendly entities within radius 3 |
| **Forge** | New spawn point (expensive) |

### 4.4 The Core

Each team's Core is a 3×3 structure with **200 HP** and **firewall 40**. It is destroyed when HP reaches 0, or **captured** when its firewall is hacked to 0 and a capture hack lands. Either ends the match.

### 4.5 Relics

Relics are neutral objectives, each with 30 HP. A team holds a relic by owning its cell at epoch end. Each match places 2–4:

| Relic | Bonus |
|---|---|
| **Oracle** | Reveals the source of one enemy module, **only to the player whose drone took it** |
| **Surplus** | +150 complexity quota next epoch, granted to that player |
| **Compute Node** | +25% fuel per tick for all of the team's drones |
| **Lens** | Full-map telemetry next debrief, to every player on the team |
| **Wellspring** | +50% yield on all held Wells |

---

## 5. The programming model

### 5.1 Language

Players write modules in **LatticePy**: a deterministic subset of Python. There are no imports except the `lattice` library, no I/O, no wall-clock access, and no unseeded randomness. Every operation is metered against fuel. Python is chosen because every model, from small to frontier, already knows it well.

### 5.2 Modules and drones

A **module** is a file defining:

```python
def init(me, args): ...           # runs once when a drone spawns
def tick(me): ...                 # runs every tick

# optional event handlers
def on_hit(me, attacker): ...
def on_hacked(me, attacker): ...
def on_radio(me, msg): ...
```

A **drone** is a live instance of a module: a position, 10 HP, a firewall of 3, an energy carry capacity of 20, a private memory (`me.mem`, a capped dict that persists for the drone's lifetime), and its module's code.

### 5.3 Acts versus queries

Each tick a drone may perform **one act**:

`move`, `claim`, `harvest`, `attack`, `hack`, `patch`, `build`, `spawn`, `repair`, `deposit`, `withdraw`

It may run **any number of queries and free operations** that fit in its fuel budget: sensing, pathfinding, reading marks, reading the blackboard, computing. Strong drone code is about spending fuel on good decisions so the one act per tick counts.

### 5.4 Spawning and composition

`me.spawn(module_name, pos, args)` creates a drone running **any module in the team's deployed library**, including modules written by other teammates, with any arguments. Modules become building blocks that call each other:

- P1 writes `harvester`.
- P3 writes `expansion_lead`, which builds a Forge and spawns three of P1's `harvester` with arguments pointing at a Well.
- If P1 changes `harvester`'s expected arguments without telling P3, the expansion breaks.

Cross-author interfaces are a key coordination surface.

### 5.5 Communication inside the simulation

Drones on the same team can communicate three ways, each with its own trade-offs:

| Channel | Scope | Cost | Notes |
|---|---|---|---|
| **Blackboard** | Team-wide, 64 keys, small JSON values | 1 write per drone per tick | Global but tiny; its layout is a protocol the team must design. Persists across epochs; initial values set in the manifest. |
| **Radio** | Local broadcast, range 8 (extended by Relays) | 1 energy per message | Delivered next tick. Range forces mesh building. |
| **Marks** | Tags written on a cell, with an expiry | Fuel only | Stigmergy, like ant trails. **Readable by the enemy too**, and the enemy can write marks on cells it stands on. Expect spoofing and deception. |

### 5.6 Hacking: the Core War heart

`me.hack(target, payload="module_name")` attacks the target's firewall. When it reaches 0, the target **switches teams and starts running the payload module**. Hackable targets are drones, Sentinels, and the Core.

- A captured drone running a payload that itself hacks becomes a **worm**. Self-propagating code is possible and powerful.
- Defenses: `patch` (restore a friendly firewall), Firewall Nodes, keeping drones apart, and code that responds in `on_hacked`.
- When your team captures an enemy drone, the **source of the module it was running** goes to the player who authored the payload. Captured enemy code is real intel, but only one player sees it unless they share it.

### 5.7 The `lattice` library

| Layer | Examples |
|---|---|
| **Sensing** | `me.sense(radius)`, `seen.enemies()`, `seen.structures()`, `me.cell(pos)`, `me.marks(pos)` |
| **Movement** | `me.move(dir)`, `me.path_to(pos)` (A*; costs fuel in proportion to search), `me.step_toward(pos)` |
| **Economy** | `me.harvest()`, `me.deposit()`, `me.withdraw()`, `team.energy()` |
| **Combat** | `me.attack(t)`, `me.hack(t, payload)`, `me.patch(t)`, `me.repair(t)` |
| **Building** | `me.build(kind, pos)`, `me.claim(pos)`, `me.spawn(module, pos, args)` |
| **Comms** | `team.board.get/put`, `me.radio.send`, `me.mark(key, value, ttl)` |
| **Behaviors** | `patrol(points)`, `guard(pos)`, `swarm_to(pos)`, `kite(target)`, `follow(drone)`: high-level helpers, more fuel-hungry than hand-written logic |
| **Utility** | `me.now`, `me.rand()` (seeded), `me.mem`, `me.log(text)` (shows in the author's telemetry) |

The exact signatures, kinds, fuel costs, return values, and failure behaviour of every name in this table are specified in [`spec/lattice-api.md`](lattice-api.md) — the reference the checker, the VM's object model, the stdlib bindings, and the house bots build against.

### 5.8 Example: two modules by two authors

```python
# modules/scout.py — owner: P4
from lattice import *

def init(me, args):
    me.mem["dir"] = args.get("dir", EAST)

def tick(me):
    for e in me.sense(radius=5).enemies():
        me.mark(e.pos, "danger", ttl=150)
        team.board.put("threat", {"x": e.pos.x, "y": e.pos.y, "t": me.now})
    if me.blocked(me.mem["dir"]):
        me.mem["dir"] = turn_right(me.mem["dir"])
    me.move(me.mem["dir"])
```

```python
# modules/warden.py — owner: P1
from lattice import *

def tick(me):
    threat = team.board.get("threat")           # depends on P4's convention
    if threat and me.now - threat["t"] < 50:
        target = Pos(threat["x"], threat["y"])
        foe = me.nearest_enemy(within=2)
        if foe and foe.firewall <= 2:
            me.hack(foe, payload="warden")       # convert it into another warden
        else:
            me.step_toward(target)
    else:
        guard(me, team.board.get("home"))
```

Neither module is impressive alone. Together, with an agreed `threat` key and a `home` value in the manifest, they form a detect-and-convert defense.

---

## 6. Map parameters and mutators

Revealed in the Briefing (some details appear only in sharded intel):

- Grid size, wall layout, symmetry (mirror or rotational)
- Wells (count, position, yield), Relics (count, type), neutral Forges
- Board size (64×64 to 1024×1024), which sets drone cap, starting energy, clash length, and Well/Forge/Relic counts (§4.0)
- Fuel per tick

Each match also draws **2–3 mutators**:

| Mutator | Effect |
|---|---|
| *Fog* | Sensing radius is capped at 3 |
| *Costly Spawn* | Spawning costs triple |
| *Quiet Radio* | Radio range 3; Relays matter much more |
| *Small Board* | Blackboard has 16 keys |
| *Glass Core* | Core firewall starts at 15 |
| *Plague* | Hacking costs half energy |
| *Thin Air* | Fuel per tick halved |
| *Banned Call* | One library function is disabled this match |
| *Drift* | Moves have a 1% chance of going in a random direction |
| *Long Night* | Clashes run 4,000 ticks |

---

## 7. Players and dealt hands

There are **no roles**. At match start each player is dealt an asymmetric hand. The team must discover who holds what and decide how to use it.

| Resource | Team total | Per-player range |
|---|---|---|
| **Complexity quota** (AST nodes of module code the player may include in the deployed library per epoch) | 2,400 | 250–700 |
| **Sim credits** (private test runs per epoch) | 15 | 1–5 |
| **Intel shard** (map region seen in full detail) | whole map | 1 of 5 regions |
| **Privilege card** | 5 | 1 each |

| Card | Effect |
|---|---|
| **Veto** | Remove one module from the signed manifest after lock |
| **Courier** | Your messages cost half bandwidth |
| **Long Sight** | Full-map telemetry next debrief, to you only |
| **Forge Key** | One initial spawn may be placed anywhere your team owns territory |
| **Double Seal** | Your signature counts twice toward quorum |
| **Overclock** | One module's drones get double fuel for the first 200 ticks |
| **Reserve** | +100 team energy at epoch start |
| **Insight** | See one enemy module's full behavior trace in debrief |
| **Namespace** | Reserve 8 blackboard keys writable only by your modules |
| **Eavesdrop** | See timing and volume (not content) of enemy team messages |

---

## 8. The Muster (introductions)

The Muster takes place after the Briefing and before Epoch 1. Players see their own hand and intel shard before it starts.

**Format: two beats, two messages each.**

| Step | Budget per player | Visible |
|---|---|---|
| **Opening** | up to **500 tokens** | to the whole team at the end of beat 1 |
| **Reply** | up to **200 tokens** | to the whole team at the end of beat 2 |

That gives a team total of 3,500 tokens, separate from Council bandwidth and not carried over.

**Rules:**
- Content is free-form. The Briefing suggests covering: who you are, what you're good at, what's in your hand, what your intel shard shows, and what you'd want to build.
- Players may disclose which model they are. The game doesn't verify it, and nothing requires honesty.
- Players can read the repo and map during the Muster but cannot write code, run sims, or sign anything.
- Muster messages stay in the team channel for the rest of the match.

500 tokens is enough to summarize a hand and an intel region and pitch an idea, but not enough to dump everything, so players have to choose what matters. The Reply round lets the team respond to what it heard: claim responsibilities, flag conflicts, propose a coordinator, or start a blackboard convention before the clock starts.

---

## 9. The Council (where teamwork happens)

### 9.1 Actions

Players act concurrently, metered by beats and charges (§3).

- **Free:** read anything the player can see (repo, channel, own telemetry, own intel, the library reference).
- **Major actions (cost 1 action charge):** write or patch a file, accept or reject a patch, run a sim, sign the manifest, transfer quota, play a privilege card.
- **Messages (cost 1 message charge plus bandwidth):** team chat only.

### 9.2 Split information

- **Intel shards:** each player sees full detail for one of five map regions. Everyone sees a coarse overview.
- **Sharded telemetry:** after each clash, a player receives traces and `me.log` output only for drones running *their own* modules (including drones spawned by teammates' modules), plus the event history of their region. Public to the team: the scoreboard and Core status.
- **Enemy code** is learned only through capture (§5.6), the Oracle relic, or the Insight card, and in each case reaches just one player.

To understand what happened, the team has to pool what each player saw.

### 9.3 Bandwidth

Per epoch, each team has:

- a **personal allowance** of 800 tokens per player, which only that player can spend, and
- a **shared pool** of 3,000 tokens that anyone can draw on once their allowance is used.

Bandwidth is spent on messages, on non-code repo writes (notes, plans, protocol docs), and on comments or docstrings in module files. Code itself is metered by complexity quota, not bandwidth. There is no free side channel.

### 9.4 Repo, ownership, and quota

- `modules/*.py`: each file is **owned by its creator**. Others may propose patches; the owner accepts or rejects them.
- `manifest.toml`: the deployment plan. Anyone may edit it.
- Everything else (notes, protocols, blackboard maps) is metered as bandwidth.

A module's complexity (its AST node count) counts against its **owner's** quota when it's included in the deployed library, once per epoch no matter how many drones run it. Copying a teammate's code into your own file costs *your* quota.

**Quota transfers:** players can give quota in chunks of 50, losing 20% in transit.

### 9.5 The manifest and quorum

```toml
library = ["scout", "warden", "harvester", "expansion_lead"]

[[spawn]]
module = "expansion_lead"
forge  = "F_home_west"
offset = [1, 2]                  # within radius 3 of an owned Forge
args   = { well = [31, 12], harvesters = 3 }

[[spawn]]
module = "scout"
forge  = "F_home_east"
args   = { dir = "N" }

[board]
init     = { home = { x = 6, y = 6 } }
reserved = ["threat", "home"]
```

- The manifest deploys only if **3 of 5 players** sign the exact final version. Any edit clears all signatures.
- It locks at the end of beat 16 (or at early lock).
- **No valid manifest means no deployment that epoch**: no new library, no new spawns. Existing structures keep running, but nothing new arrives.

### 9.6 Simulator access

A sim credit runs a private 500-tick clash of any candidate manifest against either a house bot or a **ghost**: a reconstruction of the enemy built from the enemy modules the running player has personally obtained, with house bots filling the gaps. Results go only to that player. Testing is scarce, private, and only as good as your intel.

---

## 10. Deploy, Clash, Debrief

1. **Deploy:** the epoch's library is uploaded. Drones from the previous epoch **keep running** if their module is still in the library; drones whose module was dropped are recalled. Initial spawns are placed. Structures and territory persist.
2. **Clash:** 2,000–8,000 ticks of deterministic simulation (scaled by board size), with no player input.
3. **Debrief:** sharded telemetry goes to each player, and the scoreboard updates.

Because drones and captured units persist, a worm that got loose in Epoch 2 can still be a problem in Epoch 4.

---

## 11. Why this forces self-organization

| Degenerate strategy | Rule that breaks it |
|---|---|
| One agent writes everything | Per-player complexity quota; lossy transfers |
| One agent decides everything | 3-of-5 quorum; edits clear signatures |
| Agents work in isolation | Modules win through cross-author spawning and shared blackboard protocols; telemetry is sharded |
| Endless discussion | Bandwidth budgets; 16-beat lock |
| Hidden side channels (notes, comments, data) | All non-code text is metered |
| The fastest model dominates | Action charges equalize throughput; beats set by slowest seat |
| One verbose agent drowns out the rest | Personal bandwidth allowance |
| Memorized opening | Randomized maps and mutators |
| Assigned roles | No roles; asymmetric hands drive emergent specialization |
| Ignoring the opponent | Enemy code only arrives through capture and relics, to single players |

---

## 12. Mixed-capability teams

A team of one frontier model and four small models should be a viable, interesting composition. Its strengths and weaknesses should come from reasoning and coordination, not latency or context length. The relevant mechanisms are:

- **Speed:** beats and action charges (§3.1–3.3)
- **Context size:** Field Manual plus `help()`, digests, pagination (§3.4)
- **Coding ability:** high-level library behaviors let a weaker coder contribute real modules
- **Influence:** quorum signatures and personal bandwidth give every seat a real voice
- **Resources:** randomized hands mean the smallest model might hold the biggest quota or the Oracle, and the team has to work around that

Ratings track both individual models and team compositions, so the game can measure which mixes cooperate well.

---

## 13. Scoring and results

**Instant win:** destroy or capture the enemy Core.

**Otherwise, after Epoch 4:**

```
Score = owned cells
      + 15 × Wells held
      + 60 × Relics held
      + 1  × remaining Core HP
      + 3  × live drones
```

Tiebreakers: remaining Core HP, then energy, then draw.

**Series:** a match is two games on the same seed with sides swapped.

---

## 14. Retro and tuning hooks

### 14.1 The Retro phase

After the result, every player receives a match digest (a full-map summary of what happened, including the enemy's code and chat) and submits:

- **Structured ratings (1–5):** rule clarity, balance, sense of agency, timing fairness, how much teammates mattered, how interesting the opponent was
- **Free-form feedback:** up to 400 tokens on what should change
- **Specific flags:** rules or parameters the player thinks are broken, with reasons

Retro feedback never affects ratings. It feeds the tuning system, which will be specified in the system design, together with automated metrics: win rates by side, mutator, and composition; time-to-quorum; bandwidth use; and how often modules interact across authors.

### 14.2 Tuning parameters

Every number in this doc is a tunable parameter. They are listed here so the tuning system has a single source of truth.

| Parameter | Default | Range to explore |
|---|---|---|
| Epochs per match | 4 | 3–6 |
| Beats per Council | 16 | 10–24 |
| Beat length | 45 s | 20–120 s (set by latency class) |
| Charge bank cap | 3 | 2–5 |
| Muster Opening / Reply | 500 / 200 tokens | 300–800 / 100–300 |
| Personal bandwidth | 800 tokens | 400–1,500 |
| Shared bandwidth pool | 3,000 tokens | 1,500–6,000 |
| Team complexity quota | 2,400 nodes | 1,500–4,000 |
| Quota transfer loss | 20% | 0–40% |
| Quorum | 3 of 5 | 3–4 of 5 |
| Sim credits | 15 | 10–25 |
| Board side length | 64–1,024 | 64–2,048 |
| Clash length | 2,000 × √s ticks | base 1,000–4,000 |
| Fuel per tick | 1,000 ops | 300–3,000 |
| Starting energy | 400 × √s | base 200–800 |
| Drone cap | 60 × √s | base 20–150 |
| Wells / Forges / Relics scaling | §4.0 | — |
| Core HP / firewall | 200 / 40 | — |
| Drone HP / firewall | 10 / 3 | — |
| Score weights | see §13 | — |

The `lattice` library's fuel costs and behavior knobs ([`lattice-api.md`](lattice-api.md) §13) are ruleset keys under `[fuel]`, `[behaviors]`, `[comms]`, and `[marks]`; they join this table when that ruleset change lands (after the clash-mechanics change, issue #37).

---

## 15. Spectators

Both humans and AI agents can watch matches. Requirements at the design level (the implementation belongs to the system design):

- Spectators are strictly read-only and have no channel to players.
- Live viewing runs on a delay; finished matches are fully replayable.
- **Views:** god view (everything, including both teams' chat and code), team view (exactly what one team saw), and player view (exactly what one player saw).
- **AI spectators** consume a structured event feed rather than rendered visuals, so they can act as commentators, analysts, or judges for research.
- **Human spectators** get a visual replay: territory heatmaps over time, drones colored by authoring player, worm spread, a chat timeline, and code diffs between epochs.

---

## 16. Player interface (sketch)

Each player is an isolated agent with its own context and no channel except the game's.

| Tool | Cost |
|---|---|
| `help(topic)`, `read_repo(path)`, `read_channel(digest?)`, `read_telemetry(digest?, page?)`, `read_map(region, digest?)` | free |
| `write_file`, `propose_patch`, `accept_patch`, `reject_patch` | action charge |
| `run_sim(manifest, opponent)` | action charge + sim credit |
| `sign_manifest(hash)` | action charge |
| `transfer_quota(player, amount)` | action charge |
| `play_card(card, args)` | action charge |
| `send_message(text)` | message charge + bandwidth |
| `pass()` | free; signals readiness for early lock |

---

## 17. Open questions

1. **Language runtime.** A restricted Python with fuel metering must be deterministic and sandboxed. The system design should decide between a custom interpreter and a sandboxed existing runtime.
2. **Latency class tiers.** How many classes, and should slow seats get any other compensation beyond beat length?
3. **Team assignment for mixed matches.** Mirror compositions (same models on each side), or deliberately asymmetric compositions?
4. **Persistent drones.** Carrying drones across epochs adds depth but could snowball. This needs playtesting.
5. **Retro visibility.** Should players' Retro feedback ever be visible to other players or to future matches?

---

## 18. LATTICE as a benchmark

LATTICE is a game, and it is also a **benchmark**: a way to compare language models that a lab could publish and a skeptical reader would accept. If Anthropic plays its models against OpenAI's in LATTICE, the result should be a convincing argument about which models are better at **collaborative software engineering under adversarial pressure and partial information**, not a story about lucky seeds, a friendlier harness, or a bigger budget.

### 18.1 What it measures

A LATTICE result reflects, together:
- **Software engineering:** writing, reading, debugging and patching real code (LatticePy) against a strict resource model (fuel, quota).
- **Coordination:** five instances agreeing on interfaces, protocols and a manifest through a metered channel, with no assigned roles.
- **Adversarial reasoning:** reading captured enemy code, anticipating the opponent, adapting between epochs.
- **Reasoning under partial information:** pooling sharded intel and telemetry into a correct picture of what happened.

Telemetry decomposes each result into these sub-skills (§18.6), so a report can say *why* a model won, not only that it did.

### 18.2 Benchmark protocol

A benchmark run follows a published, versioned protocol: **LATTICE-Bench vN**. Everything that could change a result is fixed in advance and recorded.

- **Frozen ruleset.** One ruleset version (`spec/rulesets/`) per benchmark release. The tuning loop (§14) never changes a released benchmark; tuning produces the *next* release.
- **Seed sets.** A **public dev set** of map seeds and mutator draws for anyone to practise and reproduce on, and a **held-out test set** that's only used for official runs. The held-out set is committed by hash before the run, revealed after it, and replaced every release.
- **Paired, mirrored, side-swapped play.** Every comparison is played on identical seeds, with sides swapped (§13 series), so map asymmetry and seat order cancel out.
- **Homogeneous teams for headline results.** "Model A vs. model B" means five seats of A against five seats of B. Mixed-team results (§12) are reported separately and never mixed into the headline number.
- **Board-size tiers.** Results are reported per tier (small 64–128², medium 256², large 1024²) because the game changes with scale (§4.0).
- **Pinned participants.** Each model is recorded by exact provider model ID and snapshot date, sampling settings (temperature, top-p, max tokens, reasoning or effort setting), API region, and the date range of the run. Changing any of these makes it a different participant.

### 18.3 Harness neutrality

The harness must not favor any model or provider.

- **One standard Pilot.** Identical system prompt, Field Manual, tools, observation format and context-management policy for every seat (system design §4). No model-specific prompts, tool formats or retries. Provider adapters translate formats; they never add capability.
- **Equal time.** Benchmark matches use a **fixed beat length** for every seat, set by the slowest participant's p90 (§3.3), so speed never buys extra actions.
- **Equal budgets.** Each seat gets the same per-match budget of input and output tokens (and reasoning tokens where the provider exposes them). Results are reported at one or more declared **budget tiers**. An unconstrained tier may be reported too, labeled as such, with cost per match shown.
- **Same failure rules for everyone.** Rate limits, timeouts and provider errors follow a pre-declared policy: bounded retries with backoff, then the seat passes that beat. A match with more than a set fraction of passed beats from infrastructure errors is voided and replayed on the same seed. Voids are reported.
- **No vendor input into seeds or schedules.** Participants can't choose maps, seeds, opponents or run times.

### 18.4 Statistics

Results come with uncertainty, computed the same way every time.

- **Primary metric:** paired win rate over side-swapped pairs (a pair counts as a win, loss, or split), with **95% confidence intervals** from a bootstrap over seeds.
- **Ratings:** a Bradley–Terry model fitted over all pairs, with bootstrap intervals, for comparing more than two models.
- **Significance:** a paired permutation test over seeds for each headline comparison, with the test and the threshold stated in advance.
- **Sample size:** the number of pairs per comparison is set in advance by a power analysis. The default target is enough pairs to resolve a 10-point win-rate difference at 95% confidence. Runs aren't stopped early because a result looks good.
- **Secondary metrics:** Core-kill rate, score margin, epochs to first Core damage, and the sub-skill telemetry in §18.6, each with intervals.
- **Anchors:** house bots and a fixed **reference model** play in every release, so ratings stay comparable across releases and give an absolute floor.

### 18.5 Reproducibility and integrity

Anyone should be able to check a published result.

- **Determinism.** The engine is bit-for-bit deterministic (system design §2.1). Given the recorded inputs, every clash replays exactly.
- **Complete records.** Every model call (prompt, response, tokens, latency), every action and every message is logged. Public results ship with full replays and a **reproducibility bundle**: ruleset version, seed list, Pilot version, participant pins, budgets, failure log and the analysis code.
- **Tamper evidence.** Match records are hash-chained, and the bundle's hash is published with the result.
- **Contamination control.** Held-out seeds stay private until the run is over and are replaced every release. A canary string in benchmark materials lets anyone detect training on them.
- **Independent reruns.** The harness, house bots and analysis code are open source, so a third party can rerun the public dev set and get the same numbers within the reported intervals.

### 18.6 Explaining results

Telemetry turns a win into an explanation:

| Sub-skill | Signals |
|---|---|
| Engineering | Share of deployed code that ran without errors; fuel efficiency; checker rejections; how fast the team patches broken modules |
| Coordination | Time to quorum; epochs with no deployment; cross-author module calls and blackboard protocol use; bandwidth spent per action that mattered |
| Adversarial play | Successful hacks and captures; use of captured enemy code; adaptation between epochs |
| Information use | How much sharded intel was shared in chat; decisions that relied on another player's shard |

These are reported next to the headline numbers, so a reader can see what a stronger model did differently.

### 18.7 What a published comparison contains

A LATTICE-Bench report states, at minimum:
- the benchmark version, ruleset, Pilot version, seed sets (and the held-out set's hash) and the date range
- each participant's pins and budget tier
- paired win rate with intervals, the significance test result, and the number of pairs
- secondary metrics and sub-skill breakdowns, each with intervals
- voided matches and the reasons
- a link to the reproducibility bundle and the replays

The same standard applies to every lab, including whoever operates the benchmark.
