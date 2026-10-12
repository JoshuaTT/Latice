//! Checker tests (latticepy §1, §2, §7; game-design §5.8). Expectations come
//! from the spec, not from current behavior.

use lattice_engine::checker::{CheckReport, MAX_CODE_NODES, Severity};
use lattice_engine::ruleset::Ruleset;
use proptest::prelude::*;

fn ruleset() -> Ruleset {
    Ruleset::from_toml_str(include_str!("../../spec/rulesets/default.toml")).unwrap()
}

fn check(src: &str) -> CheckReport {
    lattice_engine::checker::check(src, &ruleset())
}

fn check_bytes(src: &[u8]) -> CheckReport {
    lattice_engine::checker::check_bytes(src, &ruleset())
}

const SCOUT: &str = r#"from lattice import EAST, turn_right

def init(me, args):
    me.mem["dir"] = args.get("dir", EAST)

def tick(me):
    for e in me.sense(radius=5).enemies():
        me.mark(e.pos, "danger", ttl=150)
        team.board.put("threat", {"x": e.pos.x, "y": e.pos.y, "t": me.now})
    if me.blocked(me.mem["dir"]):
        me.mem["dir"] = turn_right(me.mem["dir"])
    me.move(me.mem["dir"])
"#;

const WARDEN: &str = r#"from lattice import Pos, guard

def tick(me):
    threat = team.board.get("threat")
    if threat and me.now - threat["t"] < 50:
        target = Pos(threat["x"], threat["y"])
        foe = me.nearest_enemy(within=2)
        if foe and foe.firewall <= 2:
            me.hack(foe, payload="warden")
        else:
            me.step_toward(target)
    else:
        guard(me, team.board.get("home"))
"#;

const WRAP: &str = "from lattice import *\ndef init(me, args):\n    pass\n";

/// Wrap `body` (indented statements) in a valid module's `tick`.
fn in_tick(body: &str) -> String {
    format!("{WRAP}def tick(me):\n{body}\n")
}

fn assert_rejected(src: &str, construct: &str, line: u32) {
    let r = check(src);
    assert!(!r.ok, "accepted: {src}");
    let d = r
        .diagnostics
        .iter()
        .find(|d| d.message.contains(construct))
        .unwrap_or_else(|| panic!("no diagnostic naming `{construct}` in {:?}", r.diagnostics));
    assert_eq!(d.line, line, "{construct}: {d:?}");
}

#[test]
fn scout_passes_with_pinned_node_count() {
    let r = check(SCOUT);
    assert!(r.ok, "{:?}", r.diagnostics);
    assert_eq!(r.node_count, 74);
    assert!(r.interface.init && r.interface.tick);
    assert!(!r.interface.on_hit && !r.interface.on_hacked && !r.interface.on_radio);
}

#[test]
fn warden_lacks_required_init_per_latticepy_7() {
    // game-design §5.8 shows a module without `init`, but latticepy §7 requires
    // it; the checker follows latticepy §7 (reported on the issue).
    let r = check(WARDEN);
    assert_eq!(r.diagnostics.len(), 1, "{:?}", r.diagnostics);
    assert!(r.diagnostics[0].message.contains("def init(me, args)"));
    assert_eq!(r.node_count, 63);
}

