//! Code checker and quota counter (system design §2.1, latticepy §1, §2, §6–§8;
//! engine API contract §1).
//!
//! [`check`] parses a module with a pinned third-party Python parser, lowers it
//! into the crate-owned [`ast`] (rejecting everything outside the allow-list),
//! validates the module interface and counts AST nodes for the complexity quota.
//! It never panics and never recurses unboundedly, whatever the input.
//!
//! # Diagnostic codes
//!
//! Codes are stable and append-only (engine API contract §1); every one is an
//! `Error` today.
//!
//! | Code | Meaning |
//! |---|---|
//! | `E_SYNTAX` | the source does not parse |
//! | `E_ENCODING` | invalid UTF-8 ([`check_bytes`]) |
//! | `E_NUL` | NUL byte in the source |
//! | `E_SIZE` | source over [`MAX_SOURCE_BYTES`] or too many tokens |
//! | `E_NESTING` | brackets, blocks or expressions nested too deeply |
//! | `E_NODE_CAP` | more than [`MAX_CODE_NODES`] AST nodes |
//! | `E_INTERFACE` | `init`/`tick`/handler missing or wrongly declared |
//! | `E_IMPORT` | any import other than one `from lattice import ...`, or (once #38 defines the exports) a name `lattice` does not export |
//! | `E_IMPORT_ALIAS` | `from lattice import x as y` (import the name directly) |
//! | `E_TOPLEVEL` | module-level statement other than import, `def`, literal constant |
//! | `E_STMT_BANNED` | `try`, `raise`, `with`, `assert`, `global`, `nonlocal`, `del`, `class`, `async`, `match`, type alias |
//! | `E_EXPR_BANNED` | lambda, `:=`, sets, dict/set comprehension, generator, `await`, `yield`, f-string, `*` |
//! | `E_BANNED_NAME` | a known-banned builtin or module name (`open`, `getattr`, `os`, ...) |
//! | `E_DUNDER` | a dunder name or attribute |
//! | `E_UNKNOWN_NAME` | a name that is not in scope or on the allow-list |
//! | `E_PARAMS` | decorators, `*args`, `**kwargs`, keyword-only/positional-only, non-literal defaults |
//! | `E_ASSIGN_TARGET` | chained assignment, attribute assignment, other bad target |
//! | `E_OPERATOR` | `/`, `**`, `@`, or a banned augmented operator |
//! | `E_ANNOTATION` | annotation that is not a plain name |
//! | `E_LOOP_ELSE` | `for`/`while` with an `else` block |
//! | `E_LOOP_CONTROL` | `break`/`continue` outside a loop |
//! | `E_COMPREHENSION` | more than one `for`, more than one `if`, or async |
//! | `E_CALL` | call of a computed value, or `**kwargs` in a call |
//! | `E_SLICE` | slice step other than `1` or `-1` |
//! | `E_INT_RANGE` | integer literal outside 64-bit signed |
//! | `E_FLOAT` | float or complex literal |
//! | `E_LITERAL` | bytes or ellipsis literal |

// Read by the VM compiler (#47); nothing consumes the fields yet.
#[allow(dead_code)]
pub(crate) mod ast;
mod lower;

use crate::ruleset::Ruleset;
use rustpython_parser::{Mode, Tok, lexer, parse};

/// Hard cap on AST nodes per module (latticepy §6 `max_code_nodes`). A
/// structural sandbox limit, not a tunable; a team's smaller quota
/// (`team_complexity_nodes`) is enforced by the caller against
/// [`CheckReport::node_count`].
pub const MAX_CODE_NODES: usize = 4_000;

/// Largest source accepted, in bytes. Well above what 4,000 nodes need; keeps
/// pathological input from reaching the parser.
pub const MAX_SOURCE_BYTES: usize = 128 * 1024;

/// Deepest allowed nesting of brackets, blocks and expression trees. The
/// checker and the third-party tree are recursive, so this bounds stack use.
pub const MAX_EXPR_DEPTH: usize = 128;

