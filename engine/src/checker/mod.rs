//! Code checker and quota counter (system design §2.1, latticepy §1, §2, §6–§8).
//!
//! [`check`] parses a module with a pinned third-party Python parser, lowers it
//! into the crate-owned [`ast`] (rejecting everything outside the allow-list),
//! validates the module interface and counts AST nodes for the complexity quota.
//! It never panics and never recurses unboundedly, whatever the input.

pub mod ast;
mod lower;

use rustpython_parser::{Mode, Tok, lexer, parse};

/// Hard cap on AST nodes per module (latticepy §6 `max_code_nodes`). A
/// structural sandbox limit, not a tunable; a player's smaller quota is
/// enforced by the caller against [`CheckReport::node_count`].
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

/// One problem found in a module, written for an LLM reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1-based line.
    pub line: u32,
    /// 1-based column in characters.
    pub col: u32,
    /// The offending construct as the player wrote or would recognise it.
    pub construct: String,
    /// Full message: the construct, why it is banned, and what to do instead.
    pub message: String,
}

/// Result of checking one module.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckReport {
    /// True when the module can be deployed (no diagnostics).
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// Statements plus expressions in the module: the complexity-quota unit.
    pub node_count: usize,
    /// The allow-listed tree, present only when `ok`.
    pub module: Option<ast::Module>,
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

/// Check LatticePy source. Cost: linear in source size, bounded by
/// [`MAX_SOURCE_BYTES`].
pub fn check(source: &str) -> CheckReport {
    let mut lower = lower::Lower::new(source);
    let reject = |diag: Diagnostic| fail(vec![diag]);
    if let Some(i) = source.find('\0') {
        let pos = lower.pos(i.try_into().unwrap_or_default());
        return reject(diag(
            pos,
            "NUL byte",
            "the source contains a NUL character. Remove it.",
        ));
    }
    if source.len() > MAX_SOURCE_BYTES {
        let pos = ast::Pos { line: 1, col: 1 };
        let msg = format!(
            "the source is {} bytes; the limit is {MAX_SOURCE_BYTES}. Split it into smaller modules.",
            source.len()
        );
        return reject(diag(pos, "module size", &msg));
    }
    if let Some(d) = prescan(source, &lower) {
        return reject(d);
    }
    let parsed = match parse(source, Mode::Module, "<lattice>") {
        Ok(m) => m,
        Err(e) => {
            let pos = lower.pos(e.offset);
            return reject(diag(pos, "syntax", &format!("syntax error: {}", e.error)));
        }
    };
    let rustpython_parser::ast::Mod::Module(m) = parsed else {
        return reject(diag(
            ast::Pos { line: 1, col: 1 },
            "module",
            "expected a module",
        ));
    };
    let module = lower.module(&m.body);
    let mut diagnostics = std::mem::take(&mut lower.diags);
    interface(&module, &mut diagnostics);
    let node_count = lower.nodes;
    if node_count > MAX_CODE_NODES {
        let pos = ast::Pos { line: 1, col: 1 };
        let msg = format!(
            "the module has {node_count} AST nodes; the hard limit is {MAX_CODE_NODES}. Remove or simplify code."
        );
        diagnostics.push(diag(pos, "module size", &msg));
    }
    diagnostics.sort_by_key(|d| (d.line, d.col));
    let ok = diagnostics.is_empty();
    CheckReport {
        ok,
        diagnostics,
        node_count,
        module: ok.then_some(module),
    }
}

/// [`check`] for raw bytes, rejecting invalid UTF-8 with a diagnostic.
pub fn check_bytes(source: &[u8]) -> CheckReport {
    match std::str::from_utf8(source) {
        Ok(s) => check(s),
        Err(e) => {
            let line = source[..e.valid_up_to()]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1;
            fail(vec![diag(
                ast::Pos { line, col: 1 },
                "encoding",
                "the source is not valid UTF-8. Save it as UTF-8 text.",
            )])
        }
    }
}

fn diag(pos: ast::Pos, construct: &str, message: &str) -> Diagnostic {
    Diagnostic {
        line: pos.line,
        col: pos.col,
        construct: construct.to_string(),
        message: message.to_string(),
    }
}

fn fail(diagnostics: Vec<Diagnostic>) -> CheckReport {
    CheckReport {
        ok: false,
        diagnostics,
        node_count: 0,
        module: None,
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
            return Some(diag(
                lower.pos(range.start()),
                "deep nesting",
                "brackets or blocks are nested too deeply. Split the code into helper functions.",
            ));
        }
        if tokens > MAX_TOKENS {
            return Some(diag(
                lower.pos(range.start()),
                "module size",
                "the module has too many tokens. Remove or simplify code.",
            ));
        }
    }
    None
}

/// Check `init`/`tick` exist and entry points use the exact parameter names.
fn interface(module: &ast::Module, out: &mut Vec<Diagnostic>) {
    for (name, params, required) in INTERFACE {
        let found = module.body.iter().find_map(|s| match &s.kind {
            ast::StmtKind::FunctionDef {
                name: n, params: p, ..
            } if n == name => Some((s.pos, p)),
            _ => None,
        });
        let want = format!("def {name}({})", params.join(", "));
        match found {
            None if required => out.push(diag(
                ast::Pos { line: 1, col: 1 },
                name,
                &format!("the module must define `{want}`; the engine calls it."),
            )),
            None => {}
            Some((pos, p))
                if !p.iter().map(|x| x.name.as_str()).eq(params.iter().copied())
                    || p.iter().any(|x| x.default.is_some()) =>
            {
                out.push(diag(pos, name, &format!("`{name}` must be declared exactly as `{want}` (no defaults, these parameter names).")));
            }
            Some(_) => {}
        }
    }
}
