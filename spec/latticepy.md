# LatticePy — the player language (subset spec)

LatticePy is the deterministic Python subset players write drone modules in
(game-design §5). This document is the source of truth for the checker and the VM
(system-design §2.1). It is written so a checker implementer can classify any
CPython program allow/deny from this document alone: every syntactic rule is
stated as the CPython `ast` node names it permits or forbids, and every
VM-enforced limit has a default and a tunable/structural marking.

Design stance: everything not on the allow-list is banned. Where the design left
a choice open, this spec picks one; the genuinely judgment-heavy picks are
collected at the end under "Decisions for @JoshuaTT".

---

## 1. Syntax allow-list

A module is a single file parsed with CPython's `ast`. The checker walks the tree
and accepts only the nodes named here. All other node types are a compile error.

### 1.1 Module structure (`ast.Mod`)

- `Module` only (no `Interactive`, no `Expression`, no `FunctionType`).
- Top level may contain, in any order:
  - exactly one `ImportFrom` with `module == "lattice"` and no `level` (the
    `from lattice import *` / `from lattice import name, ...` form),
  - `FunctionDef` nodes for the module interface (§7),
  - `Assign` of module-level constant names used as defaults (optional; values
    must be literals or tuples/lists/dicts of literals).
- No top-level executable statements other than the constant `Assign` above.
  All behavior lives in functions. (Decisive: no top-level computation, so check
  time and fuel are predictable.)

### 1.2 Statements

| Allowed `ast` node | Notes |
|---|---|
| `Assign` | Single target or tuple target; no attribute targets on names other than `me.mem[...]` via `Subscript`. |
| `AugAssign` | `+= -= *= //= %=` only (operators from §1.3). |
| `AnnAssign` | Annotation ignored by VM but counted; type is not checked. Allowed so models can annotate; annotation must be a `Name`. |
| `Return` | Value optional. |
| `If` | `elif` **allowed** (it is just nested `If`); no restriction on depth. |
| `For` | Over `list`, `tuple`, `range(...)`, or `dict` (insertion order, §9). Target may be a tuple. |
| `While` | Condition must be a boolean expression; no `while True` without a bounded `break` is *not* statically enforced — fuel (§5) bounds it instead. |
| `Break`, `Continue` | Inside loops only. |
| `Expr` | Expression statements (calls, etc.). |
| `Pass` | Anywhere. |
| `Assert` | **Banned** (its message string and `-O` semantics are host-dependent). |
| `With`, `Raise`, `Try` | **Banned** (error semantics are §8, not Python exceptions). |
| `Import` | Banned except the single `ImportFrom` in §1.1. |
| `Global`, `Nonlocal`, `Delete` | Banned. |
| `ClassDef`, `AsyncFunctionDef`, `AsyncFor`, `AsyncWith`, `Await` | Banned. |
| `Yield`, `YieldFrom` | Banned (no generators). |
| `Match` | Banned. |

### 1.3 Expressions

