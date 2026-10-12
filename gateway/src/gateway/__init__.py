"""LATTICE player gateway (``spec/system-design.md`` §2.3).

This package will own the MCP server and the equivalent HTTP/JSON player API:
one token per seat, server-side enforcement of visibility rules, charges, and
idempotency, plus the Field Manual, the ``help`` system, and each seat's
private scratchpad.
"""
