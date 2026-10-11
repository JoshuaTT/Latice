# LATTICE — System Design
*v0.1 · companion to `spec/game-design.md` (v0.2)*

---

## 0. Summary

LATTICE runs as a set of services around one deterministic core:

- **Lattice Engine** (Rust): the simulation, the LatticePy virtual machine, the code checker, and the map generator. It compiles to native code for servers and to WebAssembly for browsers.
- **Match Orchestrator** (Python): runs each match through Briefing, Muster, the four epochs, and Retro. It owns the beat clock, action charges, team repos, chat, quorum, and practice sims.
- **Player Gateway**: the only door into a match for players. It speaks MCP and HTTP and enforces what each seat may see and do.
- **Pilot**: a model-agnostic harness that seats any LLM (frontier, small, old, or local) as a player, with identical prompts and tools for every seat.
- **Event Store and Replay**: matches are stored as their seed plus player inputs and re-simulated on demand.
- **Spectators**: a web viewer for humans and an MCP/stream feed for AI agents.
- **League services**: scheduling, ratings, budgets, and the **tuning loop**, which turns Retro feedback and match metrics into proposed rule changes for a human to approve.

The whole system is designed to be **built by coding agents**:

- Contracts and specs come first.
- Every module has machine-checkable acceptance tests.
- Determinism is enforced by golden tests.
- Human checkpoints are limited to taste, policy, money, and credentials.

---

## 1. Architecture

```
                        ┌───────────────────────────────────────────┐
                        │              League Services              │
                        │  Scheduler · Ratings · Budget · Tuning    │
                        └───────────────┬───────────────────────────┘
                                        │ starts matches, reads results
┌──────────────┐   MCP / HTTP   ┌───────▼────────┐      ┌────────────────────┐
│ Pilot × 10   │◀──────────────▶│ Player Gateway │◀────▶│ Match Orchestrator │
│ (one / seat) │                └────────────────┘      │ beats · charges ·  │
│  ┌────────┐  │                                        │ repo · chat ·      │
│  │Provider│──┼──▶ Anthropic / OpenAI / Google /       │ quorum · sims      │
│  │adapters│  │    OpenRouter / local vLLM             └──────┬─────────────┘
└──────────────┘                                               │ calls
                                                      ┌────────▼────────────┐
                                                      │   Lattice Engine    │
                                                      │ sim · LatticePy VM  │
                                                      │ checker · mapgen    │
                                                      └────────┬────────────┘
                                                               │ events, snapshots
┌────────────────────┐   WebSocket   ┌──────────────┐  ┌───────▼──────┐
│ Human Spectator UI │◀─────────────▶│  Spectator   │◀─│ Event Store  │
│ (WASM replay)      │               │  Service     │  │ Postgres +   │
└────────────────────┘   MCP/stream  │ (delay, ACL) │  │ object store │
┌────────────────────┐◀─────────────▶│              │  └──────────────┘
│ AI Spectators      │               └──────────────┘
└────────────────────┘
        ▲
        │ Admin console (humans): config, approvals, costs, kill switch
```

---

## 2. Software modules needed to run the game (Q1)

### 2.1 Core engine (Rust crate: `lattice-engine`)

| Module | Responsibility | Key requirements |
|---|---|---|
| **World model** | Chunked grid (64×64 chunks, sparse for empty areas), cells, terrain, ownership, integrity, marks, structures, drones, Cores, Relics | A 1024² board in under 200 MB; cheap snapshots (copy-on-write chunks) |
| **Tick scheduler** | Runs each tick in two phases: **think** (every drone runs its code against a read-only snapshot, in parallel) and **resolve** (acts applied serially in a deterministic order) | Bit-identical results on every platform and thread count |
| **LatticePy VM** | Parse → validate subset → compile to bytecode → run with fuel metering and memory caps | No host access, insertion-ordered dicts, deterministic floats (basic IEEE ops only; math library implemented in software), seeded RNG |
| **Checker / quota counter** | Validates the Python subset, counts AST nodes for quota, checks the module interface (`init`/`tick`/handlers), gives lint messages players can act on | Fast (under 50 ms per module); error messages written for LLM readers |
| **Game library (`lattice`)** | Sensing, movement, A*, economy, combat, hacking, building, comms, high-level behaviors | Every call has a documented fuel cost; reference docs generated from code |
| **Map generator** | Seeded, symmetric maps; scaling rules (game doc §4.0); mutator draws; fairness validator (path lengths, Well access, and Relic distances equal for both sides) | Same seed always produces the same map |
| **Ghost builder** | Builds a practice opponent from the enemy modules one player has obtained, plus house bots | Uses only that player's knowledge |
| **Telemetry sharder** | Splits clash events into per-player, per-team, and god views; builds digests (hotspots, deltas, contested cells) | Visibility rules tested exhaustively |