/// Most lexical tokens accepted. A left-leaning chain like `1+1+1+...` makes a
/// tree as deep as it is long, so this also bounds the parser's tree depth.
const MAX_TOKENS: usize = 16_384;

/// Whether a diagnostic blocks deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One problem found in a module, written for an LLM reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable code from the table in the module docs, e.g. `E_IMPORT`.
    pub code: &'static str,
    /// 1-based line.
    pub line: u32,
    /// 1-based column in characters.
    pub col: u32,
    /// What is wrong, where, why it is banned, and what to do instead.
    pub message: String,
}

/// Which entry points a module defines (latticepy §7).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleInterface {
    pub init: bool,
    pub tick: bool,
    pub on_hit: bool,
    pub on_hacked: bool,
    pub on_radio: bool,
}

/// Result of checking one module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// True iff there are no `Error` diagnostics.
    pub ok: bool,
    /// Sorted by `(line, col, code)`.
    pub diagnostics: Vec<Diagnostic>,
    /// Statements plus expressions in the module: the complexity-quota unit.
    pub node_count: u32,
    pub interface: ModuleInterface,
}

/// Required and optional entry points with their exact parameter names
/// (latticepy §7).
const INTERFACE: [(&str, &[&str], bool); 5] = [
    ("init", &["me", "args"], true),
    ("tick", &["me"], true),
    ("on_hit", &["me", "attacker"], false),
    ("on_hacked", &["me", "attacker"], false),
    ("on_radio", &["me", "msg"], false),
];

/// Check LatticePy source (engine API contract §1). A pure function of its
/// arguments; cost is linear in source size, bounded by [`MAX_SOURCE_BYTES`].
/// The ruleset is part of the contract signature for ruleset-driven limits; no
/// per-module limit is read from it today (the team quota is applied by the
/// caller).
pub fn check(source: &str, ruleset: &Ruleset) -> CheckReport {
    check_and_lower(source, ruleset).0
}

/// [`check`] plus the allow-listed tree the VM compiles from (`Some` iff `ok`).
pub(crate) fn check_and_lower(
    source: &str,
    _ruleset: &Ruleset,
) -> (CheckReport, Option<ast::Module>) {
    let mut lower = lower::Lower::new(source);
    let early = |code, pos, message: String| (fail(vec![diag(code, pos, message)]), None);
    let start = ast::Pos { line: 1, col: 1 };
    if let Some(i) = source.find('\0') {
        let pos = lower.pos(i.try_into().unwrap_or_default());
        return early(
            "E_NUL",
            pos,
            "the source contains a NUL character. Remove it.".into(),
        );
    }
    if source.len() > MAX_SOURCE_BYTES {
        let msg = format!(
            "the source is {} bytes; the limit is {MAX_SOURCE_BYTES}. Split it into smaller modules.",
            source.len()
        );
        return early("E_SIZE", start, msg);
    }
    if let Some(d) = prescan(source, &lower) {
        return (fail(vec![d]), None);
    }
    let parsed = match parse(source, Mode::Module, "<lattice>") {
        Ok(m) => m,
        Err(e) => {
            return early(
                "E_SYNTAX",
                lower.pos(e.offset),
                format!("syntax error: {}", e.error),
            );
        }
    };
    let rustpython_parser::ast::Mod::Module(m) = parsed else {
        return early("E_SYNTAX", start, "expected a module".into());
    };
    let module = lower.module(&m.body);
    let mut diagnostics = std::mem::take(&mut lower.diags);
    let interface = interface(&module, &mut diagnostics);
    let node_count = u32::try_from(lower.nodes).unwrap_or(u32::MAX);
    if lower.nodes > MAX_CODE_NODES {
        let msg = format!(
            "the module has {node_count} AST nodes; the hard limit is {MAX_CODE_NODES}. Remove or simplify code."
        );
        diagnostics.push(diag("E_NODE_CAP", start, msg));
    }
    diagnostics.sort_by(|a, b| (a.line, a.col, a.code).cmp(&(b.line, b.col, b.code)));
    let ok = diagnostics.iter().all(|d| d.severity != Severity::Error);
    let report = CheckReport {
        ok,
        diagnostics,
        node_count,
        interface,
    };
    (report, ok.then_some(module))
}

