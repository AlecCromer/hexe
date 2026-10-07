//! Recursive-descent parser with a nesting cap. Precedence (low→high): assignment, `?:`, `||`, `&&`, equality,
//! relational, additive, multiplicative, unary, postfix.
use crate::ast::{BinOp, Declarator, Expr, ExprKind, Program, Stmt, UnOp};
use crate::{Tok, Token, Ty, Value, VexError, lex};

/// Max syntactic nesting (statements + expressions) before `Err` (stack safety).
pub const MAX_DEPTH: u32 = 96;

struct Parser {
    t: Vec<Token>,
    i: usize,
    depth: u32,
}

type R<T> = Result<T, VexError>;

/// Source → [`Program`]. Parse errors are `kind = Parse` with `line:col`.
pub fn parse(src: &str) -> R<Program> {
    let mut p = Parser { t: lex(src)?, i: 0, depth: 0 };
    let mut stmts = Vec::new();
    while p.peek().tok != Tok::Eof {
        stmts.push(p.stmt()?);
    }
    Ok(Program { stmts })
}

fn binop(s: &str) -> Option<(BinOp, u8)> {
    Some(match s {
        "||" => (BinOp::Or, 1),
        "&&" => (BinOp::And, 2),
        "==" => (BinOp::Eq, 3),
        "!=" => (BinOp::Ne, 3),
        "<" => (BinOp::Lt, 4),
        "<=" => (BinOp::Le, 4),
        ">" => (BinOp::Gt, 4),
        ">=" => (BinOp::Ge, 4),
        "+" => (BinOp::Add, 5),
        "-" => (BinOp::Sub, 5),
        "*" => (BinOp::Mul, 6),
        "/" => (BinOp::Div, 6),
        "%" => (BinOp::Mod, 6),
        _ => return None,
    })
}

fn assign_op(s: &str) -> Option<Option<BinOp>> {
    Some(match s {
        "=" => None,
        "+=" => Some(BinOp::Add),
        "-=" => Some(BinOp::Sub),
        "*=" => Some(BinOp::Mul),
        "/=" => Some(BinOp::Div),
        _ => return None,
    })
}

fn component(name: &str) -> Option<usize> {
    match name {
        "x" | "r" => Some(0),
        "y" | "g" => Some(1),
        "z" | "b" => Some(2),
        "w" | "a" => Some(3),
        _ => None,
    }
}

fn is_lvalue(e: &Expr) -> bool {
    match &e.k {
        ExprKind::Var(_) | ExprKind::Attr(..) => true,
        ExprKind::Member(b, _) => is_lvalue(b),
        _ => false,
    }
}

