//! The crate-owned LatticePy AST.
//!
//! The checker lowers the third-party parser's tree into this one and the VM
//! compiles from this one only, so the language allow-list (latticepy §1) is
//! enforced by construction: a construct with no variant here cannot reach the VM.

/// 1-based line and column (counted in characters) of a node's first token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    /// Literal default (latticepy §1.4); never an arbitrary expression.
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub pos: Pos,
    pub kind: StmtKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// `from lattice import ...`; an empty list means `import *`.
    ImportLattice(Vec<String>),
    FunctionDef {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Assign {
        target: Expr,
        value: Expr,
    },
    AugAssign {
        target: Expr,
        op: BinOp,
        value: Expr,
    },
    AnnAssign {
        target: Expr,
        annotation: String,
        value: Option<Expr>,
    },
    If {
        test: Expr,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    For {
        target: Expr,
        iter: Expr,
        body: Vec<Stmt>,
    },
    While {
        test: Expr,
        body: Vec<Stmt>,
    },
    Break,
    Continue,
    Pass,
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    FloorDiv,
    Mod,
    LShift,
    RShift,
    BitAnd,
    BitOr,
    BitXor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Pos,
    Neg,
    Not,
    Invert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    Is,
    IsNot,
    In,
    NotIn,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub pos: Pos,
    pub kind: ExprKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// Integers are 64-bit signed (latticepy §3); wider literals are rejected.
    Int(i64),
    Bool(bool),
    Str(String),
    None,
    Name(String),
    List(Vec<Expr>),
    Tuple(Vec<Expr>),
    Dict(Vec<(Expr, Expr)>),
    /// `and` (`is_and`) / `or` over two or more operands.
    BoolOp {
        is_and: bool,
        values: Vec<Expr>,
    },
    BinOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    /// Chained comparisons stay chained; the VM desugars them.
    Compare {
        left: Box<Expr>,
        rest: Vec<(CmpOp, Expr)>,
    },
    Call {
        func: Box<Expr>,
        args: Vec<Expr>,
        kwargs: Vec<(String, Expr)>,
    },
    IfExp {
        test: Box<Expr>,
        body: Box<Expr>,
        orelse: Box<Expr>,
    },
    Index {
        value: Box<Expr>,
        index: Box<Expr>,
    },
    /// `value[lo:hi:step]`; `step` is only ever `1` or `-1` (latticepy §1.3).
    Slice {
        value: Box<Expr>,
        lo: Option<Box<Expr>>,
        hi: Option<Box<Expr>>,
        step: i8,
    },
    Attr {
        value: Box<Expr>,
        attr: String,
    },
    /// Single generator with at most one filter (latticepy §1.3).
    ListComp {
        elt: Box<Expr>,
        target: Box<Expr>,
        iter: Box<Expr>,
        cond: Option<Box<Expr>>,
    },
}