The engine ships as a Rust library, a Python binding (PyO3) used by the orchestrator, a WASM build used by the browser viewer, and a CLI (`lattice sim`, `lattice check`, `lattice mapgen`, `lattice replay`).

### 2.2 Match runtime (Python service: `orchestrator`)

| Module | Responsibility |
|---|---|
| **Match state machine** | Briefing → Muster → (Council → Deploy → Clash → Debrief) × 4 → Result → Retro, with crash recovery from persisted state |
| **Beat clock and charges** | Beat timing from the match's latency class; action and message charges; resolving actions at each beat boundary; early lock |
| **Action resolver** | Deterministic order within a beat (by action type, then a rotating seat order); conflict rules (e.g., two patches to one file in the same beat both become pending) |
| **Team repo** | Versioned file store per team; file ownership; patch proposals and accept/reject; history visible to the team |
| **Channel and bandwidth meter** | Team chat; personal and shared bandwidth; metering with one **canonical tokenizer** so every model is charged the same no matter which tokenizer it uses |
| **Hand dealer and quota ledger** | Deals hands; tracks quota, transfers (with loss), sim credits, and privilege cards |
| **Manifest service** | Parses and validates `manifest.toml` (library, spawn positions within Forge radius, blackboard setup); content hash; signatures; quorum; locking |
| **Practice sim runner** | Runs 500-tick private sims on a worker pool; results go only to the requesting seat |
| **Muster and Retro handlers** | Enforce the Muster's token budget and two-step reveal; collect Retro feedback |

### 2.3 Player access

| Module | Responsibility |
|---|---|
| **Player Gateway** | MCP server plus an equivalent HTTP/JSON API. One token per seat. Enforces visibility, charges, and idempotency (every action carries `beat_id` and an idempotency key). Read tools support `digest` and pagination. |
| **Field Manual and help system** | A short manual (about 3k tokens) generated from the ruleset and library docs, plus `help(topic)` for the full reference. Regenerated automatically whenever the ruleset changes. |
| **Private scratchpad** | Each seat gets a fixed-size private notes file (default 4k tokens), so no player's memory depends on its harness (see §10). |

### 2.4 Persistence and replay

| Module | Responsibility |
|---|---|
| **Event store** | Postgres for structured records (matches, seats, actions, messages, manifests, results, feedback); object storage for clash event logs and periodic snapshots (every 500 ticks) |
| **Replay service** | Rebuilds any tick from seed + manifests + snapshots. Because the engine is deterministic, a match's canonical record is small. |
| **Ruleset registry** | Versioned ruleset files (TOML) holding every tunable parameter. Every match records its ruleset version. |

### 2.5 League and operations

| Module | Responsibility |
|---|---|
| **Scheduler** | Queues matches, assigns models to seats (mirrored or asymmetric compositions), creates side-swapped pairs, respects budgets |
| **Latency calibrator** | Measures each model's p90 response time on a standard Council-sized prompt and assigns latency classes |
| **Ratings** | TrueSkill per model and per team composition, plus per-ruleset breakdowns |
| **Budget controller** | Per-match and per-seat token and dollar caps; projects spend before a match starts and stops matches that exceed limits |
| **Tuning system** | See §6 |
| **Observability** | Traces for Pilot calls (latency, tokens, cost, errors), engine performance, match health dashboards, alerts |
| **Admin console** | Web UI for configuring and starting matches, viewing costs, approving rule changes, and stopping matches |

---

## 3. Modules for humans vs. AI (Q2)

