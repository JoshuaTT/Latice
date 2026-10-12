# AGENTS.md — orchestrator/

## Purpose

The LATTICE **match runtime**. Owns the match state machine (Briefing →
Muster → (Council → Deploy → Clash → Debrief) × 4 → Result → Retro), the beat
clock and action charges, the deterministic in-beat action resolver, the team
repo, the channel and bandwidth meter, the hand dealer and quota ledger, the
manifest service, the practice sim runner, and the Muster and Retro handlers.
See `spec/system-design.md` §2.2.

## Owned contracts

None yet. Any interface another module consumes gets a schema in
`spec/contracts/` first (root `AGENTS.md` §2).

## Local commands

Run from the repo root:

- `uv sync --all-packages --dev` — sync the workspace
- `uv run pytest` — run the tests
- `uvx ruff check .` and `uvx ruff format --check .` — lint and format check
- `uvx pyright` — strict type check (configured in the root `pyproject.toml`)

## Acceptance tests

- `orchestrator/tests/test_orchestrator.py` — the package imports cleanly
- CI jobs `lint` and `test` stay green on every PR touching this package

## What not to touch

- The other packages (`gateway/`, `pilot/`, `league/`, `tuning/`), the Rust
  crates, `spec/`, `.github/`, `tests/golden/`
- Anything the root `AGENTS.md` §6 reserves for a human
