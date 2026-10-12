# Golden Replay Fixtures

Golden replays are seeded matches replayed from a canonical recording. The CI
`determinism` job replays every fixture and compares state hashes. If a hash
changes, the engine changed its observable behavior — the change must be
explained and approved.

**Changing a golden fixture changes what "correct" means.** This directory is
code-owned by @JoshuaTT (see `.github/CODEOWNERS`). Any PR that adds, removes,
or updates a golden fixture needs human approval.

---

## Directory layout

```
tests/golden/
  README.md               # this file
  <match-id>/             # one directory per golden match, e.g. "clash-1024-v1"
    match.json            # match identity and configuration
    manifests/            # the exact manifest.toml deployed per team per epoch
      <team>/             # "alpha" or "bravo"
        epoch-1.toml
        epoch-2.toml
        ...
    inputs/               # player inputs (seat actions, messages) by epoch/beat
      epoch-1/
        beat-01/
          seat-A1.json
          seat-A2.json
          ...
        beat-02/
          ...
        ...
      epoch-2/
        ...
    expected/
      hashes.json         # tick → state hash at snapshot cadence
```

Matches are added under `tests/golden/` by M1 fixture generator tooling. M0
has no match directories — only this README. The CI `determinism` job uses
`lattice-cli replay --verify tests/golden`, which exits 0 with a notice when
no match directories are found.

---

## `match.json` — match identity

```jsonc
{
  "match_id": "clash-1024-v1",         // matches the directory name
  "seed": "a1b2c3d4e5f6...",           // 256-bit hex seed; must be deterministic
  "ruleset_version": "1.0.0",          // identifies the ruleset in spec/rulesets/
  "board_size": 1024,                  // square side length; powers of 64 per game-design §4.0
  "mutators": ["no_relics"],           // list of active mutator ids; empty for default
  "teams": {
    "alpha": {
      "model": "gpt-5.2",             // recorded for context; not used during replay
      "seats": ["A1", "A2", "A3", "A4", "A5"]
    },
    "bravo": {
      "model": "claude-opus-5",
      "seats": ["B1", "B2", "B3", "B4", "B5"]
    }
  },
  "clash_length": 8000,               // ticks per clash; 2,000 × sqrt(s) per game-design §4.0
  "snapshot_cadence": 500,            // state hashed every N ticks
  "engine_version": "0.1.0"           // engine version that produced this fixture
}
```

## `manifests/<team>/epoch-<N>.toml` — deployed manifests

One TOML file per team per epoch. The exact `manifest.toml` that achieved quorum
and was deployed. Content matches the [manifest schema](../../spec/contracts/manifest.json) (TBD-M1).

```toml
# manifest.toml minimal example (schema TBD)
epoch = 1
team = "alpha"

[library]
path = "team_lib.py"
content_hash = "sha256:abcdef01..."

[[drones]]
module = "scout"
spawn_positions = [[10, 20], [12, 20]]
count = 2

[blackboard]
strategy = "rush"
```

## `inputs/<epoch>/<beat>/seat-<id>.json` — player inputs

One JSON file per seat per beat. Records every action and message the seat
submitted during that beat. Replaying these inputs against the same seed must
produce identical state.

```jsonc
{
  "seat": "A1",
  "epoch": 1,
  "beat": 1,
  "actions": [
    {
      "action": "write_file",
      "path": "team_lib.py",
      "content": "def init()...",      // exact file content; merged with other writes per repo rules
      "beat_id": "epoch-1-beat-1",
      "idempotency_key": "a1-write-1"
    }
  ],
  "messages": [
    {
      "channel": "team",
      "content": "I'll write the scout module",
      "beat_id": "epoch-1-beat-1"
    }
  ],
  "pass": false                        // true if the seat explicitly passed (took no actions)
}
```

Input files are only present for Council beats (beats 1–16 of each epoch).
Muster, Deploy, Clash, Debrief, and Retro phases have no player inputs.

## `expected/hashes.json` — expected state hashes

```jsonc
{
  "match_id": "clash-1024-v1",
  "hash_function": "SHA-256",
  "serialization_contract": "spec/contracts/state-hash.md#v1",
  "snapshots": [
    {
      "tick": 0,                       // initial state before tick 0 resolves
      "hash": "e3b0c44298fc1c14..."    // SHA-256 of canonical serialization
    },
    {
      "tick": 500,
      "hash": "6a2c8e4d1b3f9075..."
    },
    // ... every snapshot_cadence ticks through final tick
    {
      "tick": 8000,                    // final state after last tick resolves
      "hash": "f7d3a1b9c4e50826..."
    }
  ]
}
```

### Hash calculation

The hash is SHA-256 computed over a canonical serialized world state. The byte
format is defined in [`spec/contracts/state-hash.md`](../../spec/contracts/state-hash.md);
`serialization_contract` names the format version a fixture was produced with
(currently `"spec/contracts/state-hash.md#v1"`).

The serialization must be deterministic — the same world state always produces
the same byte sequence, and therefore the same hash, on every platform and
target (native and WASM). See root `AGENTS.md` §8 for determinism rules.

### Snapshot cadence

Snapshots are taken at tick 0 (before any resolution) and then every
`snapshot_cadence` ticks (default 500). The final snapshot is always included,
even if the clash length is not a multiple of the cadence.

---

## Replaying

```bash
# Verify all fixtures (CI determinism job):
cargo run --release -p lattice-cli -- replay --verify tests/golden

# Verify with WASM parity:
cargo run --release -p lattice-cli -- replay --verify tests/golden --target-parity wasm

# Replay a single match for debugging:
cargo run --release -p lattice-cli -- replay tests/golden/clash-1024-v1
```

When `tests/golden/` is empty (no match directories), `replay --verify` exits 0
with a notice. This is the M0 state — fixtures are added in M1.

---

## Adding a fixture (M1+)

1. Generate the fixture with the M1 fixture generator tooling.
2. Create a directory `tests/golden/<match-id>/` with all required files.
3. Run `replay --verify tests/golden` locally and confirm the hashes match.
4. Open a PR. The PR will carry `needs-human` (CODEOWNERS on `tests/golden/`).
5. @JoshuaTT approves after reviewing what behavior the fixture locks in.

Every fixture addition or change is a behavior commitment. Explain in the PR
what the fixture exercises and why the hashes are the expected values.