#[test]
fn banned_constructs_are_rejected_with_construct_and_line() {
    let line = 5; // `def tick` is line 4, its first body line is 5
    let cases: &[(&str, &str)] = &[
        ("    print(1)", "print"),
        ("    x = open(1)", "open"),
        ("    input()", "input"),
        ("    os.x", "os"),
        ("    sys.x", "sys"),
        ("    io.x", "io"),
        ("    time.x", "time"),
        ("    datetime.x", "datetime"),
        ("    random.x", "random"),
        ("    getattr(me, 1)", "getattr"),
        ("    setattr(me, 1, 2)", "setattr"),
        ("    hasattr(me, 1)", "hasattr"),
        ("    vars(me)", "vars"),
        ("    dir(me)", "dir"),
        ("    type(me)", "type"),
        ("    isinstance(me, 1)", "isinstance"),
        ("    globals()", "globals"),
        ("    locals()", "locals"),
        ("    eval(1)", "eval"),
        ("    exec(1)", "exec"),
        ("    compile(1)", "compile"),
        ("    __import__(1)", "__import__"),
        ("    me.__dict__", "__dict__"),
        ("    threading.x", "threading"),
        ("    asyncio.x", "asyncio"),
        ("    hash(1)", "hash"),
        ("    x = {1, 2}", "set"),
        ("    x = set()", "set"),
        ("    x = 1.5", "float literal"),
        ("    x = 1 / 2", "/"),
        ("    x = 2 ** 3", "**"),
        ("    import os", "import"),
        ("    from os import path", "import"),
        ("    try:\n        pass\n    except:\n        pass", "try"),
        ("    raise 1", "raise"),
        ("    with me:\n        pass", "with"),
        ("    assert 1", "assert"),
        ("    global g", "global"),
        ("    del me", "del"),
        ("    class A:\n        pass", "class"),
        ("    x = lambda: 1", "lambda"),
        ("    x = f'{1}'", "f-string"),
        ("    x = (y := 1)", ":="),
        ("    x = (i for i in [])", "generator expression"),
        ("    x = {i: i for i in []}", "dict comprehension"),
        ("    x = {i for i in []}", "set"),
        (
            "    x = [i for i in [] for j in []]",
            "nested comprehension",
        ),
        ("    x = [i for i in [] if i if i]", "comprehension filters"),
        ("    x = [*me]", "*"),
        ("    me.f(*me)", "*"),
        ("    me.f(**me)", "**kwargs"),
        ("    x = {**me}", "**mapping"),
        ("    me.x = 1", "attribute assignment"),
        ("    a = b = 1", "chained assignment"),
        ("    x /= 2", "augmented assignment"),
        ("    x = [1][::2]", "slice step"),
        ("    x = b'a'", "literal"),
        ("    x = ...", "literal"),
        ("    x = 99999999999999999999", "integer literal"),
        ("    match me:\n        case 1:\n            pass", "match"),
        (
            "    for i in me:\n        pass\n    else:\n        pass",
            "for ... else",
        ),
        ("    break", "break/continue"),
        ("    me.f()()", "call of a computed value"),
        ("    x: me.t = 1", "annotation"),
    ];
    for (body, construct) in cases {
        let multiline = body.contains('\n');
        let r = check(&in_tick(body));
        let d = r.diagnostics.iter().find(|d| d.message.contains(construct));
        let d = d.unwrap_or_else(|| {
            panic!(
                "no diagnostic naming `{construct}` for {body:?}: {:?}",
                r.diagnostics
            )
        });
        // Multi-line constructs may be reported on a later line of the block.
        assert!(
            if multiline {
                d.line >= line
            } else {
                d.line == line
            },
            "{construct}: {d:?}"
        );
    }
}

#[test]
fn async_and_generators_are_rejected() {
    assert_rejected(
        &format!("{WRAP}async def f(me):\n    pass\ndef tick(me):\n    pass\n"),
        "async",
        4,
    );
    assert_rejected(&in_tick("    yield 1"), "yield", 5);
    assert_rejected(&in_tick("    await me"), "await", 5);
}

#[test]
fn bad_function_shapes_are_rejected() {
    let pre = "from lattice import *\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n";
    assert_rejected(&format!("{pre}def f(*a):\n    pass\n"), "*args", 6);
    assert_rejected(&format!("{pre}def f(**k):\n    pass\n"), "**kwargs", 6);
    assert_rejected(
        &format!("{pre}def f(a=[me]):\n    pass\n"),
        "non-literal default",
        6,
    );
    assert_rejected(&format!("{pre}@d\ndef f(a):\n    pass\n"), "decorator", 6);
    assert_rejected(&format!("{pre}x = me()\n"), "top-level assignment", 6);
    assert_rejected(&format!("{pre}print(1)\n"), "top-level statement", 6);
    assert_rejected(&format!("{pre}import lattice\n"), "import", 6);
    assert_rejected(
        &format!("{pre}from lattice import x\n"),
        "from lattice import",
        6,
    );
}

#[test]
fn allowed_forms_pass() {
    let src = in_tick(
        "    x = [i * 2 for i in range(3) if i > 0]\n    a, b = 1, 2\n    y = x[1:2]\n    z = x[::-1]\n    c = 1 < a <= 2\n    n = -a if a else ~b\n    while a:\n        a -= 1\n        if a:\n            continue\n        elif b:\n            break\n    me.mem[\"k\"] = {1: [2], \"s\": None}\n    return None",
    );
    let r = check(&src);
    assert!(r.ok, "{:?}", r.diagnostics);
    let ok = "from lattice import *\nLIMIT = 3\nNAMES = [\"a\", (1, -2)]\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\ndef on_hit(me, attacker):\n    pass\ndef on_hacked(me, attacker):\n    pass\ndef on_radio(me, msg):\n    pass\ndef helper(a: int, b=2):\n    return a\n";
    let r = check(ok);
    assert!(r.ok, "{:?}", r.diagnostics);
}

#[test]
fn all_diagnostics_are_reported() {
    let r = check(&in_tick("    print(1)\n    x = 1.5\n    open(2)"));
    assert!(r.diagnostics.len() >= 3);
}

