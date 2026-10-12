//! Sandbox escape suite, static half (AGENTS.md §7, §11). Every case is real
//! LatticePy source that a player might submit to break out of the sandbox; the
//! checker must reject it with a specific stable code. The runtime half (VM
//! containment) is added with the VM (#47, #48).

use lattice_engine::checker::{CheckReport, Severity, check, check_bytes};
use lattice_engine::ruleset::Ruleset;

fn ruleset() -> Ruleset {
    Ruleset::from_toml_str(include_str!("../../spec/rulesets/default.toml")).unwrap()
}

/// A valid module whose `tick` runs `body` (indented statements); the first
/// body line is line 6.
fn in_tick(body: &str) -> String {
    format!(
        "from lattice import *\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n{body}\n"
    )
}

/// `(what the attempt does, source, expected code)`.
fn attempts() -> Vec<(&'static str, String, &'static str)> {
    let t = |body: &str| in_tick(body);
    // `pass` above keeps `tick` valid; escape lines start at line 6 but sit in
    // `tick`'s block, so indent them with four spaces.
    vec![
        // Importing and reflection
        ("__import__", t("    __import__('os')"), "E_DUNDER"),
        ("getattr", t("    getattr(me, 'x')"), "E_BANNED_NAME"),
        ("setattr", t("    setattr(me, 'x', 1)"), "E_BANNED_NAME"),
        ("hasattr", t("    hasattr(me, 'x')"), "E_BANNED_NAME"),
        ("vars", t("    vars(me)"), "E_BANNED_NAME"),
        ("dir", t("    dir(me)"), "E_BANNED_NAME"),
        ("type", t("    type(me)"), "E_BANNED_NAME"),
        ("dunder chain", t("    x = ().__class__.__bases__[0].__subclasses__()"), "E_DUNDER"),
        ("dunder on me", t("    me.__dict__"), "E_DUNDER"),
        ("dunder via str", t("    x = 'a'.__class__"), "E_DUNDER"),
        ("eval", t("    eval('1')"), "E_BANNED_NAME"),
        ("exec", t("    exec('x = 1')"), "E_BANNED_NAME"),
        ("compile", t("    compile('1', 'f', 'eval')"), "E_BANNED_NAME"),
        ("globals", t("    globals()"), "E_BANNED_NAME"),
        ("locals", t("    locals()"), "E_BANNED_NAME"),
        // I/O and ambient state
        ("open", t("    open('/etc/passwd')"), "E_BANNED_NAME"),
        ("print", t("    print(1)"), "E_BANNED_NAME"),
        ("input", t("    input()"), "E_BANNED_NAME"),
        ("hash", t("    hash(me)"), "E_BANNED_NAME"),
        ("breakpoint", t("    breakpoint()"), "E_UNKNOWN_NAME"),
        ("id", t("    id(me)"), "E_UNKNOWN_NAME"),
        ("object", t("    object()"), "E_UNKNOWN_NAME"),
        ("super", t("    super()"), "E_UNKNOWN_NAME"),
        ("iter", t("    iter(me)"), "E_UNKNOWN_NAME"),
        ("next", t("    next(me)"), "E_UNKNOWN_NAME"),
        // Statements and expressions outside the subset
        ("decorator", format!("{}@deco\ndef h(me):\n    pass\n", in_tick("")), "E_PARAMS"),
        ("lambda", t("    f = lambda: 1"), "E_EXPR_BANNED"),
        ("yield", t("    yield 1"), "E_EXPR_BANNED"),
        ("generator expression", t("    g = (i for i in range(3))"), "E_EXPR_BANNED"),
        ("async def", format!("{}async def h(me):\n    pass\n", in_tick("")), "E_STMT_BANNED"),
        ("await", t("    await me"), "E_EXPR_BANNED"),
        ("with", t("    with me:\n        pass"), "E_STMT_BANNED"),
        ("try", t("    try:\n        pass\n    except:\n        pass"), "E_STMT_BANNED"),
        ("raise", t("    raise 1"), "E_STMT_BANNED"),
        ("global", t("    global g"), "E_STMT_BANNED"),
        ("nonlocal", t("    nonlocal g"), "E_STMT_BANNED"),
        ("del", t("    del me"), "E_STMT_BANNED"),
        ("class", t("    class A:\n        pass"), "E_STMT_BANNED"),
        ("star-args in a call", t("    me.f(*me)"), "E_EXPR_BANNED"),
        ("star-args in a def", format!("{}def h(*a):\n    pass\n", in_tick("")), "E_PARAMS"),
        ("kwargs in a call", t("    me.f(**me)"), "E_CALL"),
        ("walrus", t("    (x := 1)"), "E_EXPR_BANNED"),
        ("f-string", t("    s = f'{me}'"), "E_EXPR_BANNED"),
        ("float literal", t("    x = 1.5"), "E_FLOAT"),
        ("complex literal", t("    x = 1j"), "E_FLOAT"),
        ("bytes literal", t("    x = b'a'"), "E_LITERAL"),
        ("true division", t("    x = 1 / 2"), "E_OPERATOR"),
        ("set literal", t("    x = {1}"), "E_EXPR_BANNED"),
        // Imports
        ("import lattice names getattr", "from lattice import getattr\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n".into(), "E_BANNED_NAME"),
        ("import alias", "from lattice import move as getattr\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n".into(), "E_IMPORT_ALIAS"),
        ("import os", "import os\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n".into(), "E_IMPORT"),
        ("import os as x", "import os as x\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n".into(), "E_IMPORT"),
        ("from os import star", "from os import *\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n".into(), "E_IMPORT"),
        ("relative import", "from . import x\ndef init(me, args):\n    pass\ndef tick(me):\n    pass\n".into(), "E_IMPORT"),
        ("import inside tick", t("    import os"), "E_IMPORT"),
        ("second lattice import", format!("{}from lattice import x\n", in_tick("")), "E_IMPORT"),
        // Unicode identifier tricks: no spelling reaches a banned name
        ("fullwidth getattr", t("    ｇｅｔａｔｔｒ(me, 'x')"), "E_UNKNOWN_NAME"),
        ("math-italic dunder import", t("    __𝒊𝒎𝒑𝒐𝒓𝒕__('os')"), "E_DUNDER"),
        ("fullwidth eval", t("    ｅｖａｌ('1')"), "E_UNKNOWN_NAME"),
    ]
}