| Allowed `ast` node | Notes |
|---|---|
| `BoolOp` | `And`, `Or` (short-circuit). |
| `BinOp` | Operators: `Add Sub Mult FloorDiv Mod LShift RShift BitAnd BitOr BitXor`. **`Div` (true division) on ints is banned**; `/` is only available on the software float via the `lattice` library (§4). |
| `UnaryOp` | `UAdd USub Not Invert`. |
| `Compare` | Operators `Eq NotEq Lt LtE Gt GtE`; **chained comparison allowed** (desugars to `And` of pairs). `Is`, `IsNot`, `In`, `NotIn` allowed on allowed container types with value-equality semantics. |
| `Call` | Only to: names from the `lattice` library, functions defined in the module, or methods on allowed values / `me`/`team` objects. No `*args`/`**kwargs` (no `Starred` in call args). Keywords allowed. |
| `IfExp` | Allowed. |
| `Subscript` | `list`/`tuple`/`dict` indexing and slicing **allowed** with constant or computed `int` bounds; slice step must be a literal `1` or `-1` (keeps slicing countable). |
| `Attribute` | Only on `me`, `team`, `seen`, and allowed value objects as defined by the `lattice` library. |
| `Name`, `Constant` | `Constant` limited to `int`, `bool`, `str`, `None` (no float literals, no bytes, no ellipsis). |
| `List`, `Tuple`, `Dict` | Allowed; dict preserves insertion order (§9). |
| `ListComp` | **Allowed**, single `comprehension`, no `if` clause nesting beyond one. (Decisive: one generator + optional one filter; keeps fuel counting simple.) |
| `DictComp`, `SetComp`, `GeneratorExp`, `Set` | **Banned** (sets are banned outright, §9). |
| `Lambda` | **Banned** (functions must be named `FunctionDef`; keeps quota counting and telemetry readable). |
| `FormattedValue` / `JoinedStr` (f-strings) | **Banned** (string building is via `+` on `str` and `str()` conversion). |
| `Starred` | Banned. |
| `NamedExpr` (`:=`) | Banned. |
| `Await`/`Yield` | Banned (see §1.2). |

### 1.4 Functions

- `FunctionDef` with positional-or-keyword parameters only. No `*args`, no
  `**kwargs`, no positional-only / keyword-only markers, no default values that
  are non-literals.
- `decorator_list` must be empty. `returns` annotation allowed but ignored.
- Recursion is allowed but bounded by the recursion-depth limit (§6).

---

## 2. Banned list (named explicitly)

Everything not on the allow-list is denied by default. By name, the checker must
reject:

- `import` of anything except `from lattice import ...` (single statement, §1.1).
- All I/O: `open`, `print` (use `me.log`), `input`, `os`, `sys`, `io`.
- Wall clock: `time`, `datetime`, `me.now` is the *tick counter* (game time), not
  a clock — no real-time access exists.
- Unseeded randomness: `random`, `os.urandom`, `hash`-based ordering.
- Reflection / introspection: `getattr`, `setattr`, `hasattr`, `vars`, `dir`,
  `type`, `isinstance` (on non-allowed values), `globals`, `locals`, `eval`,
  `exec`, `compile`, `__import__`, dunder access (`me.__dict__`, etc.).
- Threads / async: `threading`, `asyncio`, `async`/`await`.
- Generators: `yield`, `GeneratorExp`, generator functions.
- Classes: `class` definitions and user-defined types.
- `*args` / `**kwargs`, `global`, `nonlocal`, `del`.
- Sets and set literals (§9).
- Float literals and host float arithmetic (§4).

---

## 3. Values

Exactly these value types exist at runtime; anything else is a check error:

| Type | Notes / cap |
|---|---|
| `int` | Arbitrary precision is *not* allowed: capped at **64-bit signed** (−2⁶³ .. 2⁶³−1). Overflow is a runtime error (§8). |
| `bool` | `True`/`False`. |
| `str` | Capped length `max_string_len` (§6). Immutable. |
| `None` | |
| `list` | Homogeneous-ness not enforced; capped length (§6). |
| `tuple` | Immutable; capped length (same as list). |
| `dict` | **Insertion-ordered** (system-design §2.1). Keys limited to `int`, `str`, `bool`, `None`, or tuples of those (hashable-by-value, §9). Capped size. |
| software float | The `lattice` library's float type (§4). Never a host float. |

No other types. In particular: no `set`, no `bytes`, no `float` (host), no
functions-as-values beyond module function references, no objects except the
`me`/`team`/`seen` handles provided by `lattice`.

Equality (`==`) is structural value equality for list/tuple/dict; identity (`is`)
is only meaningful for `None`.

---

## 4. Float policy

Host floats are banned. All non-integer arithmetic uses the `lattice` library's
**software IEEE 754 double** (`lattice.Float`, exposed as `F`). The VM implements
it in software (integer limbs) on every target so native and WASM agree bit-for-bit
(determinism rule, AGENTS.md §8).

Provided operations (exactly these):

