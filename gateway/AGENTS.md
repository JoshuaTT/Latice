# AGENTS.md — gateway/

## Purpose

The LATTICE **player gateway**: the only door into a match for players. Owns
the MCP server and the equivalent HTTP/JSON API — one token per seat,
server-side enforcement of every visibility rule, charges, and idempotency
(every action carries `beat_id` and an idempotency key) — plus the Field
Manual and `help` system and each seat's private scratchpad. See
`spec/system-design.md` §2.3.

Visibility bugs are game-breaking (root `AGENTS.md` §10): test every
visibility rule from both the allowed and the denied side.

## Owned contracts

Gateway tool schemas will live in `spec/contracts/` (JSON Schema draft
2020-12). None exist yet; never change a tool interface without changing its
contract first.

## Local commands

Run from the repo root:

- `uv sync --all-packages --dev` — sync the workspace
- `uv run pytest` — run the tests
- `uvx ruff check .` and `uvx ruff format --check .` — lint and format check
- `uvx pyright` — strict type check (configured in the root `pyproject.toml`)

## Acceptance tests

- `gateway/tests/test_gateway.py` — the package imports cleanly
- CI jobs `lint` and `test` stay green on every PR touching this package

## What not to touch

- The other packages (`orchestrator/`, `pilot/`, `league/`, `tuning/`), the
  Rust crates, `spec/`, `.github/`, `tests/golden/`
- Anything the root `AGENTS.md` §6 reserves for a human
