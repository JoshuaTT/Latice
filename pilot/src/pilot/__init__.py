"""LATTICE pilot harness (``spec/system-design.md`` §4).

This package will seat any LLM as a player with an identical harness: the
per-beat loop, the provider adapters (with a text-protocol fallback for
models without native tool calling), and context management. Differences
between seats must come from the model, never from the harness.
"""