- `F.add(a, b)`, `F.sub(a, b)`, `F.mul(a, b)`, `F.div(a, b)` — the four basic IEEE
  754 operations, round-to-nearest-even, no fused multiply-add.
- Comparisons: `F.lt/le/gt/ge/eq/ne`.
- Conversions: `F.from_int(i)`, `F.to_int(f)` (round-toward-zero), `F.from_str`
  **banned** (parsing is host-dependent), `F.to_str(f, digits)` (fixed-point,
  deterministic).

Not provided: `%`, `**`, unary negation as a separate op (use `F.sub(0, x)`), and
all transcendental / `math` functions. **Decisive: there is no software `math`
library in v1.** If a module needs `sqrt`/`atan2` etc., the `lattice` game library
exposes the few geometry helpers it needs (`me.distance(a, b)`, `me.angle_to(b)`)
implemented in software float; the language itself stays at +, -, *, / and
comparisons. (One-line justification: a closed op set is what makes "deterministic
floats" provable; transcendental helpers belong in the game library, which already
documents fuel costs.)

Division by zero on `F.div` is a runtime error (§8), not NaN/Inf; NaN and Inf are
not representable.

---

## 5. Fuel metering

Fuel is counted at the **AST level** (decisive: one fuel op per AST node evaluated,
so the quota counter and the fuel meter share one counting rule and the checker's
50 ms budget is trivially met). Concretely:

- Evaluating one expression/statement node costs **1 fuel op** (each `BinOp`,
  `Compare`, `Call`, `Subscript`, `Attribute`, loop iteration header, etc.).
- A function **call** adds a fixed overhead of `call_overhead` fuel ops (default
  **4**: frame setup + argument binding + return), on top of the callee's nodes.
- A **loop iteration** costs the cost of its body plus **1** for the iteration
  step (the `For`/`While` node re-evaluation).
- **Attribute access** on `me`/`team`/`seen` that triggers a query (e.g.
  `me.sense(...)`, `me.path_to(...)`) costs its documented library fuel cost
  (system-design §2.1: every `lattice` call has a documented fuel cost) *in
  addition* to the 1 op for the `Call` node. Plain field reads (`me.pos`) cost 1.
- Software-float ops cost `float_op` fuel ops each (default **4**), since they are
  multi-limb integer arithmetic.

When a drone's fuel for the tick is exhausted, execution stops for that tick
(game-design §4.2); partial acts are discarded (acts only commit in the resolve
phase). The fuel-per-tick *value* lives in the ruleset, not here.

---

## 6. Limits

| Limit | Default | Marking |
|---|---|---|
| `fuel_per_tick` | 1,000 ops | tunable (ruleset; game-design §14.2) |
| `max_recursion_depth` | 32 | tunable |
| `max_value_count` (live values per drone: list/tuple/dict elements + dict entries + string chars share one budget) | 65,536 | tunable |
| `max_code_nodes` (AST nodes per module = the complexity-quota unit) | bounded by the player's complexity quota (game-design §7); the VM hard-caps at 4,000 | structural cap, tunable quota |
| `max_string_len` | 256 | tunable |
| `max_list_len` / `max_tuple_len` | 1,024 | tunable |
| `max_dict_size` | 1,024 | tunable |
| `max_modules_per_library` | bounded by manifest `library`; VM hard-cap 16 | structural |
| `max_mem_keys` (`me.mem` entries) | 256 | tunable |

All tunables live in the ruleset once the ruleset schema lands; the structural
caps are part of the sandbox boundary and change only with human approval
(AGENTS.md §6).

---

## 7. Module interface

A module defines exactly:

```python
def init(me, args): ...     # once, when the drone spawns; args from the spawner
def tick(me): ...           # every tick, in the think phase
# optional:
def on_hit(me, attacker): ...
def on_hacked(me, attacker): ...
def on_radio(me, msg): ...
```

- `init` and `tick` are required; the three handlers are optional. Any other
  top-level function is allowed as a helper but is not called by the engine.