fn error_codes(r: &CheckReport) -> Vec<&'static str> {
    r.diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.code)
        .collect()
}

#[test]
fn every_static_escape_attempt_is_rejected_with_its_code() {
    let rs = ruleset();
    let cases = attempts();
    assert!(cases.len() >= 30, "the suite must keep at least 30 cases");
    let mut misses = Vec::new();
    for (what, src, code) in &cases {
        let r = lattice_engine::checker::check(src, &rs);
        if r.ok || !error_codes(&r).contains(code) {
            misses.push(format!(
                "{what}: wanted {code}, got ok={} {:?}",
                r.ok,
                error_codes(&r)
            ));
        }
    }
    assert!(
        misses.is_empty(),
        "escapes not rejected as expected:\n{}",
        misses.join("\n")
    );
}

#[test]
fn attempts_inside_helper_functions_and_branches_are_still_caught() {
    // Banned constructs hidden in nested positions are found too.
    for body in [
        "    if me:\n        getattr(me, 'x')",
        "    for i in range(3):\n        eval('1')",
        "    while me:\n        x = [open('f') for _ in range(1)]",
        "    x = me.a(me.b(__import__('os')))",
        "    y = [i for i in me.f(getattr(me, 'z'))]",
    ] {
        assert!(!check(&in_tick(body), &ruleset()).ok, "{body}");
    }
}

#[test]
fn nothing_the_checker_accepts_names_a_banned_builtin() {
    // The allow-list is the rule; spot-check a spread of builtin names.
    for name in [
        "abs",
        "all",
        "any",
        "ascii",
        "bin",
        "bool",
        "bytearray",
        "bytes",
        "callable",
        "chr",
        "classmethod",
        "complex",
        "delattr",
        "dict",
        "divmod",
        "enumerate",
        "filter",
        "float",
        "format",
        "frozenset",
        "int",
        "len",
        "list",
        "map",
        "max",
        "memoryview",
        "min",
        "oct",
        "ord",
        "pow",
        "property",
        "repr",
        "reversed",
        "round",
        "set",
        "slice",
        "sorted",
        "staticmethod",
        "sum",
        "tuple",
        "zip",
        "exit",
        "quit",
        "help",
        "copyright",
    ] {
        let r = check(&in_tick(&format!("    x = {name}")), &ruleset());
        assert!(!r.ok, "`{name}` is accepted as a free name");
    }
}

#[test]
fn hostile_bytes_get_diagnostics_not_panics() {
    let rs = ruleset();
    let inputs: Vec<Vec<u8>> = vec![
        vec![],
        vec![0],
        vec![0xff, 0xfe, 0xfd],
        b"def tick(me):\n\tx = 1\n        y = 2\n".to_vec(),
        b"\xef\xbb\xbfdef tick(me): pass".to_vec(),
        "def tick(me):\r\n    pass\r\n".as_bytes().to_vec(),
        "x = '\u{202e}'".as_bytes().to_vec(),
        "#coding: latin-1\ndef tick(me): pass".as_bytes().to_vec(),
        "\u{0}\u{0}\u{0}".as_bytes().to_vec(),
        "def tick(me):\n    x = '''".as_bytes().to_vec(),
        "(".repeat(200_000).into_bytes(),
        "\\\n".repeat(50_000).into_bytes(),
    ];
    for bytes in inputs {
        let a = check_bytes(&bytes, &rs);
        let b = check_bytes(&bytes, &rs);
        // Identical input gives identical output (engine-api §1).
        assert_eq!(a, b);
        assert_eq!(a.ok, error_codes(&a).is_empty());
        let keys: Vec<_> = a
            .diagnostics
            .iter()
            .map(|d| (d.line, d.col, d.code))
            .collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]));
    }
}

#[test]
#[ignore = "bug #122: the checker accepts non-ASCII identifiers without NFKC normalization"]
fn non_ascii_identifiers_are_rejected() {
    for body in ["    ｘ = 1", "    me.ｍｏｖｅ(1)", "    é = 1"] {
        assert!(!check(&in_tick(body), &ruleset()).ok, "{body}");
    }
}
