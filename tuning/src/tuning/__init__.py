"""LATTICE tuning system (``spec/system-design.md`` §6).

This package will own the feedback and tuning pipeline: metrics ETL, the
Tuning Analyst agent, the balance harness, and the review UI that turn Retro
feedback and match metrics into proposed ruleset changes for a human to
approve.
"""