| Module | Human players | AI players | Human spectators | AI spectators | Operators (you) |
|---|---|---|---|---|---|
| Player Gateway (MCP/HTTP) | later (§3.3) | ✔ | | | |
| Pilot harness | | ✔ | | | |
| Field Manual + `help()` | ✔ | ✔ | | ✔ (rules context) | |
| Spectator web app | | | ✔ | | ✔ |
| Spectator feed (MCP + stream) | | | | ✔ | |
| Commentary overlay | | | ✔ (consumes) | ✔ (produces) | |
| Retro form | | ✔ | | | ✔ (reads) |
| Tuning review UI | | | | | ✔ |
| Admin console | | | | | ✔ |

### 3.1 For AI

- **Pilot** (§4) seats a model as a player.
- **Spectator feed**: a read-only MCP server and event stream with tools such as:
  - `get_state(view, tick)`, `query_events(filter, tick_range)`, `region_digest(region, epoch)`
  - `get_code(team, module, epoch)`, `get_chat(team, range)`, `get_scoreboard()`, `subscribe(stream)`
  - Views are god, team, or player, enforced server-side. Live access runs on a delay.

AI spectators can play several parts:
- **Commentator**: writes live text commentary, shown in the human viewer.
- **Analyst**: writes post-match reports explaining why a team won and which modules mattered.
- **Judge**: scores coordination quality for research, e.g., how much code depended on other authors' modules, or how well intel was shared.
- **Tuning Analyst**: proposes rule changes (§6).

### 3.2 For humans

- **Spectator web app** (TypeScript, WebGL):
  - Replays run in the browser using the WASM engine, so the server sends only seed, inputs, and snapshots.
  - Large boards render as a tiled texture with level of detail; only changed chunks are redrawn.
  - Panels: territory heatmap, drones colored by authoring player, worm spread overlay, a scrubbable timeline across epochs and ticks, both teams' chat timelines, code diffs between epochs, and a manifest viewer.
  - View switcher: god, team, or player (see exactly what one model saw).
  - Optional AI commentary track.
- **Admin console**, **tuning review UI**, and a **Retro dashboard**.

### 3.3 Human players (later, not v1)

The Gateway is plain MCP/HTTP, so a human seat would only need a web client: an editor, chat, map, and charge indicators. It's worth keeping in mind for mixed human/AI matches, but it's out of scope for v1.

---

## 4. The Pilot (seating any model)

The Pilot is the most important piece for **mixed-capability play**. Every seat runs the same Pilot code with the same system prompt template, so differences between seats come from the model, not the harness.

### 4.1 Loop

```
on beat boundary (or when the previous model call finishes):
    obs = gateway.observe(since=last_seen)       # delta + notifications + charges
    prompt = system_template + field_manual + scratchpad + rolling_summary + obs
    model call with tools (free reads + actions), max N tool steps per beat
    actions submitted with the current beat_id
    if the call runs past the beat: actions land in the beat they arrive in
```

- **Slow models aren't penalized for being slow.** The Pilot never cancels a call because a beat ended; action charges simply accumulate (game doc §3.2).
- **Free reads are capped** at a fixed number of tool steps per beat, to control cost and keep seats equal.

### 4.2 Provider adapters

- Anthropic, OpenAI, and Google APIs; OpenRouter for breadth; an OpenAI-compatible endpoint for local models (vLLM, Ollama).
- Native tool calling where the model supports it. Otherwise a **text protocol fallback** (the model writes fenced JSON action blocks, which the Pilot parses and validates, re-prompting once on error). This keeps older models playable.
- Prompt caching where the provider supports it (the system template and Field Manual are stable).

### 4.3 Context management

Each seat declares a context tier: small (under 16k tokens), medium, or large. The Pilot:
- trims observations to digest mode for small tiers,
- keeps a rolling summary of chat and events,
- relies on the gateway scratchpad for durable notes.

Context handling is part of the Pilot, so it's identical for every model in the same tier.

### 4.4 Harness policy

- **v1: one standard Pilot only.** Custom harnesses would mix harness quality into model comparisons.
- **Later: an "open harness" league** where teams bring their own Pilot, with ratings tracked separately.

---

## 5. Spectating, replay, and data flow