#[test]
fn interface_violations() {
    let r = check("from lattice import *\ndef tick(me):\n    pass\n");
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("def init(me, args)"))
    );
    let r = check("def init(me, args):\n    pass\ndef tick(drone):\n    pass\n");
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == "E_INTERFACE" && d.line == 3)
    );
    let r = check(
        "def init(me, args):\n    pass\ndef tick(me):\n    pass\ndef on_hit(me):\n    pass\n",
    );
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == "E_INTERFACE" && d.line == 5)
    );
    let r = check("def init(me, args=1):\n    pass\ndef tick(me):\n    pass\n");
    assert!(!r.ok);
}

#[test]
fn syntax_errors_have_positions() {
    let r = check("def tick(me:\n");
    assert!(!r.ok);
    assert_eq!(r.diagnostics[0].code, "E_SYNTAX");
}

#[test]
fn oversized_modules_are_rejected() {
    let body = "    me.f(1)\n".repeat(MAX_CODE_NODES / 5 + 1);
    let r = check(&format!("{WRAP}def tick(me):\n{body}"));
    assert!(!r.ok);
    assert!(r.diagnostics.iter().any(|d| d.code == "E_NODE_CAP"));
}

#[test]
fn adversarial_inputs_get_a_diagnostic_not_a_panic() {
    let deep_parens = format!(
        "{WRAP}def tick(me):\n    x = {}1{}\n",
        "(".repeat(10_000),
        ")".repeat(10_000)
    );
    let deep_unary = format!("{WRAP}def tick(me):\n    x = {}1\n", "-".repeat(10_000));
    let deep_not = format!("{WRAP}def tick(me):\n    x = {}1\n", "not ".repeat(10_000));
    let deep_lists = format!(
        "{WRAP}def tick(me):\n    x = {}{}\n",
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    let long_chain = format!("{WRAP}def tick(me):\n    x = 1{}\n", "+1".repeat(100_000));
    let mut deep_blocks = String::from(WRAP);
    deep_blocks.push_str("def tick(me):\n");
    for i in 0..500 {
        deep_blocks.push_str(&format!("{}if me:\n", " ".repeat(4 + i)));
    }
    let big = format!("{WRAP}# {}\n", "x".repeat(1 << 20));
    for src in [
        deep_parens,
        deep_unary,
        deep_not,
        deep_lists,
        long_chain,
        deep_blocks,
        big,
    ] {
        let r = check(&src);
        assert!(!r.ok);
        assert!(!r.diagnostics.is_empty());
    }
    assert!(!check("\0").ok);
    assert!(!check(&format!("{WRAP}def tick(me):\n    x = 1\0\n")).ok);
    let r = check_bytes(b"def tick(me):\n    x = \xff\xfe\n");
    assert!(!r.ok);
    assert_eq!(r.diagnostics[0].line, 2);
}

#[test]
fn largest_accepted_chain_does_not_overflow_a_small_stack() {
    // 2 MiB is the default test-thread stack; the deepest tree the token cap
    // lets through must still be handled on it.
    let src = format!("{WRAP}def tick(me):\n    x = 1{}\n", "+1".repeat(8_000));
    let r = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || check(&src))
        .unwrap()
        .join()
        .unwrap();
    assert!(!r.ok || r.node_count > 0);
}

proptest! {
    #[test]
    fn arbitrary_strings_never_panic(s in "\\PC*") {
        let _ = check(&s);
    }

    #[test]
    fn arbitrary_pythonish_text_never_panics(s in "[ a-z0-9_()\\[\\]{}:,.=+*/<>!\\-\\n\"'#@\\\\]{0,400}") {
        let _ = check(&s);
    }
}

// The wall clock is banned in game logic (AGENTS.md §8); this test only measures
// the checker's 50 ms budget (system-design §2.1) and never feeds a simulation.
#[test]
#[allow(clippy::disallowed_methods, clippy::disallowed_types)]
fn four_thousand_node_module_checks_quickly() {
    let body = "    me.f(1)\n".repeat(MAX_CODE_NODES / 5 - 10);
    let src = format!("{WRAP}def tick(me):\n{body}");
    let start = std::time::Instant::now();
    let r = check(&src);
    let elapsed = start.elapsed();
    assert!(r.ok, "{:?}", r.diagnostics.first());
    assert!(r.node_count as usize > MAX_CODE_NODES * 9 / 10);
    // The 50 ms budget (system design §2.1) is for release builds; debug builds
    // get slack so CI stays stable.
    let budget = if cfg!(debug_assertions) { 1_000 } else { 50 };
    assert!(elapsed.as_millis() < budget, "took {elapsed:?}");
}

