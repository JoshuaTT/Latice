//! Lowering from the third-party parser's AST to the crate-owned one
//! (`ast.rs`), rejecting everything outside latticepy §1 with diagnostics an LLM
//! can act on. Every rejection says what was written, why it is banned, and what
//! to do instead; all diagnostics are collected, not just the first.

use super::ast::{self as own, Pos};
use super::{Diagnostic, MAX_EXPR_DEPTH, Severity};
use rustpython_parser::ast::{self, Ranged};
use rustpython_parser::text_size::TextSize;
use std::collections::BTreeSet;

pub(super) struct Lower<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
    pub diags: Vec<Diagnostic>,
    /// Statements and expressions lowered so far: the complexity-quota unit
    /// (latticepy §5/§6), the same one fuel is metered in.
    pub nodes: usize,
    depth: usize,
    loops: usize,
    /// Module-level functions, constants and explicitly imported names.
    globals: BTreeSet<String>,
    /// Names bound so far in the current function: parameters, assignment,
    /// loop and comprehension targets.
    locals: BTreeSet<String>,
}

type E = own::Expr;

impl<'a> Lower<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Self {
            src,
            line_starts,
            diags: Vec::new(),
            nodes: 0,
            depth: 0,
            loops: 0,
            globals: BTreeSet::new(),
            locals: BTreeSet::new(),
        }
    }

    pub fn pos(&self, at: TextSize) -> Pos {
        let off = usize::from(at).min(self.src.len());
        let line = self.line_starts.partition_point(|&s| s <= off) - 1;
        let start = self.line_starts[line];
        let col = self
            .src
            .get(start..off)
            .map_or(off - start, |s| s.chars().count());
        Pos {
            line: line as u32 + 1,
            col: col as u32 + 1,
        }
    }

    /// Report a banned construct. The stable code comes from [`code_of`].
    pub fn bad(&mut self, at: TextSize, construct: &str, why: &str) {
        let message = format!("`{construct}` is not allowed in LatticePy: {why}");
        self.error(at, code_of(construct), message);
    }

    pub fn error(&mut self, at: TextSize, code: &'static str, message: String) {
        let Pos { line, col } = self.pos(at);
        self.diags.push(Diagnostic {
            severity: Severity::Error,
            code,
            line,
            col,
            message,
        });
    }

    fn placeholder(&self, at: TextSize) -> E {
        E {
            pos: self.pos(at),
            kind: own::ExprKind::None,
        }
    }

    pub fn module(&mut self, body: &[ast::Stmt]) -> own::Module {
        self.collect_globals(body);
        let mut out = Vec::new();
        let mut imported = false;
        for s in body {
            let at = s.range().start();
            match s {
                ast::Stmt::ImportFrom(_) if imported => {
                    self.bad(at, "from lattice import", "only one import statement is allowed. Merge the names into the first one.");
                }
                ast::Stmt::ImportFrom(_) => {
                    imported = true;
                    out.extend(self.stmt(s));
                }
                // `import x` at module level is an import error ("no such
                // module"), not a misplaced statement: same as inside a `def`.
                ast::Stmt::Import(_) => out.extend(self.stmt(s)),
                ast::Stmt::FunctionDef(_) | ast::Stmt::AsyncFunctionDef(_) => out.extend(self.stmt(s)),
                ast::Stmt::Assign(_) => {
                    if let Some(st) = self.stmt(s) {
                        if let own::StmtKind::Assign { value, .. } = &st.kind
                            && !is_literal(value)
                        {
                            self.bad(at, "top-level assignment", "module-level constants must be literals (numbers, strings, True/False/None, or lists/tuples/dicts of them). Move computation into a function.");
                        }
                        out.push(st);
                    }
                }
                _ => self.bad(at, "top-level statement", "only the `lattice` import, function definitions and literal constants may appear at module level. Put behavior inside `init`, `tick` or a helper function."),
            }
        }
        own::Module { body: out }
    }

    /// Module-level names, gathered first so functions may refer to each other
    /// and to constants regardless of order.
    fn collect_globals(&mut self, body: &[ast::Stmt]) {
        fn names(e: &ast::Expr, out: &mut Vec<String>) {
            match e {
                ast::Expr::Name(n) => out.push(n.id.to_string()),
                ast::Expr::Tuple(t) => t.elts.iter().for_each(|x| names(x, out)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for s in body {
            match s {
                ast::Stmt::FunctionDef(f) => found.push(f.name.to_string()),
                ast::Stmt::Assign(a) => a.targets.iter().for_each(|t| names(t, &mut found)),
                ast::Stmt::ImportFrom(i) => {
                    found.extend(i.names.iter().map(|a| a.name.to_string()))
                }
                _ => {}
            }
        }
        self.globals.extend(found.into_iter().filter(|n| n != "*"));
    }

    fn block(&mut self, body: &[ast::Stmt]) -> Vec<own::Stmt> {
        body.iter().filter_map(|s| self.stmt(s)).collect()
    }

    fn stmt(&mut self, s: &ast::Stmt) -> Option<own::Stmt> {
        let at = s.range().start();
        self.nodes += 1;
        self.depth += 1;
        let kind = if self.depth > MAX_EXPR_DEPTH {
            self.bad(
                at,
                "deep nesting",
                "blocks are nested too deeply. Flatten the code with helper functions.",
            );
            None
        } else {
            self.stmt_kind(s, at)
        };
        self.depth -= 1;
        Some(own::Stmt {
            pos: self.pos(at),
            kind: kind?,
        })
    }

    fn stmt_kind(&mut self, s: &ast::Stmt, at: TextSize) -> Option<own::StmtKind> {
        use own::StmtKind as K;
        Some(match s {
            ast::Stmt::ImportFrom(i) => {
                let level_ok = i.level.as_ref().is_none_or(|l| l.to_u32() == 0);
                if i.module.as_ref().map(|m| m.as_str()) != Some("lattice") || !level_ok {
                    self.bad(at, "import", "only `from lattice import ...` is available; there are no other modules.");
                    return None;
                }
                let mut names = Vec::new();
                for a in &i.names {
                    let name = a.name.as_str();
                    if let Some(alias) = &a.asname {
                        self.error(
                            a.range().start(),
                            "E_IMPORT_ALIAS",
                            format!("`{name} as {alias}` is not allowed in LatticePy: import the name directly, `from lattice import {name}`."),
                        );
                    }
                    if name != "*" {
                        self.import_name(name, a.range().start());
                        names.push(name.to_string());
                    }
                }
                K::ImportLattice(names)
            }
            ast::Stmt::Import(_) => {
                self.bad(at, "import", "only `from lattice import ...` is available; there are no other modules.");
                return None;
            }
            ast::Stmt::FunctionDef(f) => {
                if !f.decorator_list.is_empty() {
                    self.bad(f.decorator_list[0].range().start(), "decorator", "functions cannot be decorated. Remove the `@...` line.");
                }
                if !f.type_params.is_empty() {
                    self.bad(at, "type parameters", "generic functions are not supported. Remove the `[T]` list.");
                }
                let params = self.params(&f.args, at);
                let outer_loops = std::mem::replace(&mut self.loops, 0);
                let outer_locals = std::mem::take(&mut self.locals);
                self.locals.extend(params.iter().map(|p| p.name.clone()));
                let body = self.block(&f.body);
                self.locals = outer_locals;
                self.loops = outer_loops;
                K::FunctionDef { name: f.name.to_string(), params, body }
            }
            ast::Stmt::Return(r) => K::Return(r.value.as_ref().map(|v| self.expr(v))),
            ast::Stmt::Assign(a) => {
                if a.targets.len() != 1 {
                    self.bad(at, "chained assignment `a = b = ...`", "assign each name on its own line.");
                }
                let value = self.expr(&a.value);
                let target = a.targets.first().map(|t| self.target(t, true, true)).unwrap_or_else(|| self.placeholder(at));
                K::Assign { target, value }
            }
            ast::Stmt::AugAssign(a) => {
                let op = match bin_op(&a.op) {
                    Some(op @ (own::BinOp::Add | own::BinOp::Sub | own::BinOp::Mul | own::BinOp::FloorDiv | own::BinOp::Mod)) => op,
                    _ => {
                        self.bad(at, "augmented assignment operator", "only `+=`, `-=`, `*=`, `//=` and `%=` are allowed. Write `x = x <op> y` instead.");
                        own::BinOp::Add
                    }
                };
                // The target is read as well as written, so it must already exist.
                let target = self.target(&a.target, true, false);
                K::AugAssign { target, op, value: self.expr(&a.value) }
            }
            ast::Stmt::AnnAssign(a) => {
                let annotation = match &*a.annotation {
                    ast::Expr::Name(n) => n.id.to_string(),
                    other => {
                        self.bad(other.range().start(), "annotation", "annotations must be a plain name like `int`.");
                        String::new()
                    }
                };
                let value = a.value.as_ref().map(|v| self.expr(v));
                K::AnnAssign { target: self.target(&a.target, false, true), annotation, value }
            }
            ast::Stmt::If(i) => K::If { test: self.expr(&i.test), body: self.block(&i.body), orelse: self.block(&i.orelse) },
            ast::Stmt::For(f) => {
                if !f.orelse.is_empty() {
                    self.bad(f.orelse[0].range().start(), "for ... else", "loop `else` blocks are not supported. Use a flag variable.");
                }
                let iter = self.expr(&f.iter);
                let target = self.target(&f.target, false, true);
                self.loops += 1;
                let body = self.block(&f.body);
                self.loops -= 1;
                K::For { target, iter, body }
            }
            ast::Stmt::While(w) => {
                if !w.orelse.is_empty() {
                    self.bad(w.orelse[0].range().start(), "while ... else", "loop `else` blocks are not supported. Use a flag variable.");
                }
                let test = self.expr(&w.test);
                self.loops += 1;
                let body = self.block(&w.body);
                self.loops -= 1;
                K::While { test, body }
            }
            ast::Stmt::Break(_) | ast::Stmt::Continue(_) if self.loops == 0 => {
                self.bad(at, "break/continue outside a loop", "`break` and `continue` only work inside `for` or `while`.");
                return None;
            }
            ast::Stmt::Break(_) => K::Break,
            ast::Stmt::Continue(_) => K::Continue,
            ast::Stmt::Pass(_) => K::Pass,
            ast::Stmt::Expr(e) => K::Expr(self.expr(&e.value)),
            ast::Stmt::Try(_) | ast::Stmt::TryStar(_) => {
                self.bad(at, "try", "runtime errors stop the drone for this tick (§8). Check the value before using it.");
                return None;
            }
            ast::Stmt::Raise(_) => return self.ban(at, "raise", "there are no exceptions; a runtime error already stops the drone for this tick. Return early instead."),
            ast::Stmt::With(_) => return self.ban(at, "with", "there are no context managers or I/O. Remove the `with` block."),
            ast::Stmt::Assert(_) => return self.ban(at, "assert", "its behavior depends on the host. Use an `if` and return early."),
            ast::Stmt::Global(_) => return self.ban(at, "global", "functions cannot rebind module names. Store state in `me.mem`."),
            ast::Stmt::Nonlocal(_) => return self.ban(at, "nonlocal", "there are no closures. Store state in `me.mem`."),
            ast::Stmt::Delete(_) => return self.ban(at, "del", "deletion is not supported. Assign a new value instead."),
            ast::Stmt::ClassDef(_) => return self.ban(at, "class", "user-defined types do not exist. Use dicts, tuples and functions."),
            ast::Stmt::AsyncFunctionDef(_) | ast::Stmt::AsyncFor(_) | ast::Stmt::AsyncWith(_) => {
                return self.ban(at, "async", "there is no async. Write a plain `def`/`for`/`with`-free function.");
            }
            ast::Stmt::Match(_) => return self.ban(at, "match", "pattern matching is not supported. Use `if`/`elif`."),
            ast::Stmt::TypeAlias(_) => return self.ban(at, "type alias", "type aliases are not supported. Remove the `type` statement."),
        })
    }

    fn ban(&mut self, at: TextSize, construct: &str, why: &str) -> Option<own::StmtKind> {
        self.bad(at, construct, why);
        None
    }

    fn params(&mut self, a: &ast::Arguments, at: TextSize) -> Vec<own::Param> {
        if !a.posonlyargs.is_empty() {
            self.bad(
                at,
                "positional-only parameters `/`",
                "use ordinary parameters.",
            );
        }
        if !a.kwonlyargs.is_empty() {
            self.bad(at, "keyword-only parameters", "use ordinary parameters.");
        }
        if a.vararg.is_some() {
            self.bad(
                at,
                "*args",
                "functions take a fixed parameter list. Pass a list or tuple instead.",
            );
        }
        if a.kwarg.is_some() {
            self.bad(
                at,
                "**kwargs",
                "functions take a fixed parameter list. Pass a dict instead.",
            );
        }
        a.args
            .iter()
            .map(|p| {
                let default = p.default.as_ref().map(|d| self.expr(d));
                if let Some(d) = &default
                    && !is_literal(d)
                {
                    self.bad(
                        d_start(p),
                        "non-literal default",
                        "parameter defaults must be literals.",
                    );
                }
                own::Param {
                    name: p.def.arg.to_string(),
                    default,
                }
            })
            .collect()
    }

    /// An assignment / loop target: a name, a tuple of targets, or (when
    /// `subscripts`) an index like `me.mem["k"]`.
    /// With `declare`, a name target is bound for later reads; without it the
    /// name must already exist (augmented assignment).
    fn target(&mut self, t: &ast::Expr, subscripts: bool, declare: bool) -> E {
        let at = t.range().start();
        match t {
            ast::Expr::Name(n) if declare => {
                self.nodes += 1;
                if n.id.as_str().starts_with("__") {
                    self.name(n.id.as_str(), at);
                }
                self.locals.insert(n.id.to_string());
                E {
                    pos: self.pos(at),
                    kind: own::ExprKind::Name(n.id.to_string()),
                }
            }
            ast::Expr::Name(_) => self.expr(t),
            ast::Expr::Tuple(tu) => {
                self.nodes += 1;
                let elts = tu
                    .elts
                    .iter()
                    .map(|e| self.target(e, subscripts, declare))
                    .collect();
                E {
                    pos: self.pos(at),
                    kind: own::ExprKind::Tuple(elts),
                }
            }
            ast::Expr::Subscript(_) if subscripts => self.expr(t),
            ast::Expr::Attribute(_) => {
                self.bad(at, "attribute assignment", "only names and `me.mem[...]` can be assigned to. Use a library call or `me.mem[\"key\"] = value`.");
                self.placeholder(at)
            }
            _ => {
                self.bad(
                    at,
                    "assignment target",
                    "assign to a name, a tuple of names, or `me.mem[...]`.",
                );
                self.placeholder(at)
            }
        }
    }

    fn expr(&mut self, e: &ast::Expr) -> E {
        let at = e.range().start();
        self.nodes += 1;
        self.depth += 1;
        let kind = if self.depth > MAX_EXPR_DEPTH {
            self.bad(
                at,
                "deep nesting",
                "this expression is nested too deeply. Split it into several statements.",
            );
            own::ExprKind::None
        } else {
            self.expr_kind(e, at)
        };
        self.depth -= 1;
        E {
            pos: self.pos(at),
            kind,
        }
    }

    fn boxed(&mut self, e: &ast::Expr) -> Box<E> {
        Box::new(self.expr(e))
    }

    fn expr_kind(&mut self, e: &ast::Expr, at: TextSize) -> own::ExprKind {
        use own::ExprKind as K;
        match e {
            ast::Expr::Constant(c) => self.constant(&c.value, at),
            ast::Expr::Name(n) => {
                self.name(n.id.as_str(), at);
                K::Name(n.id.to_string())
            }
            ast::Expr::List(l) => K::List(l.elts.iter().map(|x| self.expr(x)).collect()),
            ast::Expr::Tuple(t) => K::Tuple(t.elts.iter().map(|x| self.expr(x)).collect()),
            ast::Expr::Dict(d) => {
                let mut items = Vec::new();
                for (k, v) in d.keys.iter().zip(&d.values) {
                    match k {
                        Some(k) => items.push((self.expr(k), self.expr(v))),
                        None => self.bad(
                            v.range().start(),
                            "**mapping in a dict",
                            "write the keys out explicitly.",
                        ),
                    }
                }
                K::Dict(items)
            }
            ast::Expr::BoolOp(b) => K::BoolOp {
                is_and: matches!(b.op, ast::BoolOp::And),
                values: b.values.iter().map(|x| self.expr(x)).collect(),
            },
            ast::Expr::BinOp(b) => {
                let op = bin_op(&b.op).unwrap_or_else(|| {
                    let (c, why) = match b.op {
                        ast::Operator::Div => ("/", "true division would produce a host float. Use `//` for integer division, or the `lattice` software float `F.div` for fractions."),
                        ast::Operator::Pow => ("**", "exponentiation is not available. Multiply in a loop."),
                        _ => ("@", "matrix multiplication is not available."),
                    };
                    self.bad(at, c, why);
                    own::BinOp::Add
                });
                K::BinOp {
                    left: self.boxed(&b.left),
                    op,
                    right: self.boxed(&b.right),
                }
            }
            ast::Expr::UnaryOp(u) => K::Unary {
                op: match u.op {
                    ast::UnaryOp::UAdd => own::UnaryOp::Pos,
                    ast::UnaryOp::USub => own::UnaryOp::Neg,
                    ast::UnaryOp::Not => own::UnaryOp::Not,
                    ast::UnaryOp::Invert => own::UnaryOp::Invert,
                },
                operand: self.boxed(&u.operand),
            },
            ast::Expr::Compare(c) => K::Compare {
                left: self.boxed(&c.left),
                rest: c
                    .ops
                    .iter()
                    .zip(&c.comparators)
                    .map(|(o, x)| (cmp_op(o), self.expr(x)))
                    .collect(),
            },
            ast::Expr::Call(c) => self.call(c, at),
            ast::Expr::IfExp(i) => K::IfExp {
                test: self.boxed(&i.test),
                body: self.boxed(&i.body),
                orelse: self.boxed(&i.orelse),
            },
            ast::Expr::Subscript(s) => self.subscript(s),
            ast::Expr::Attribute(a) => {
                if a.attr.as_str().starts_with("__") {
                    self.error(at, "E_DUNDER", format!("`{}` is not allowed in LatticePy: dunder attributes are reflection and are not available.", a.attr));
                }
                K::Attr {
                    value: self.boxed(&a.value),
                    attr: a.attr.to_string(),
                }
            }
            ast::Expr::ListComp(l) => self.list_comp(l, at),
            ast::Expr::Lambda(_) => {
                self.ban_expr(at, "lambda", "define a named function with `def` instead.")
            }
            ast::Expr::NamedExpr(_) => self.ban_expr(at, ":=", "assign on its own line first."),
            ast::Expr::Set(_) | ast::Expr::SetComp(_) => self.ban_expr(
                at,
                "set",
                "sets have no deterministic order and are banned. Use a list or a dict.",
            ),
            ast::Expr::DictComp(_) => self.ban_expr(
                at,
                "dict comprehension",
                "build the dict with a `for` loop.",
            ),
            ast::Expr::GeneratorExp(_) => self.ban_expr(
                at,
                "generator expression",
                "use a list comprehension `[... for ... in ...]` or a loop.",
            ),
            ast::Expr::Await(_) => self.ban_expr(at, "await", "there is no async."),
            ast::Expr::Yield(_) | ast::Expr::YieldFrom(_) => {
                self.ban_expr(at, "yield", "generators are not supported. Return a list.")
            }
            ast::Expr::JoinedStr(_) | ast::Expr::FormattedValue(_) => {
                self.ban_expr(at, "f-string", "build strings with `+` and `str(...)`.")
            }
            ast::Expr::Starred(_) => self.ban_expr(
                at,
                "*",
                "unpacking with `*` is not supported. Index or loop instead.",
            ),
            ast::Expr::Slice(_) => self.ban_expr(
                at,
                "slice",
                "slices are only allowed directly inside `[...]`, e.g. `xs[1:3]`.",
            ),
        }
    }

    fn ban_expr(&mut self, at: TextSize, construct: &str, why: &str) -> own::ExprKind {
        self.bad(at, construct, why);
        own::ExprKind::None
    }

    fn constant(&mut self, c: &ast::Constant, at: TextSize) -> own::ExprKind {
        use own::ExprKind as K;
        match c {
            ast::Constant::None => K::None,
            ast::Constant::Bool(b) => K::Bool(*b),
            ast::Constant::Str(s) => K::Str(s.clone()),
            ast::Constant::Int(i) => match i64::try_from(i) {
                Ok(v) => K::Int(v),
                Err(_) => self.ban_expr(at, "integer literal", "integers are 64-bit signed (-2**63 .. 2**63-1)."),
            },
            ast::Constant::Float(_) | ast::Constant::Complex { .. } => {
                self.ban_expr(at, "float literal", "host floats are banned. Use integers, or the `lattice` software float (`F.from_int`).")
            }
            _ => self.ban_expr(at, "literal", "only int, bool, str and None literals exist."),
        }
    }

    /// Validate a name in `from lattice import ...`. Banned and dunder names
    /// are refused (so the import list cannot smuggle them past the name
    /// allow-list); once `lattice` has a defined export table (#38) any name
    /// outside it is refused too.
    fn import_name(&mut self, id: &str, at: TextSize) {
        if let Some(why) = banned_why(id) {
            let code = if id.starts_with("__") {
                "E_DUNDER"
            } else {
                "E_BANNED_NAME"
            };
            self.error(
                at,
                code,
                format!("`{id}` is not allowed in LatticePy: {why}"),
            );
        } else if UNPROVIDED_BUILTINS.contains(&id) {
            self.error(
                at,
                "E_UNKNOWN_NAME",
                format!("`{id}` is a Python builtin that `lattice` does not provide."),
            );
        } else if !LATTICE_EXPORTS.is_empty() && !LATTICE_EXPORTS.contains(&id) {
            self.error(
                at,
                "E_IMPORT",
                format!("`lattice` has no export named `{id}`."),
            );
        }
    }

    /// Resolve a free name load. Names are denied unless they are in scope or on
    /// the allow-list (latticepy §2); the deny-list only picks friendlier text.
    fn name(&mut self, id: &str, at: TextSize) {
        if self.locals.contains(id)
            || self.globals.contains(id)
            || ALLOWED_GLOBALS.contains(&id)
            || LATTICE_EXPORTS.contains(&id)
        {
            return;
        }
        let banned = banned_why(id);
        if let Some(why) = banned {
            let code = if id.starts_with("__") {
                "E_DUNDER"
            } else {
                "E_BANNED_NAME"
            };
            self.error(
                at,
                code,
                format!("`{id}` is not allowed in LatticePy: {why}"),
            );
            return;
        }
        let hint = self
            .locals
            .iter()
            .map(String::as_str)
            .chain(self.globals.iter().map(String::as_str))
            .chain(ALLOWED_GLOBALS.iter().copied())
            .chain(LATTICE_EXPORTS.iter().copied())
            .map(|c| (edit_distance(id, c), c))
            .filter(|&(d, c)| d <= 2 && d * 2 <= c.len().max(id.len()))
            .min()
            .map(|(_, c)| format!(" Did you mean `{c}`?"))
            .unwrap_or_default();
        self.error(
            at,
            "E_UNKNOWN_NAME",
            format!("`{id}` is not defined. Only module functions and constants, local variables, `me`, `team`, `seen`, `range`, `str` and names imported from `lattice` can be used.{hint}"),
        );
    }

    fn call(&mut self, c: &ast::ExprCall, at: TextSize) -> own::ExprKind {
        if !matches!(&*c.func, ast::Expr::Name(_) | ast::Expr::Attribute(_)) {
            self.bad(
                at,
                "call of a computed value",
                "call a function by name, e.g. `helper(x)` or `me.move(...)`.",
            );
        }
        let mut kwargs = Vec::new();
        for k in &c.keywords {
            match &k.arg {
                Some(name) => kwargs.push((name.to_string(), self.expr(&k.value))),
                None => self.bad(
                    k.value.range().start(),
                    "**kwargs in a call",
                    "pass each keyword explicitly.",
                ),
            }
        }
        own::ExprKind::Call {
            func: self.boxed(&c.func),
            args: c.args.iter().map(|a| self.expr(a)).collect(),
            kwargs,
        }
    }

    fn subscript(&mut self, s: &ast::ExprSubscript) -> own::ExprKind {
        let value = self.boxed(&s.value);
        let ast::Expr::Slice(sl) = &*s.slice else {
            return own::ExprKind::Index {
                value,
                index: self.boxed(&s.slice),
            };
        };
        self.nodes += 1;
        let lo = sl.lower.as_ref().map(|x| self.boxed(x));
        let hi = sl.upper.as_ref().map(|x| self.boxed(x));
        let step = match sl.step.as_ref().map(|x| self.expr(x)).map(|x| x.kind) {
            None => 1,
            Some(own::ExprKind::Int(1)) => 1,
            Some(own::ExprKind::Int(-1)) => -1,
            Some(own::ExprKind::Unary {
                op: own::UnaryOp::Neg,
                operand,
            }) if operand.kind == own::ExprKind::Int(1) => -1,
            Some(_) => {
                self.bad(
                    sl.range().start(),
                    "slice step",
                    "the step must be the literal `1` or `-1`.",
                );
                1
            }
        };
        own::ExprKind::Slice {
            value,
            lo,
            hi,
            step,
        }
    }

    fn list_comp(&mut self, l: &ast::ExprListComp, at: TextSize) -> own::ExprKind {
        let [g] = l.generators.as_slice() else {
            self.bad(
                at,
                "nested comprehension",
                "a list comprehension may have only one `for`. Use a loop for more.",
            );
            return own::ExprKind::None;
        };
        if g.is_async {
            self.bad(at, "async comprehension", "there is no async.");
        }
        if g.ifs.len() > 1 {
            self.bad(
                at,
                "comprehension filters",
                "a list comprehension may have at most one `if`. Join conditions with `and`.",
            );
        }
        let iter = self.boxed(&g.iter);
        let target = Box::new(self.target(&g.target, false, true));
        let cond = g.ifs.first().map(|x| self.boxed(x));
        let elt = self.boxed(&l.elt);
        own::ExprKind::ListComp {
            elt,
            target,
            iter,
            cond,
        }
    }
}

fn d_start(p: &ast::ArgWithDefault) -> TextSize {
    p.def.range().start()
}

fn bin_op(op: &ast::Operator) -> Option<own::BinOp> {
    use own::BinOp as B;
    Some(match op {
        ast::Operator::Add => B::Add,
        ast::Operator::Sub => B::Sub,
        ast::Operator::Mult => B::Mul,
        ast::Operator::FloorDiv => B::FloorDiv,
        ast::Operator::Mod => B::Mod,
        ast::Operator::LShift => B::LShift,
        ast::Operator::RShift => B::RShift,
        ast::Operator::BitAnd => B::BitAnd,
        ast::Operator::BitOr => B::BitOr,
        ast::Operator::BitXor => B::BitXor,
        ast::Operator::Div | ast::Operator::Pow | ast::Operator::MatMult => return None,
    })
}

fn cmp_op(op: &ast::CmpOp) -> own::CmpOp {
    use own::CmpOp as C;
    match op {
        ast::CmpOp::Eq => C::Eq,
        ast::CmpOp::NotEq => C::NotEq,
        ast::CmpOp::Lt => C::Lt,
        ast::CmpOp::LtE => C::LtE,
        ast::CmpOp::Gt => C::Gt,
        ast::CmpOp::GtE => C::GtE,
        ast::CmpOp::Is => C::Is,
        ast::CmpOp::IsNot => C::IsNot,
        ast::CmpOp::In => C::In,
        ast::CmpOp::NotIn => C::NotIn,
    }
}

fn is_literal(e: &E) -> bool {
    use own::ExprKind as K;
    match &e.kind {
        K::Int(_) | K::Bool(_) | K::Str(_) | K::None => true,
        K::Unary {
            op: own::UnaryOp::Neg,
            operand,
        } => matches!(operand.kind, K::Int(_)),
        K::List(xs) | K::Tuple(xs) => xs.iter().all(is_literal),
        K::Dict(kv) => kv.iter().all(|(k, v)| is_literal(k) && is_literal(v)),
        _ => false,
    }
}

/// Names every free `Name` load may resolve to besides module and local names
/// (latticepy §1.2 `For` over `range`, §1.3 `str(...)`, the `me`/`team`/`seen`
/// handles of §1.3 `Attribute`). Builtins the spec does not name wait for the
/// `lattice` library API (#38).
const ALLOWED_GLOBALS: [&str; 5] = ["range", "str", "me", "team", "seen"];

/// Names exported by `lattice` itself. Empty until the library API (#38)
/// defines them; until then a module names what it uses with
/// `from lattice import a, b`, which is the only way to bring more in. While
/// this table is empty imports are checked only for banned and dunder names;
/// TODO(#38): fill it in, after which unknown imports are rejected.
const LATTICE_EXPORTS: &[&str] = &[];

/// Stable code for each banned construct (the table in `checker/mod.rs`).
fn code_of(construct: &str) -> &'static str {
    match construct {
        "import" | "from lattice import" => "E_IMPORT",
        "top-level assignment" | "top-level statement" => "E_TOPLEVEL",
        "deep nesting" => "E_NESTING",
        "decorator"
        | "type parameters"
        | "positional-only parameters `/`"
        | "keyword-only parameters"
        | "*args"
        | "**kwargs"
        | "non-literal default" => "E_PARAMS",
        "chained assignment `a = b = ...`" | "attribute assignment" | "assignment target" => {
            "E_ASSIGN_TARGET"
        }
        "augmented assignment operator" | "/" | "**" | "@" => "E_OPERATOR",
        "annotation" => "E_ANNOTATION",
        "for ... else" | "while ... else" => "E_LOOP_ELSE",
        "break/continue outside a loop" => "E_LOOP_CONTROL",
        "try" | "raise" | "with" | "assert" | "global" | "nonlocal" | "del" | "class" | "async"
        | "match" | "type alias" => "E_STMT_BANNED",
        "nested comprehension" | "comprehension filters" | "async comprehension" => {
            "E_COMPREHENSION"
        }
        "call of a computed value" | "**kwargs in a call" => "E_CALL",
        "slice step" => "E_SLICE",
        "integer literal" => "E_INT_RANGE",
        "float literal" => "E_FLOAT",
        "literal" => "E_LITERAL",
        _ => "E_EXPR_BANNED",
    }
}

/// Levenshtein distance, for "did you mean" hints on unknown names.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = std::mem::replace(&mut row[0], i + 1);
        for (j, &cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != cb)).min(cur + 1).min(row[j] + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

/// Friendly reason for names that are known to be banned (latticepy §2).
fn banned_why(id: &str) -> Option<&'static str> {
    match id {
        "open" | "print" | "input" => {
            Some("there is no I/O. Use `me.log(...)` to record information.")
        }
        "os" | "sys" | "io" | "time" | "datetime" | "threading" | "asyncio" | "random" => Some(
            "there are no modules. `me.now` is the tick counter and `me.rand()` is the seeded random source.",
        ),
        "eval" | "exec" | "compile" | "__import__" | "getattr" | "setattr" | "hasattr"
        | "delattr" | "vars" | "dir" | "type" | "isinstance" | "globals" | "locals" | "hash" => {
            Some("reflection and dynamic code are not available.")
        }
        "set" | "frozenset" => {
            Some("sets have no deterministic order and are banned. Use a list or a dict.")
        }
        "float" => Some("host floats are banned. Use the `lattice` software float."),
        _ if id.starts_with("__") => Some("dunder names are reflection and are not available."),
        _ => None,
    }
}

/// Python builtins the spec does not grant and `lattice` must never export;
/// refused in import lists even while `LATTICE_EXPORTS` is still empty.
const UNPROVIDED_BUILTINS: [&str; 10] = [
    "id",
    "object",
    "super",
    "breakpoint",
    "iter",
    "next",
    "memoryview",
    "issubclass",
    "exit",
    "quit",
];