1. Every seat action, message, repo write, and manifest is written to Postgres the moment it resolves.
2. Each clash writes an event log (moves, acts, hacks, captures, Core damage) and snapshots to object storage.
3. The **Spectator Service** applies a delay (default: one full epoch behind live; configurable) and access rules, then streams to the web app over WebSocket and to AI spectators over MCP.
4. Finished matches are fully public in god view (configurable per league).

The delay exists so live viewers can't learn anything that could leak back into an ongoing match. Players have no outside channels, but the delay protects any future human-seat matches.

---

## 6. Feedback and tuning system

This is the mechanism for learning from games and players how the game should be tuned.

### 6.1 Inputs

| Source | What it provides |
|---|---|
| **Retro feedback** | 1–5 ratings (clarity, balance, agency, timing fairness, teammate impact, opponent interest), free-form text, flags on specific rules |
| **Automated match metrics** | Win rate by side, map size, mutator, and composition; Core-kill vs. points finishes; time to quorum; epochs with no deployment; bandwidth use (personal vs. shared); quota transfers; share of modules spawned by other authors; blackboard key use; worm prevalence; fuel exhaustion rate; practice sim usage; snowball index (how well the epoch-1 leader predicts the winner) |
| **Model-capability fairness metrics** | Contribution by latency class and context tier: code that actually ran, actions per beat, messages sent, signatures |
| **Self-play balance runs** | Cheap house-bot and small-model matches run on candidate rulesets |
| **AI judge reports** | Coordination and strategy analyses (§3.1) |

### 6.2 Pipeline

```
matches ─▶ metrics ETL ─▶ Tuning Analyst agent ─▶ proposal ─▶ balance runs ─▶ human review ─▶ canary ─▶ promote
   ▲                         (reads Retro +            (diff to          (old vs new        (you)       (10% of
   └──────── feedback ───────  metrics + reports)       ruleset)          on 200+ cheap)              matches)
```

1. **Tuning Analyst agent** (scheduled weekly or every N matches) groups Retro feedback into themes, connects them to metrics, and writes a **proposal**: a diff to the ruleset with a rationale, predicted effects, and the metrics that should move.
2. **Balance harness** runs the old and new rulesets in many cheap matches and reports the metric changes with confidence intervals.
3. **Human review**: you approve, edit, or reject in the tuning review UI.
4. **Canary**: the new ruleset runs in a fraction of scheduled matches. If metrics regress, it rolls back automatically.
5. **Promote**: the ruleset version bumps, and the Field Manual and docs regenerate automatically.

### 6.3 Guardrails

- At most 3 parameter changes per release, so the effect of each can be attributed.
- Parameters marked "structural" (quorum size, team size, win conditions) always need human sign-off and never auto-promote.
- Ratings are kept per ruleset version and merged only between compatible versions.
- Retro feedback is never visible to other players or used in future prompts. It only feeds the tuning system.

---

## 7. What agents can build without human input (Q3)

These parts have clear specs and objective tests, so agents can build and verify them end to end:

| Module | Why agents can do it alone |
|---|---|
| Lattice Engine: world model, tick scheduler, game library | Fully specified by the game doc; correctness checked by unit, property, and golden tests |
| LatticePy VM and checker | Clear subset spec; fuzzing and differential tests against CPython for allowed programs |
| Map generator and fairness validator | Measurable fairness properties |
| Ghost builder, telemetry sharder | Visibility rules can be tested exhaustively |
| Orchestrator: state machine, beats, charges, resolver, repo, manifest, quorum, metering | Deterministic rules; scripted-player integration tests |
| Player Gateway (MCP + HTTP) | Contract-first; schema tests |
| Event store, replay service, ruleset registry | Round-trip tests (record → replay → identical state) |
| House bots (LatticePy) | Measurable by win rate against each other |
| Field Manual and help generation | Generated from ruleset and library docstrings; checked by a small-model quiz (can a small model answer 20 rules questions correctly?) |
| CLI tools, CI pipelines, determinism tests | Purely technical |
| Spectator feed (AI) | Contract-first; access-rule tests |
| Metrics ETL, balance harness, ratings | Statistical code with known-answer tests |
| Pilot core loop, scripted players, text-protocol fallback | Testable against mock providers |

## 8. What agents can build with some human input (Q4)

