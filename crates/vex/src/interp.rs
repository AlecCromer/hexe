//! Tree-walking interpreter. One [`run`] = one execution of the snippet for one element; the geometry lives behind
//! [`Host`]. A shared step budget bounds total work (infinite loops end in `Err`, never hang).
use crate::ast::{BinOp, Expr, ExprKind, Program, Stmt, UnOp};
use crate::builtins;
use crate::{ErrorKind, HostError, Ty, Value, VexError, builtin_ty, is_readonly};

/// Geometry/attribute access for the element currently being processed.
pub trait Host {
    /// Read `@name` of the current element. `hint` = explicit/builtin type (else infer). `None` = attribute absent.
    fn attr(&mut self, name: &str, hint: Option<Ty>) -> Result<Option<Value>, HostError>;
    /// Write `@name` of the current element (creates the attribute if absent).
    fn set_attr(&mut self, name: &str, hint: Option<Ty>, v: Value) -> Result<(), HostError>;
    fn add_point(&mut self, p: [f64; 3]) -> Result<i64, HostError>;
    fn remove_point(&mut self, n: i64) -> Result<(), HostError>;
    /// `kind` = `poly` | `polyline`.
    fn add_prim(&mut self, kind: &str, pts: &[i64]) -> Result<i64, HostError>;
    fn set_point_attrib(&mut self, name: &str, n: i64, v: Value) -> Result<(), HostError>;
    fn point(&mut self, name: &str, n: i64) -> Result<Value, HostError>;
    fn npoints(&mut self) -> i64;
}

/// Internal failure; positioned by the caller.
pub(crate) enum Fail {
    Msg(String),
    Unsup(String),
}
pub(crate) type R<T> = Result<T, Fail>;
pub(crate) fn fail<T>(m: impl Into<String>) -> R<T> {
    Err(Fail::Msg(m.into()))
}
impl From<HostError> for Fail {
    fn from(e: HostError) -> Fail {
        match e {
            HostError::Failed(m) => Fail::Msg(m),
            HostError::Unsupported(m) => Fail::Unsup(m),
        }
    }
}
fn at(f: Fail, e: &Expr) -> VexError {
    match f {
        Fail::Msg(m) => VexError::new(ErrorKind::Runtime, e.line, e.col, m),
        Fail::Unsup(m) => VexError::new(ErrorKind::Unsupported, e.line, e.col, m),
    }
}

enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

struct Var {
    name: String,
    ty: Ty,
    val: Value,
}

struct Interp<'a> {
    host: &'a mut dyn Host,
    steps: &'a mut u64,
    vars: Vec<Var>,
}

/// Run `prog` once against `host`, spending from `steps` (error when it reaches 0).
pub fn run(prog: &Program, host: &mut dyn Host, steps: &mut u64) -> Result<(), VexError> {
    let mut it = Interp { host, steps, vars: Vec::new() };
    for s in &prog.stmts {
        if !matches!(it.stmt(s)?, Flow::Next) {
            break;
        }
    }
    Ok(())
}

fn rt(e: &Expr, m: impl Into<String>) -> VexError {
    VexError::new(ErrorKind::Runtime, e.line, e.col, m)
}