- Handlers run in the think phase against the read-only tick snapshot; they may
  compute freely and queue **at most one act** per tick (§5.3 game-design). Extra
  act calls in one tick are ignored after the first (decisive: first act wins, so
  behavior is deterministic regardless of code path).
- Queries and `lattice` calls are allowed in any handler.
- `me.mem` is a capped dict (§6) that persists across ticks **and epochs** for the
  drone's lifetime (a captured drone keeps its memory; the payload module starts
  fresh with a new `me.mem` on hack — decisive, prevents worms smuggling state).
- The checker validates that `init`/`tick` exist with exactly these parameter
  names, and that handlers, if present, use the exact signatures above.

---

## 8. Error semantics

LatticePy has no exceptions. A runtime error **stops that drone for that tick**;
the error is recorded and surfaced only in the owner's telemetry shard
(game-design §9.2). It never propagates to the engine or to other drones, and it
never crashes the simulation.

Runtime errors (each a named code in telemetry):

- `div_by_zero`: int `//`/`%` by zero, or `F.div` by zero.
- `int_overflow`: result outside 64-bit signed range.
- `fuel_exhausted`: fuel reached 0 mid-tick (also the normal stop condition).
- `cap_violation`: exceeding any §6 limit (string/list/dict/mem/recursion/value
  count).
- `bad_call`: a `lattice` library call with an argument of the wrong type or
  a semantically invalid value (spec/lattice-api.md §0). The call is not
  executed; soft failures (legitimately unavailable results) are *not* this
  error — they return `None`/`False`.
- `index_error` / `key_error`: out-of-range subscript or missing dict key.
- `check_error`: a compile-time violation (banned node) — the module is rejected
  at `lattice check` time and cannot be deployed; this is not a runtime error.

`on_hit`, `on_hacked`, `on_radio` errors are likewise contained to the tick.

---

## 9. Determinism guarantees

- **Iteration order:** `dict` iterates in insertion order; `list`/`tuple` in index
  order. **Sets are banned**, so there is no unordered iteration anywhere
  (AGENTS.md §8).
- **No observable hashing:** dict lookup is by value equality (implemented as
  ordered linear/bucket search), never by a randomized hash; `hash()` is banned.
- **Same seed → same result:** the only randomness is `me.rand()`, seeded from the
  match seed plus the drone's stream ID (AGENTS.md §8); `me.now` is the tick
  counter; there is no wall clock, environment, filesystem, or network access.
- **Native and WASM parity:** the software float and AST fuel meter are target-
  independent by construction; the nightly parity job verifies.

---

## Decisions for @JoshuaTT

These are the genuinely-open choices this spec resolved; each is flagged for
sign-off at the M0 checkpoint:

1. **No software `math` library in v1** (§4). Floats get only +, -, *, / and
   comparisons; geometry helpers live in the `lattice` game library instead.
   (Alternative: a full software `math` module — more expressive, much harder to
   prove bit-identical.)
2. **AST-level fuel metering, 1 op per node** (§5), with call overhead 4 and float
   op 4. (Alternative: bytecode-level metering — finer, but doubles the counting
   rules the checker and VM must agree on.)
3. **`lambda`, f-strings, `assert`, `Try/Raise`, sets, and generators banned;
   `elif`, chained comparison, slicing (step ±1), and single-generator list
   comprehensions allowed** (§1). (Alternative: allow `Try` for local error
   handling — rejected because error semantics are §8's containment model, not
   exceptions.)
4. **First act wins** when a handler queues more than one act in a tick (§7).
   (Alternative: last act wins — rejected; first-wins makes act selection
   independent of later code paths.)
5. **`me.mem` resets on hack** (§7): a captured drone runs the payload with fresh
   memory. (Alternative: carry memory across capture — rejected; it lets worms
   smuggle state between teams.)
6. **64-bit capped ints** (§3). (Alternative: bigint — rejected; unbounded ints make
   fuel and memory budgets uncountable.)