/// [`check`] for raw bytes, rejecting invalid UTF-8 with a diagnostic.
pub fn check_bytes(source: &[u8], ruleset: &Ruleset) -> CheckReport {
    match std::str::from_utf8(source) {
        Ok(s) => check(s, ruleset),
        Err(e) => {
            let line = source[..e.valid_up_to()]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1;
            let pos = ast::Pos { line, col: 1 };
            let msg = "the source is not valid UTF-8. Save it as UTF-8 text.".into();
            fail(vec![diag("E_ENCODING", pos, msg)])
        }
    }
}

fn diag(code: &'static str, pos: ast::Pos, message: String) -> Diagnostic {
    Diagnostic {
        severity: Severity::Error,
        code,
        line: pos.line,
        col: pos.col,
        message,
    }
}

fn fail(diagnostics: Vec<Diagnostic>) -> CheckReport {
    CheckReport {
        ok: false,
        diagnostics,
        node_count: 0,
        interface: ModuleInterface::default(),
    }
}

/// Bound token count and nesting before the recursive parser sees the input.
fn prescan(source: &str, lower: &lower::Lower) -> Option<Diagnostic> {
    let (mut tokens, mut depth) = (0usize, 0usize);
    for item in lexer::lex(source, Mode::Module) {
        // Lexical errors are reported by the parser with better context.
        let Ok((tok, range)) = item else { return None };
        tokens += 1;
        match tok {
            Tok::Lpar | Tok::Lsqb | Tok::Lbrace | Tok::Indent => depth += 1,
            Tok::Rpar | Tok::Rsqb | Tok::Rbrace | Tok::Dedent => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth > MAX_EXPR_DEPTH {
            let msg =
                "brackets or blocks are nested too deeply. Split the code into helper functions.";
            return Some(diag("E_NESTING", lower.pos(range.start()), msg.into()));
        }
        if tokens > MAX_TOKENS {
            let msg = "the module has too many tokens. Remove or simplify code.";
            return Some(diag("E_SIZE", lower.pos(range.start()), msg.into()));
        }
    }
    None
}

/// Check `init`/`tick` exist and entry points use the exact parameter names.
fn interface(module: &ast::Module, out: &mut Vec<Diagnostic>) -> ModuleInterface {
    let mut defined = [false; INTERFACE.len()];
    for (slot, (name, params, required)) in defined.iter_mut().zip(INTERFACE) {
        let found = module.body.iter().find_map(|s| match &s.kind {
            ast::StmtKind::FunctionDef {
                name: n, params: p, ..
            } if n == name => Some((s.pos, p)),
            _ => None,
        });
        let want = format!("def {name}({})", params.join(", "));
        *slot = found.is_some();
        match found {
            None if required => out.push(diag(
                "E_INTERFACE",
                ast::Pos { line: 1, col: 1 },
                format!("the module must define `{want}`; the engine calls it."),
            )),
            Some((pos, p))
                if !p.iter().map(|x| x.name.as_str()).eq(params.iter().copied())
                    || p.iter().any(|x| x.default.is_some()) =>
            {
                let msg = format!(
                    "`{name}` must be declared exactly as `{want}` (no defaults, these parameter names)."
                );
                out.push(diag("E_INTERFACE", pos, msg));
            }
            _ => {}
        }
    }
    let [init, tick, on_hit, on_hacked, on_radio] = defined;
    ModuleInterface {
        init,
        tick,
        on_hit,
        on_hacked,
        on_radio,
    }
}