| Module | Human input needed |
|---|---|
| **Pilot provider adapters** | API keys; which models to include; budget limits |
| **Latency calibration and classes** | Agreeing on tier boundaries after seeing the measurements |
| **LatticePy subset** | Sign-off on the language: which Python features are allowed, float policy, memory limits |
| **System prompt template for players** | Review for neutrality (it must not coach a strategy) |
| **Spectator web app** | Visual direction, layout, what matters most to watch; a few review rounds |
| **Admin console and tuning review UI** | Workflow preferences; who has access |
| **Tuning Analyst prompts and guardrails** | Which parameters can change automatically; which metrics count as regressions |
| **First balance pass** | Playtest judgment: are matches interesting? Agents can measure balance, but "fun" needs you |
| **Infrastructure / deployment** | Cloud account, domain, secrets; agents write the infrastructure-as-code, you apply it the first time |
| **Ratings policy** | How to rate mixed compositions; what goes public |
| **Security review** | Final review of sandbox escape risks and gateway access rules |
| **Commentary agent tone** | Style and personality, if spectating is public |

---

## 9. What you need to set up (Q5)

### 9.1 Accounts and access

1. **Source control:** a GitHub organization and monorepo, with branch protection and required CI checks so agent PRs can't merge red.
2. **Coding agents:** Claude Code (or your preferred coding agent) with a plan that supports long, parallel sessions; GitHub app access for opening PRs.
3. **Model providers** (for players and spectators): Anthropic, OpenAI, and Google API accounts, plus **OpenRouter** for older and open-weights models from one key. Optionally a GPU host (e.g., a rented single GPU running vLLM) for small local models.
4. **Cloud:** one provider for containers, managed Postgres, and object storage (e.g., S3 or Cloudflare R2). A small setup is enough at first: one orchestrator node and one or two worker nodes for sims.
5. **Secrets manager** for API keys, never stored in the repo.
6. **Domain and auth** for the spectator site and admin console (e.g., GitHub OAuth or Cloudflare Access in front of admin).

### 9.2 Money and limits

- Billing alerts and hard spending limits on every provider account.
- A per-match budget. **Rough estimate, to verify against current pricing:** a Council beat is about 10–20k input tokens and 1–3k output tokens per seat. A full match is about 700 model calls, roughly 10M input and 1.5M output tokens. With prompt caching, an all-frontier match likely costs **tens of dollars, possibly over $100**. Small-model matches cost a small fraction of that. Balance runs should use house bots and cheap models.

### 9.3 Decisions only you can make

- Sign off on the LatticePy subset and the player system prompt.
- Which models are in the first league, and whether results are public.
- Visual direction for the spectator app.
- Tuning guardrails, and approval of each ruleset release.
- Review model providers' terms of service for running automated competitions and publishing outputs.

### 9.4 Your time

Short reviews at each milestone (§11), and playtest viewing from M4 on. Plan for about an hour per milestone, plus watching a handful of matches per tuning cycle.

---

## 10. Changes this design makes to the game rules

These should be folded into the game design doc v0.3:

1. **Two-phase ticks:** within a tick, all drones see the state from the start of the tick, and acts resolve together in a deterministic order. This makes simulation parallel and deterministic.
2. **Canonical tokenizer** for all bandwidth metering.
3. **Private scratchpad** of fixed size (4k tokens) per seat, not metered as bandwidth.
4. **Comments and docstrings are stripped** from enemy code revealed by capture, the Oracle, or Insight. String literals stay; in-game deception is allowed, but it can't carry instructions outside game code.
5. **Free-read step cap** per beat, so fast models can't out-read slow ones within a beat.
6. **Standard Pilot only** in the v1 league.

---

## 11. Build plan for coding agents

### 11.1 How to organize the agents

- **Specs are the source of truth.** The game design, this system design, and machine-readable **contracts** (JSON Schema for gateway tools and events; the Rust trait for the engine API; a TOML schema for rulesets) live in `/spec`. Agents may propose changes to specs but cannot merge them without your approval.
- **One module per agent workstream.** Each module directory has an `AGENTS.md` (or `CLAUDE.md`) stating its scope, contracts, acceptance tests, and what not to touch. Agents work in parallel git worktrees.
- **A coordinator agent** keeps the milestone task list, assigns workstreams, and resolves contract questions. **Reviewer agents** check each PR against the spec. **Test agents** write adversarial tests (fuzzers, sandbox escape attempts, determinism breakers).
- **CI is the referee:** lint, unit tests, golden determinism replays (native and WASM must match), contract tests, and performance budgets.
- **Human checkpoints** happen only at milestone boundaries and for the items in §8.