#[test]
fn names_are_allow_listed() {
    // Builtins and names the spec does not grant are denied by default.
    for name in [
        "id",
        "object",
        "super",
        "breakpoint",
        "iter",
        "next",
        "totally_undefined",
    ] {
        let r = check(&in_tick(&format!("    x = {name}(me)")));
        let d = r
            .diagnostics
            .iter()
            .find(|d| d.message.contains(&format!("`{name}`")));
        let d = d.unwrap_or_else(|| panic!("`{name}` accepted: {:?}", r.diagnostics));
        assert_eq!(d.code, "E_UNKNOWN_NAME");
        assert_eq!(d.line, 5);
    }
    // Names in scope are accepted: parameter, local, loop and comprehension
    // targets, module constant, module function, `range`/`str`, explicit import.
    let src = "from lattice import EAST\nLIMIT = 3\ndef helper(a):\n    return a\ndef init(me, args):\n    pass\ndef tick(me):\n    n = LIMIT\n    for i in range(n):\n        n = helper(str(i))\n    ys = [j for j in range(2)]\n    me.move(EAST, ys, later())\ndef later():\n    return 1\n";
    let r = check(src);
    assert!(r.ok, "{:?}", r.diagnostics);
    // Use before assignment and use of another function's local are rejected.
    let r = check(&in_tick("    y = x\n    x = 1"));
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == "E_UNKNOWN_NAME" && d.line == 5)
    );
    let src = format!("{WRAP}def f(me):\n    local = 1\ndef tick(me):\n    return local\n");
    assert!(
        check(&src)
            .diagnostics
            .iter()
            .any(|d| d.code == "E_UNKNOWN_NAME")
    );
    // Known-bad names keep their friendlier code.
    let r = check(&in_tick("    open(1)"));
    assert_eq!(r.diagnostics[0].code, "E_BANNED_NAME");
    // The message suggests the closest allowed name.
    let r = check(&in_tick("    x = rnge(3)"));
    assert!(
        r.diagnostics[0].message.contains("Did you mean `range`?"),
        "{:?}",
        r.diagnostics
    );
}

#[test]
fn report_follows_the_engine_api_contract() {
    let r = check(&in_tick("    print(1)\n    x = 1.5\n    open(2)"));
    assert!(!r.ok);
    assert!(
        r.diagnostics
            .iter()
            .all(|d| d.severity == Severity::Error && d.code.starts_with("E_"))
    );
    let keys: Vec<_> = r
        .diagnostics
        .iter()
        .map(|d| (d.line, d.col, d.code))
        .collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
    let r = check("def tick(me):\n    pass\ndef on_hit(me, attacker):\n    pass\n");
    assert!(r.interface.tick && r.interface.on_hit && !r.interface.init);
    // Identical input gives identical output.
    assert_eq!(check(SCOUT), check(SCOUT));
}

#[test]
fn import_list_cannot_smuggle_banned_names() {
    for (name, code) in [
        ("getattr", "E_BANNED_NAME"),
        ("id", "E_UNKNOWN_NAME"),
        ("object", "E_UNKNOWN_NAME"),
        ("__builtins__", "E_DUNDER"),
        ("open", "E_BANNED_NAME"),
    ] {
        let src = format!(
            "from lattice import {name}\ndef init(me, args):\n    pass\ndef tick(me):\n    x = {name}(me)\n"
        );
        let r = check(&src);
        assert!(!r.ok, "`from lattice import {name}` accepted");
        if code != "E_UNKNOWN_NAME" {
            assert!(
                r.diagnostics.iter().any(|d| d.code == code && d.line == 1),
                "{name}: {:?}",
                r.diagnostics
            );
        }
    }
}

#[test]
fn plain_imports_report_e_import_at_module_level_and_in_functions() {
    let body = "def init(me, args):\n    pass\ndef tick(me):\n    pass\n";
    for src in [
        format!("import os\n{body}"),
        format!("import os as x\n{body}"),
        format!("import os, sys\n{body}"),
        "def init(me, args):\n    import os\ndef tick(me):\n    pass\n".to_owned(),
    ] {
        let r = check(&src);
        assert!(!r.ok, "accepted: {src}");
        let codes: Vec<_> = r.diagnostics.iter().map(|d| d.code).collect();
        assert_eq!(codes, ["E_IMPORT"], "{src}");
        assert!(
            r.diagnostics[0].message.contains("no other modules"),
            "{src}"
        );
    }
    // A misplaced non-import statement keeps its own code.
    let r = check(&format!("print(1)\n{body}"));
    assert!(r.diagnostics.iter().any(|d| d.code == "E_TOPLEVEL"));
}

#[test]
fn import_aliases_are_rejected() {
    let src = "from lattice import foo as bar\ndef init(me, args):\n    pass\ndef tick(me):\n    bar(1)\n";
    let r = check(src);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.code == "E_IMPORT_ALIAS" && d.line == 1)
    );
}

#[test]
#[ignore = "needs the lattice export table from #38; imports are only checked for banned names until then"]
fn unknown_lattice_exports_are_rejected() {
    let src = "from lattice import not_a_real_export\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n";
    assert!(!check(src).ok);
}
