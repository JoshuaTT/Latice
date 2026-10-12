# AGENTS.md — league/

## Purpose

The LATTICE **league services**. Owns the scheduler, the latency calibrator
(measuring each model's p90 response time and assigning latency classes),
ratings (TrueSkill per model and per team composition), the budget
controller, and observability for matches and provider calls. See
`spec/system-design.md` §2.5.

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

- `league/tests/test_league.py` — the package imports cleanly
- CI jobs `lint` and `test` stay green on every PR touching this package

## What not to touch

- The other packages (`orchestrator/`, `gateway/`, `pilot/`, `tuning/`), the
  Rust crates, `spec/`, `.github/`, `tests/golden/`
- Provider API keys, spending limits, and which models are in a league —
  those are human decisions (root `AGENTS.md` §6)
