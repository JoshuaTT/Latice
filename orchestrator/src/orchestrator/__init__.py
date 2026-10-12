"""LATTICE match runtime (``spec/system-design.md`` §2.2).

This package will own the match state machine (Briefing, Muster, the four
epochs of Council, Deploy, Clash, Debrief, Result, and Retro), the beat clock
and action charges, the deterministic in-beat action resolver, the team repo,
the channel and bandwidth meter, the hand dealer and quota ledger, the
manifest service, the practice sim runner, and the Muster and Retro handlers.
"""
