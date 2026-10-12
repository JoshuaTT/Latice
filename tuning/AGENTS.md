# AGENTS.md — tuning/

## Purpose

The LATTICE **tuning system**: the pipeline that turns Retro feedback and
match metrics into proposed ruleset changes. Owns metrics ETL, the Tuning
Analyst agent, the balance harness, and the review UI a human approves
changes in. Guardrails: at most 3 parameter changes per release, structural
parameters (quorum, team size, win conditions) always need human sign-off,
and ratings merge only between compatible ruleset versions. See
`spec/system-design.md` §6.

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

- `tuning/tests/test_tuning.py` — the package imports cleanly
- CI jobs `lint` and `test` stay green on every PR touching this package

## What not to touch

- `tuning/guardrails/` — code-owned by @JoshuaTT; limits on automatic rule
  changes (root `AGENTS.md` §6)
- The other packages (`orchestrator/`, `gateway/`, `pilot/`, `league/`), the
  Rust crates, `spec/`, `.github/`, `tests/golden/`
- Any ruleset promotion: humans approve, agents only propose