impl Parser {
    fn peek(&self) -> &Token {
        // `lex` always ends with Eof and we never advance past it.
        self.t.get(self.i).unwrap_or_else(|| &self.t[self.t.len() - 1])
    }
    fn peek_at(&self, k: usize) -> &Tok {
        self.t.get(self.i + k).map_or(&Tok::Eof, |t| &t.tok)
    }
    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if t.tok != Tok::Eof {
            self.i += 1;
        }
        t
    }
    fn is_p(&self, s: &str) -> bool {
        matches!(&self.peek().tok, Tok::P(p) if *p == s)
    }
    fn eat_p(&mut self, s: &str) -> bool {
        let hit = self.is_p(s);
        if hit {
            self.i += 1;
        }
        hit
    }
    fn err_here<T>(&self, msg: impl Into<String>) -> R<T> {
        let t = self.peek();
        Err(VexError::parse(t.line, t.col, msg))
    }
    fn expect_p(&mut self, s: &str, what: &str) -> R<()> {
        if self.eat_p(s) {
            Ok(())
        } else {
            self.err_here(format!("expected `{s}` {what}, found {}", self.peek().tok.show()))
        }
    }
    fn enter(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { self.err_here("nesting too deep") } else { Ok(()) }
    }
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ---- statements ----
    fn stmt(&mut self) -> R<Stmt> {
        self.enter()?;
        let r = self.stmt_inner();
        self.leave();
        r
    }
    fn is_decl(&self) -> Option<Ty> {
        match (self.peek_at(0), self.peek_at(1)) {
            (Tok::Ident(k), Tok::Ident(_)) => Ty::from_keyword(k),
            _ => None,
        }
    }
    /// Declaration or expression, without the trailing `;`.
    fn simple(&mut self) -> R<Stmt> {
        if let Some(ty) = self.is_decl() {
            self.bump();
            let mut ds = Vec::new();
            loop {
                let n = self.bump();
                let Tok::Ident(name) = n.tok else {
                    return Err(VexError::parse(n.line, n.col, "expected a variable name"));
                };
                let init = if self.eat_p("=") { Some(self.expr()?) } else { None };
                ds.push(Declarator { name, init, line: n.line, col: n.col });
                if !self.eat_p(",") {
                    break;
                }
            }
            Ok(Stmt::Decl(ty, ds))
        } else {
            Ok(Stmt::Expr(self.expr()?))
        }
    }
    fn stmt_inner(&mut self) -> R<Stmt> {
        if self.eat_p(";") {
            return Ok(Stmt::Empty);
        }
        if self.eat_p("{") {
            let mut v = Vec::new();
            while !self.is_p("}") {
                if self.peek().tok == Tok::Eof {
                    return self.err_here("expected `}` to close block, found end of input");
                }
                v.push(self.stmt()?);
            }
            self.bump();
            return Ok(Stmt::Block(v));
        }
        if let Tok::Ident(kw) = self.peek().tok.clone() {
            match kw.as_str() {
                "if" => {
                    self.bump();
                    self.expect_p("(", "after `if`")?;
                    let c = self.expr()?;
                    self.expect_p(")", "after condition")?;
                    let a = Box::new(self.stmt()?);
                    let b = if matches!(&self.peek().tok, Tok::Ident(e) if e == "else") {
                        self.bump();
                        Some(Box::new(self.stmt()?))
                    } else {
                        None
                    };
                    return Ok(Stmt::If(c, a, b));
                }
                "while" => {
                    self.bump();
                    self.expect_p("(", "after `while`")?;
                    let c = self.expr()?;
                    self.expect_p(")", "after condition")?;
                    return Ok(Stmt::While(c, Box::new(self.stmt()?)));
                }
                "for" => {
                    self.bump();
                    self.expect_p("(", "after `for`")?;
                    let init = if self.is_p(";") { None } else { Some(Box::new(self.simple()?)) };
                    self.expect_p(";", "in `for`")?;
                    let cond = if self.is_p(";") { None } else { Some(self.expr()?) };
                    self.expect_p(";", "in `for`")?;
                    let step = if self.is_p(")") { None } else { Some(self.expr()?) };
                    self.expect_p(")", "to close `for`")?;
                    return Ok(Stmt::For(init, cond, step, Box::new(self.stmt()?)));
                }
                "break" | "continue" | "return" => {
                    self.bump();
                    self.expect_p(";", &format!("after `{kw}`"))?;
                    return Ok(match kw.as_str() {
                        "break" => Stmt::Break,
                        "continue" => Stmt::Continue,
                        _ => Stmt::Return,
                    });
                }
                _ => {}
            }
        }
        let s = self.simple()?;
        self.expect_p(";", "at end of statement")?;
        Ok(s)
    }

    // ---- expressions ----
    fn expr(&mut self) -> R<Expr> {
        self.enter()?;
        let r = self.assign();
        self.leave();
        r
    }
    fn mk(k: ExprKind, t: &Token) -> Expr {
        Expr { k, line: t.line, col: t.col }
    }
    fn assign(&mut self) -> R<Expr> {
        let lhs = self.ternary()?;
        let op = match &self.peek().tok {
            Tok::P(p) => assign_op(p),
            _ => None,
        };
        let Some(op) = op else { return Ok(lhs) };
        let t = self.bump();
        if !is_lvalue(&lhs) {
            return Err(VexError::parse(t.line, t.col, "left side of assignment is not assignable"));
        }
        let rhs = self.expr()?;
        Ok(Expr { line: lhs.line, col: lhs.col, k: ExprKind::Assign(op, Box::new(lhs), Box::new(rhs)) })
    }
    fn ternary(&mut self) -> R<Expr> {
        let c = self.binary(1)?;
        if !self.is_p("?") {
            return Ok(c);
        }
        self.bump();
        let a = self.expr()?;
        self.expect_p(":", "in `?:`")?;
        let b = self.expr()?;
        Ok(Expr { line: c.line, col: c.col, k: ExprKind::Ternary(Box::new(c), Box::new(a), Box::new(b)) })
    }
    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.unary()?;
        while let Tok::P(p) = &self.peek().tok {
            let Some((op, prec)) = binop(p) else { break };
            if prec < min {
                break;
            }
            self.bump();
            let rhs = self.binary(prec + 1)?;
            lhs = Expr { line: lhs.line, col: lhs.col, k: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)) };
        }
        Ok(lhs)
    }
    fn unary(&mut self) -> R<Expr> {
        self.enter()?;
        let r = self.unary_inner();
        self.leave();
        r
    }
    fn unary_inner(&mut self) -> R<Expr> {
        let t = self.peek().clone();
        if let Tok::P(p) = t.tok {
            match p {
                "-" | "!" | "+" => {
                    self.bump();
                    let e = self.unary()?;
                    return Ok(match p {
                        "-" => Self::mk(ExprKind::Unary(UnOp::Neg, Box::new(e)), &t),
                        "!" => Self::mk(ExprKind::Unary(UnOp::Not, Box::new(e)), &t),
                        _ => e,
                    });
                }
                "++" | "--" => {
                    self.bump();
                    let e = self.unary()?;
                    if !is_lvalue(&e) {
                        return Err(VexError::parse(t.line, t.col, "operand of ++/-- is not assignable"));
                    }
                    let delta = if p == "++" { 1 } else { -1 };
                    return Ok(Self::mk(ExprKind::IncDec { target: Box::new(e), delta, post: false }, &t));
                }
                _ => {}
            }
        }
        self.postfix()
    }
    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.is_p(".") {
                self.bump();
                let n = self.bump();
                let idx = match &n.tok {
                    Tok::Ident(s) => component(s),
                    _ => None,
                };
                let Some(idx) = idx else {
                    return Err(VexError::parse(
                        n.line,
                        n.col,
                        format!("expected component x/y/z/w after `.`, found {}", n.tok.show()),
                    ));
                };
                e = Expr { line: e.line, col: e.col, k: ExprKind::Member(Box::new(e), idx) };
            } else if self.is_p("++") || self.is_p("--") {
                let t = self.bump();
                if !is_lvalue(&e) {
                    return Err(VexError::parse(t.line, t.col, "operand of ++/-- is not assignable"));
                }
                let delta = if t.tok == Tok::P("++") { 1 } else { -1 };
                e = Expr { line: e.line, col: e.col, k: ExprKind::IncDec { target: Box::new(e), delta, post: true } };
            } else {
                return Ok(e);
            }
        }
    }
    fn primary(&mut self) -> R<Expr> {
        let t = self.bump();
        match t.tok.clone() {
            Tok::Int(i) => Ok(Self::mk(ExprKind::Lit(Value::Int(i)), &t)),
            Tok::Float(f) => Ok(Self::mk(ExprKind::Lit(Value::Float(f)), &t)),
            Tok::Str(s) => Ok(Self::mk(ExprKind::Lit(Value::Str(s)), &t)),
            Tok::Attr { prefix, name } => Ok(Self::mk(ExprKind::Attr(prefix, name), &t)),
            Tok::Ident(name) => {
                if self.eat_p("(") {
                    let mut args = Vec::new();
                    if !self.is_p(")") {
                        loop {
                            args.push(self.expr()?);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                    }
                    self.expect_p(")", "to close call")?;
                    Ok(Self::mk(ExprKind::Call(name, args), &t))
                } else if Ty::from_keyword(&name).is_some() {
                    Err(VexError::parse(t.line, t.col, format!("type `{name}` must be followed by a variable name")))
                } else {
                    Ok(Self::mk(ExprKind::Var(name), &t))
                }
            }
            Tok::P("(") => {
                let e = self.expr()?;
                self.expect_p(")", "to close `(`")?;
                Ok(e)
            }
            Tok::P("{") => {
                let mut v = vec![self.expr()?];
                while self.eat_p(",") {
                    v.push(self.expr()?);
                }
                if !(2..=4).contains(&v.len()) {
                    return Err(VexError::parse(t.line, t.col, "vector literal needs 2 to 4 components"));
                }
                self.expect_p("}", "to close vector literal")?;
                Ok(Self::mk(ExprKind::VecLit(v), &t))
            }
            other => Err(VexError::parse(t.line, t.col, format!("expected expression, found {}", other.show()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ok(s: &str) -> Program {
        parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }
    fn bad(s: &str) -> VexError {
        parse(s).expect_err(s)
    }
    #[test]
    fn parse_error_has_position() {
        let e = bad("@P.y +=;");
        assert_eq!((e.line, e.col), (1, 8));
        assert_eq!(e.to_string(), "1:8: expected expression, found `;`");
        let e = bad("float a = 1;\n  a = (2;");
        assert_eq!((e.line, e.col), (2, 9));
    }
    #[test]
    fn precedence_and_assign_shapes() {
        let p = ok("f@h = @P.x * 2 + 1;");
        let Stmt::Expr(Expr { k: ExprKind::Assign(None, _, rhs), .. }) = &p.stmts[0] else { panic!("not assign") };
        let ExprKind::Binary(BinOp::Add, l, _) = &rhs.k else { panic!("top must be +") };
        assert!(matches!(l.k, ExprKind::Binary(BinOp::Mul, ..)));
        assert!(matches!(
            ok("@P.y += 1;").stmts[0],
            Stmt::Expr(Expr { k: ExprKind::Assign(Some(BinOp::Add), ..), .. })
        ));
    }
    #[test]
    fn full_grammar_parses() {
        let src = "int n = addpoint(0, {5,5,5}); vector a = {1,2,3}, b; float f = 1, g;
            /* c */ for (int i = 0; i < 3; i++) { if (i == 1) continue; else if (i > 5) break; @P.x += 1; }
            while (n > 0) n -= 1; for (;;) { return; } string s = 'x'; v@v = -a * 2 + (b / 2); i@k = !n && n || 1 ? 1 : 0; ;";
        assert_eq!(ok(src).stmts.len(), 10);
        assert!(matches!(&ok("for(;;){}").stmts[0], Stmt::For(None, None, None, _)));
    }
    #[test]
    fn rejects_garbage_without_panic() {
        for s in
            ["1 = 2;", "@P.q = 1;", "if (1", "{ 1;", "{1};", "f(;", "x = ;", "int ;", "a ^ b;", "i++ ++ 1;", "for(;;"]
        {
            let _ = bad(s);
        }
    }
    #[test]
    fn deep_nesting_is_an_error_not_a_stack_overflow() {
        let e = bad(&format!("x = {}1{};", "(".repeat(5000), ")".repeat(5000)));
        assert!(e.msg.contains("nesting"));
        let e = bad(&format!("{}{}", "{".repeat(5000), "}".repeat(5000)));
        assert!(e.msg.contains("nesting"));
        let e = bad(&format!("x = {}1;", "-".repeat(5000)));
        assert!(e.msg.contains("nesting"));
    }
}
