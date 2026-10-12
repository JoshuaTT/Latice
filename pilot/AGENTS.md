# AGENTS.md — pilot/

## Purpose

The LATTICE **pilot**: the model-agnostic harness that seats any LLM (fast or
slow, frontier or small) as a player. Owns the per-beat loop, the provider
adapters (native tool calling where available, a text-protocol fallback
otherwise), and context management. Every seat runs the same Pilot code with
the same prompts, so differences between seats come from the model, never the
harness. See `spec/system-design.md` §4.

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

- `pilot/tests/test_pilot.py` — the package imports cleanly
- CI jobs `lint` and `test` stay green on every PR touching this package

Tests use scripted players and mock providers only — never real model APIs
(root `AGENTS.md` §9).

## What not to touch

- `pilot/prompts/` — code-owned by @JoshuaTT; player prompts must stay
  strategy-neutral (root `AGENTS.md` §6)
- The other packages (`orchestrator/`, `gateway/`, `league/`, `tuning/`),
  the Rust crates, `spec/`, `.github/`, `tests/golden/`
- Which providers or models are supported — that is a human decision