impl Interp<'_> {
    fn tick(&mut self, e: &Expr) -> Result<(), VexError> {
        if *self.steps == 0 {
            return Err(rt(e, "step budget exhausted (runaway loop?)"));
        }
        *self.steps -= 1;
        Ok(())
    }
    fn tick_at(&mut self, line: u32, col: u32) -> Result<(), VexError> {
        if *self.steps == 0 {
            return Err(VexError::new(ErrorKind::Runtime, line, col, "step budget exhausted (runaway loop?)"));
        }
        *self.steps -= 1;
        Ok(())
    }

    fn block(&mut self, v: &[Stmt]) -> Result<Flow, VexError> {
        let mark = self.vars.len();
        let mut flow = Flow::Next;
        for s in v {
            flow = self.stmt(s)?;
            if !matches!(flow, Flow::Next) {
                break;
            }
        }
        self.vars.truncate(mark);
        Ok(flow)
    }

    fn cond(&mut self, e: &Expr) -> Result<bool, VexError> {
        Ok(self.eval(e)?.truthy())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<Flow, VexError> {
        match s {
            Stmt::Empty => Ok(Flow::Next),
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Return => Ok(Flow::Return),
            Stmt::Expr(e) => {
                self.eval(e)?;
                Ok(Flow::Next)
            }
            Stmt::Block(v) => self.block(v),
            Stmt::Decl(ty, ds) => {
                for d in ds {
                    self.tick_at(d.line, d.col)?;
                    let val = match &d.init {
                        Some(e) => {
                            let v = self.eval(e)?;
                            v.coerce(*ty).ok_or_else(|| {
                                rt(e, format!("cannot initialise {} `{}` from {}", ty.name(), d.name, v.ty().name()))
                            })?
                        }
                        None => Value::zero(*ty),
                    };
                    self.vars.push(Var { name: d.name.clone(), ty: *ty, val });
                }
                Ok(Flow::Next)
            }
            Stmt::If(c, a, b) => {
                if self.cond(c)? {
                    self.scoped(a)
                } else if let Some(b) = b {
                    self.scoped(b)
                } else {
                    Ok(Flow::Next)
                }
            }
            Stmt::While(c, body) => {
                while self.cond(c)? {
                    match self.scoped(body)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        _ => {}
                    }
                }
                Ok(Flow::Next)
            }
            Stmt::For(init, c, step, body) => {
                let mark = self.vars.len();
                let r = self.for_loop(init.as_deref(), c.as_ref(), step.as_ref(), body);
                self.vars.truncate(mark);
                r
            }
        }
    }

    fn scoped(&mut self, s: &Stmt) -> Result<Flow, VexError> {
        let mark = self.vars.len();
        let r = self.stmt(s);
        self.vars.truncate(mark);
        r
    }

    fn for_loop(
        &mut self,
        init: Option<&Stmt>,
        c: Option<&Expr>,
        step: Option<&Expr>,
        body: &Stmt,
    ) -> Result<Flow, VexError> {
        if let Some(i) = init {
            self.stmt(i)?;
        }
        loop {
            if let Some(c) = c
                && !self.cond(c)?
            {
                return Ok(Flow::Next);
            }
            match self.scoped(body)? {
                Flow::Break => return Ok(Flow::Next),
                Flow::Return => return Ok(Flow::Return),
                _ => {}
            }
            if let Some(s) = step {
                self.eval(s)?;
            } else if c.is_none() {
                self.tick_at(0, 0)?; // `for(;;){}` has no expression to tick
            }
        }
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, VexError> {
        self.tick(e)?;
        match &e.k {
            ExprKind::Lit(v) => Ok(v.clone()),
            ExprKind::Var(n) => match self.vars.iter().rev().find(|v| v.name == *n) {
                Some(v) => Ok(v.val.clone()),
                None => Err(rt(e, format!("undefined variable `{n}`"))),
            },
            ExprKind::Attr(p, n) => self.read_attr(e, *p, n),
            ExprKind::Member(b, i) => {
                let v = self.eval(b)?;
                match v.comps().and_then(|c| c.get(*i)) {
                    Some(x) => Ok(Value::Float(*x)),
                    None => Err(rt(
                        e,
                        format!(
                            "component {} not available on {}",
                            "xyzw".chars().nth(*i).unwrap_or('?'),
                            v.ty().name()
                        ),
                    )),
                }
            }
            ExprKind::Unary(op, x) => {
                let v = self.eval(x)?;
                unary(*op, v).map_err(|f| at(f, e))
            }
            ExprKind::Binary(BinOp::And, a, b) => {
                let l = self.eval(a)?.truthy();
                Ok(Value::Int(i64::from(l && self.eval(b)?.truthy())))
            }
            ExprKind::Binary(BinOp::Or, a, b) => {
                let l = self.eval(a)?.truthy();
                Ok(Value::Int(i64::from(l || self.eval(b)?.truthy())))
            }
            ExprKind::Binary(op, a, b) => {
                let (l, r) = (self.eval(a)?, self.eval(b)?);
                builtins::arith(*op, &l, &r).map_err(|f| at(f, e))
            }
            ExprKind::Ternary(c, a, b) => {
                if self.eval(c)?.truthy() {
                    self.eval(a)
                } else {
                    self.eval(b)
                }
            }
            ExprKind::VecLit(parts) => {
                let mut c = Vec::with_capacity(parts.len());
                for p in parts {
                    let v = self.eval(p)?;
                    c.push(
                        v.as_f64().ok_or_else(|| {
                            rt(p, format!("vector component must be a number, got {}", v.ty().name()))
                        })?,
                    );
                }
                Value::from_comps(&c).ok_or_else(|| rt(e, "vector literal needs 2 to 4 components"))
            }
            ExprKind::Call(name, args) => {
                let mut a = Vec::with_capacity(args.len());
                for x in args {
                    a.push(self.eval(x)?);
                }
                builtins::call(self.host, name, &a).map_err(|f| at(f, e))
            }
            ExprKind::Assign(op, target, rhs) => {
                let r = self.eval(rhs)?;
                let v = match op {
                    None => r,
                    Some(op) => {
                        let cur = self.eval(target)?;
                        builtins::arith(*op, &cur, &r).map_err(|f| at(f, e))?
                    }
                };
                self.store(target, v.clone())?;
                Ok(v)
            }
            ExprKind::IncDec { target, delta, post } => {
                let cur = self.eval(target)?;
                let one = Value::Int(*delta);
                let new = builtins::arith(BinOp::Add, &cur, &one).map_err(|f| at(f, e))?;
                self.store(target, new.clone())?;
                Ok(if *post { cur } else { new })
            }
        }
    }

    fn read_attr(&mut self, e: &Expr, prefix: Option<Ty>, name: &str) -> Result<Value, VexError> {
        let hint = prefix.or_else(|| builtin_ty(name));
        let got = self.host.attr(name, hint).map_err(|h| at(h.into(), e))?;
        match (got, hint) {
            (Some(v), Some(t)) if v.ty() != t => match v.coerce(t) {
                Some(c) => Ok(c),
                None => Err(rt(e, format!("attribute `{name}` is {} but used as {}", v.ty().name(), t.name()))),
            },
            (Some(v), _) => Ok(v),
            (None, h) => Ok(Value::zero(h.unwrap_or(Ty::Float))),
        }
    }

    fn store(&mut self, target: &Expr, v: Value) -> Result<(), VexError> {
        match &target.k {
            ExprKind::Var(n) => {
                let Some(var) = self.vars.iter_mut().rev().find(|x| x.name == *n) else {
                    return Err(rt(target, format!("undefined variable `{n}`")));
                };
                match v.coerce(var.ty) {
                    Some(c) => {
                        var.val = c;
                        Ok(())
                    }
                    None => Err(rt(target, format!("cannot assign {} to {} `{n}`", v.ty().name(), var.ty.name()))),
                }
            }
            ExprKind::Attr(p, n) => {
                if is_readonly(n) {
                    return Err(rt(target, format!("`@{n}` is read-only")));
                }
                let hint = p.or_else(|| builtin_ty(n));
                let v = match hint {
                    Some(t) => v.coerce(t).ok_or_else(|| {
                        rt(target, format!("cannot assign {} to {} attribute `@{n}`", v.ty().name(), t.name()))
                    })?,
                    None => v,
                };
                self.host.set_attr(n, hint, v).map_err(|h| at(h.into(), target))
            }
            ExprKind::Member(base, i) => {
                let cur = self.eval(base)?;
                let x = v
                    .as_f64()
                    .ok_or_else(|| rt(target, format!("component must be a number, got {}", v.ty().name())))?;
                let mut c = cur.comps().map(<[f64]>::to_vec).unwrap_or_default();
                match c.get_mut(*i) {
                    Some(slot) => *slot = x,
                    None => {
                        return Err(rt(
                            target,
                            format!(
                                "component {} not available on {}",
                                "xyzw".chars().nth(*i).unwrap_or('?'),
                                cur.ty().name()
                            ),
                        ));
                    }
                }
                let nv = Value::from_comps(&c).ok_or_else(|| rt(target, "bad vector"))?;
                self.store(base, nv)
            }
            _ => Err(rt(target, "not assignable")),
        }
    }
}

fn unary(op: UnOp, v: Value) -> R<Value> {
    match (op, &v) {
        (UnOp::Not, _) => Ok(Value::Int(i64::from(!v.truthy()))),
        (UnOp::Neg, Value::Int(i)) => Ok(Value::Int(i.wrapping_neg())),
        (UnOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
        (UnOp::Neg, x) => match x.comps() {
            Some(c) => {
                Value::from_comps(&c.iter().map(|a| -a).collect::<Vec<_>>()).map_or_else(|| fail("bad vector"), Ok)
            }
            None => fail(format!("cannot negate {}", x.ty().name())),
        },
    }
}