(Fittingly, building LATTICE is itself a test of agents organizing around shared contracts.)

### 11.2 Repo layout

```
lattice/
  spec/           game-design.md · system-design.md · contracts/ · rulesets/
  engine/         Rust: world, tick, vm, checker, stdlib, mapgen, ghost, shard
  engine-py/      PyO3 bindings
  engine-wasm/    browser build
  orchestrator/   Python: match FSM, beats, repo, chat, manifest, sims
  gateway/        MCP + HTTP player API
  pilot/          harness + provider adapters + context manager
  spectator/      feed service (MCP/stream) + web app (TypeScript)
  league/         scheduler, ratings, budget, latency calibrator
  tuning/         metrics ETL, analyst agent, balance harness, review UI
  admin/          admin console
  bots/           house bots (LatticePy)
  infra/          IaC, containers, CI
  tests/golden/   seeded replays with expected hashes
```

### 11.3 Milestones

| # | Milestone | Done when | Human checkpoint |
|---|---|---|---|
| **M0** | Contracts and skeleton | Schemas, repo, CI, empty modules compile | Approve contracts and LatticePy subset |
| **M1** | Headless engine | `lattice sim` runs house bot vs. house bot at 64² deterministically; checker and quota work | Glance at sample replays (ASCII) |
| **M2** | Maps and scaling | Map generator with all mutators; fairness validator; 1024² clash under the performance budget | — |
| **M3** | Match runtime | Full match with **scripted** players through Gateway: beats, charges, repo, quorum, Muster, Retro | — |
| **M4** | First LLM match | Pilot with 2+ providers; a real 5v5 at 64² ends with a result | Watch it; approve system prompt; set budgets |
| **M5** | Watchable | Event store, replay, AI spectator feed, basic web viewer | Visual direction review |
| **M6** | League and tuning | Scheduler, ratings, latency classes, metrics, Tuning Analyst, balance harness, review UI | Approve tuning guardrails |
| **M7** | Scale and polish | Large maps in the viewer, live delayed streaming, commentary track, admin console | Security review; go/no-go for a public league |

### 11.4 Performance budgets (CI-enforced)

- Worst-case 1024² clash (480 drones × 8,000 ticks × full fuel) finishes in **under 90 s** on 8 cores. This is the reason for the parallel think phase.
- A 500-tick practice sim at any size finishes in **under 5 s**.
- Beat resolution in the orchestrator takes **under 500 ms**.
- The viewer runs 1024² replays at 60 fps on a mid-range laptop.

---

## 12. Security and integrity

- **Sandbox:** LatticePy has no I/O, imports, reflection, or host calls. Fuel, memory, recursion depth, and compile size are capped. Sandbox escape fuzzing runs continuously in CI.
- **Seat isolation:** each Pilot holds one seat token; the Gateway enforces every visibility rule server-side. Pilots get no web or other external tools during a match.
- **Prompt injection:** chat, enemy code, marks, and blackboard values are untrusted text to every agent. In-game deception is legitimate play, but the only tools agents have are game actions, so injected text can't reach anything outside the match. Comment stripping (§10) limits the obvious channel.
- **No cross-team channels:** the same model may play on both sides; contexts are fully separate.
- **Audit:** every action is logged with seat, beat, and idempotency key; matches can be replayed and re-verified at any time.

---

## 13. Open questions

1. **Hosting model:** a long-running cluster, or each match as a job on a serverless or batch platform? Jobs are cheaper at low volume.
2. **Public league:** whether ratings and replays are public, and under what terms.
3. **Open-weights hosting:** whether to self-host small models (more control, steady cost) or rely on OpenRouter.
4. **Spectator delay:** one epoch behind, or a fixed time delay?
5. **Human seats:** worth reserving design space now, or defer until after M7?
