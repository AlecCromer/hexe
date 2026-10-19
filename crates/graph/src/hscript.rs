//! HScript expression evaluator (the language of `.hip` parm channels: `ch("../ctrl/width")/2`).
//!
//! Values are numbers (f64) or strings. Operators (loosest first): `||` · `&&` · `== != < > <= >=` · `+ -` (string `+`
//! concatenates) · `* / %` · `^` (power, right-assoc) · unary `- + !`. Literals: numbers, `"..."` / `'...'` strings,
//! `$VAR` / `${VAR}` variables. Trig is in DEGREES (HScript). Functions: math (`sin cos tan asin acos atan atan2 sqrt
//! abs pow exp log log10 floor ceil round int frac sign min max clamp`), `fit fit01 fit10 fit11 smooth`, `if ifs`,
//! `strcmp strlen strreplace strcat atof`, channel refs `ch chf chs chsraw` (path resolved by the [`Scope`]).
//! Anything else (geometry queries `detail()` / `point()` / `$N`, `rand`, python) is [`ExprError::Unsupported`] so the
//! caller keeps the saved value and says so. Hostile input: length-capped, nesting depth capped, never panics.
use std::fmt;

/// Longest expression accepted (bytes).
pub const MAX_EXPR: usize = 64 * 1024;
/// Deepest nesting of parentheses / calls / unary operators.
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
}
impl Value {
    /// Number view (HScript: a string converts with `atof` rules, unparsable = 0).
    pub fn num(&self) -> f64 {
        match self {
            Value::Num(x) => *x,
            Value::Str(s) => atof(s),
        }
    }
    /// String view (numbers print without a trailing `.0`).
    pub fn text(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Num(x) => fmt_num(*x),
        }
    }
}
fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{}", x as i64) } else { format!("{x}") }
}
fn atof(s: &str) -> f64 {
    let t = s.trim();
    let end = t
        .char_indices()
        .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || ((*c == '-' || *c == '+') && *i == 0) || *c == 'e')
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    (0..=end).rev().find_map(|e| t.get(..e)?.parse::<f64>().ok()).unwrap_or(0.0)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprError {
    /// Malformed expression.
    Syntax(String),
    /// Valid HScript this evaluator does not model (the caller falls back to the saved value).
    Unsupported(String),
}
impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::Syntax(m) => write!(f, "syntax: {m}"),
            ExprError::Unsupported(m) => write!(f, "unsupported: {m}"),
        }
    }
}

/// What an expression can see: variables and other channels.
pub trait Scope {
    /// `$NAME` (without the `$`); `None` = unknown variable (Unsupported).
    fn var(&self, name: &str) -> Option<Value>;
    /// `ch("path")` family: `path` as written (relative to the evaluating node), `string` = chs / chsraw.
    fn channel(&self, path: &str, string: bool) -> Result<Value, ExprError>;
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Var(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

const OPS: [&str; 15] = ["||", "&&", "==", "!=", "<=", ">=", "<", ">", "+", "-", "*", "/", "%", "^", "!"];

fn lex(src: &str) -> Result<Vec<Tok>, ExprError> {
    if src.len() > MAX_EXPR {
        return Err(ExprError::Syntax("expression too long".into()));
    }
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\\' && b.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            // line continuation (hython: `ch("a")/2\` evaluates as `ch("a")/2`)
            i += 1;
        } else if c == '{' && out.is_empty() {
            return Err(ExprError::Unsupported("HScript statement block `{ ... }`".into()));
        } else if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let s = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.') {
                i += 1;
            }
            if i < b.len() && (b[i] == 'e' || b[i] == 'E') {
                let save = i;
                i += 1;
                if i < b.len() && (b[i] == '+' || b[i] == '-') {
                    i += 1;
                }
                if i < b.len() && b[i].is_ascii_digit() {
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let t: String = b[s..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| ExprError::Syntax(format!("bad number `{t}`")))?));
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut s = String::new();
            loop {
                match b.get(i) {
                    None => return Err(ExprError::Syntax("unterminated string".into())),
                    Some('\\') => {
                        if let Some(&n) = b.get(i + 1) {
                            s.push(match n {
                                'n' => '\n',
                                't' => '\t',
                                o => o,
                            });
                        }
                        i += 2;
                    }
                    Some(&q) if q == c => {
                        i += 1;
                        break;
                    }
                    Some(&o) => {
                        s.push(o);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c == '$' {
            i += 1;
            let braced = b.get(i) == Some(&'{');
            if braced {
                i += 1;
            }
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            let name: String = b[s..i].iter().collect();
            if braced {
                if b.get(i) != Some(&'}') {
                    return Err(ExprError::Syntax("unterminated ${".into()));
                }
                i += 1;
            }
            if name.is_empty() {
                return Err(ExprError::Syntax("empty variable".into()));
            }
            out.push(Tok::Var(name));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(b[s..i].iter().collect()));
        } else if c == '(' {
            out.push(Tok::LParen);
            i += 1;
        } else if c == ')' {
            out.push(Tok::RParen);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if c == '@' {
            return Err(ExprError::Unsupported("`@` attribute reference".into()));
        } else {
            let two: String = b[i..(i + 2).min(b.len())].iter().collect();
            let op = OPS
                .iter()
                .find(|o| o.len() == 2 && **o == two)
                .or_else(|| OPS.iter().find(|o| o.len() == 1 && o.starts_with(c)));
            match op {
                Some(o) => {
                    i += o.len();
                    out.push(Tok::Op(o));
                }
                None => return Err(ExprError::Syntax(format!("unexpected `{c}`"))),
            }
        }
    }
    Ok(out)
}

struct P<'a> {
    t: Vec<Tok>,
    i: usize,
    depth: usize,
    scope: &'a dyn Scope,
}

fn bool_num(b: bool) -> Value {
    Value::Num(if b { 1.0 } else { 0.0 })
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Num(x) => *x != 0.0,
        Value::Str(s) => !s.is_empty() && atof(s) != 0.0,
    }
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn op(&mut self, ops: &[&str]) -> Option<&'static str> {
        if let Some(Tok::Op(o)) = self.peek() {
            let o = *o;
            if ops.contains(&o) {
                self.i += 1;
                return Some(o);
            }
        }
        None
    }
    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH { Err(ExprError::Syntax("nested too deeply".into())) } else { Ok(()) }
    }
    fn expr(&mut self) -> Result<Value, ExprError> {
        self.enter()?;
        let r = self.or()?;
        self.depth -= 1;
        Ok(r)
    }
    fn or(&mut self) -> Result<Value, ExprError> {
        let mut a = self.and()?;
        while self.op(&["||"]).is_some() {
            let b = self.and()?;
            a = bool_num(truthy(&a) || truthy(&b));
        }
        Ok(a)
    }
    fn and(&mut self) -> Result<Value, ExprError> {
        let mut a = self.cmp()?;
        while self.op(&["&&"]).is_some() {
            let b = self.cmp()?;
            a = bool_num(truthy(&a) && truthy(&b));
        }
        Ok(a)
    }
    fn cmp(&mut self) -> Result<Value, ExprError> {
        let mut a = self.add()?;
        while let Some(o) = self.op(&["==", "!=", "<=", ">=", "<", ">"]) {
            let b = self.add()?;
            let ord = match (&a, &b) {
                (Value::Str(x), Value::Str(y)) => x.partial_cmp(y),
                _ => a.num().partial_cmp(&b.num()),
            };
            use std::cmp::Ordering::*;
            a = bool_num(match o {
                "==" => ord == Some(Equal),
                "!=" => ord != Some(Equal),
                "<" => ord == Some(Less),
                ">" => ord == Some(Greater),
                "<=" => matches!(ord, Some(Less | Equal)),
                _ => matches!(ord, Some(Greater | Equal)),
            });
        }
        Ok(a)
    }
    fn add(&mut self) -> Result<Value, ExprError> {
        let mut a = self.mul()?;
        while let Some(o) = self.op(&["+", "-"]) {
            let b = self.mul()?;
            a = match (o, &a, &b) {
                ("+", Value::Str(x), _) => Value::Str(format!("{x}{}", b.text())),
                ("+", _, Value::Str(y)) => Value::Str(format!("{}{y}", a.text())),
                ("+", _, _) => Value::Num(a.num() + b.num()),
                _ => Value::Num(a.num() - b.num()),
            };
        }
        Ok(a)
    }
    fn mul(&mut self) -> Result<Value, ExprError> {
        let mut a = self.pow()?;
        while let Some(o) = self.op(&["*", "/", "%"]) {
            let (x, y) = (a.num(), self.pow()?.num());
            a = Value::Num(match o {
                "*" => x * y,
                // HScript: division by zero yields 0 rather than inf
                "/" => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x / y
                    }
                }
                _ => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x % y
                    }
                }
            });
        }
        Ok(a)
    }
    /// `^` is LEFT-associative in HScript (hython: `2 ^ 3 ^ 2` = 64).
    fn pow(&mut self) -> Result<Value, ExprError> {
        let mut a = self.unary()?;
        while self.op(&["^"]).is_some() {
            let b = self.unary()?;
            a = Value::Num(a.num().powf(b.num()));
        }
        Ok(a)
    }
    fn unary(&mut self) -> Result<Value, ExprError> {
        if let Some(o) = self.op(&["-", "+", "!"]) {
            self.enter()?;
            let v = self.unary()?;
            self.depth -= 1;
            return Ok(match o {
                "-" => Value::Num(-v.num()),
                "!" => bool_num(!truthy(&v)),
                _ => Value::Num(v.num()),
            });
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<Value, ExprError> {
        let t = self.peek().cloned().ok_or_else(|| ExprError::Syntax("unexpected end".into()))?;
        self.i += 1;
        match t {
            Tok::Num(x) => Ok(Value::Num(x)),
            Tok::Str(s) => Ok(Value::Str(s)),
            Tok::Var(v) => var(self.scope, &v),
            Tok::LParen => {
                let v = self.expr()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err(ExprError::Syntax("expected `)`".into()));
                }
                self.i += 1;
                Ok(v)
            }
            Tok::Ident(name) => {
                if self.peek() != Some(&Tok::LParen) {
                    return Err(ExprError::Unsupported(format!("bare word `{name}`")));
                }
                self.i += 1;
                let mut args = Vec::new();
                if self.peek() == Some(&Tok::RParen) {
                    self.i += 1;
                } else {
                    loop {
                        args.push(self.expr()?);
                        match self.peek() {
                            Some(Tok::Comma) => self.i += 1,
                            Some(Tok::RParen) => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(ExprError::Syntax(format!("expected `,` or `)` in {name}()"))),
                        }
                    }
                }
                call(self.scope, &name, &args)
            }
            other => Err(ExprError::Syntax(format!("unexpected {other:?}"))),
        }
    }
}

fn var(scope: &dyn Scope, v: &str) -> Result<Value, ExprError> {
    match v {
        "PI" => Ok(Value::Num(std::f64::consts::PI)),
        "E" => Ok(Value::Num(std::f64::consts::E)),
        _ => scope.var(v).ok_or_else(|| ExprError::Unsupported(format!("variable ${v}"))),
    }
}

fn call(scope: &dyn Scope, name: &str, a: &[Value]) -> Result<Value, ExprError> {
    let n = |i: usize| a.get(i).map_or(0.0, Value::num);
    let s = |i: usize| a.get(i).map(Value::text).unwrap_or_default();
    let want = |k: usize| {
        if a.len() < k { Err(ExprError::Syntax(format!("{name}() needs {k} argument(s)"))) } else { Ok(()) }
    };
    let deg = std::f64::consts::PI / 180.0;
    let num = |x: f64| Ok(Value::Num(x));
    match name {
        "ch" | "chf" | "chs" | "chsraw" => {
            want(1)?;
            scope.channel(&s(0), name.starts_with("chs"))
        }
        "sin" => num((n(0) * deg).sin()),
        "cos" => num((n(0) * deg).cos()),
        "tan" => num((n(0) * deg).tan()),
        // out of domain = 0 (hython: acos(2) = 0)
        "asin" => num(if n(0).abs() <= 1.0 { n(0).asin() / deg } else { 0.0 }),
        "acos" => num(if n(0).abs() <= 1.0 { n(0).acos() / deg } else { 0.0 }),
        "atan" => num(n(0).atan() / deg),
        "atan2" => num(n(0).atan2(n(1)) / deg),
        "sqrt" => num(n(0).max(0.0).sqrt()),
        "abs" => num(n(0).abs()),
        "pow" => num(n(0).powf(n(1))),
        "exp" => num(n(0).exp()),
        "log" => num(if n(0) > 0.0 { n(0).ln() } else { 0.0 }),
        "log10" => num(if n(0) > 0.0 { n(0).log10() } else { 0.0 }),
        "floor" => num(n(0).floor()),
        "ceil" => num(n(0).ceil()),
        // halves round up (hython: round(-2.5) = -2, round(2.5) = 3)
        "round" | "rint" => num((n(0) + 0.5).floor()),
        "int" => num(n(0).trunc()),
        "frac" => num(n(0) - n(0).floor()),
        "sign" => num(if n(0) > 0.0 {
            1.0
        } else if n(0) < 0.0 {
            -1.0
        } else {
            0.0
        }),
        "min" => {
            want(2)?;
            num(n(0).min(n(1)))
        }
        "max" => {
            want(2)?;
            num(n(0).max(n(1)))
        }
        "clamp" => {
            want(3)?;
            num(n(0).max(n(1)).min(n(2)))
        }
        "fit" => {
            want(5)?;
            num(fit(n(0), n(1), n(2), n(3), n(4)))
        }
        "fit01" => {
            want(3)?;
            num(fit(n(0), 0.0, 1.0, n(1), n(2)))
        }
        "fit10" => {
            want(3)?;
            num(fit(n(0), 1.0, 0.0, n(1), n(2)))
        }
        "fit11" => {
            want(3)?;
            num(fit(n(0), -1.0, 1.0, n(1), n(2)))
        }
        "smooth" => {
            want(3)?;
            // smooth(value, min, max) (hython: smooth(1, 0, 0.25) = 1, smooth(0, 1, 0.5) = 0)
            // below min first, then at / above max
            let t = if n(0) <= n(1) {
                0.0
            } else if n(0) >= n(2) {
                1.0
            } else {
                (n(0) - n(1)) / (n(2) - n(1))
            };
            num(t * t * (3.0 - 2.0 * t))
        }
        "if" | "ifs" => {
            want(3)?;
            let v = if truthy(&a[0]) { a[1].clone() } else { a[2].clone() };
            Ok(if name == "ifs" { Value::Str(v.text()) } else { v })
        }
        "strcmp" => {
            want(2)?;
            num(match s(0).cmp(&s(1)) {
                std::cmp::Ordering::Less => -1.0,
                std::cmp::Ordering::Equal => 0.0,
                std::cmp::Ordering::Greater => 1.0,
            })
        }
        "strlen" => num(s(0).chars().count() as f64),
        "strcat" => Ok(Value::Str(a.iter().map(Value::text).collect())),
        "strreplace" => {
            want(3)?;
            let (src, from) = (s(0), s(1));
            Ok(Value::Str(if from.is_empty() { src } else { src.replace(&from, &s(2)) }))
        }
        "atof" => num(atof(&s(0))),
        other => Err(ExprError::Unsupported(format!("function {other}()"))),
    }
}

/// HScript `fit`: clamped linear remap (an empty source range gives `nmin`: hython fit(5,10,10,0,1) = 0).
fn fit(v: f64, omin: f64, omax: f64, nmin: f64, nmax: f64) -> f64 {
    if omin == omax {
        return nmin;
    }
    let t = ((v - omin) / (omax - omin)).clamp(0.0, 1.0);
    nmin + (nmax - nmin) * t
}

/// Evaluate `src` in `scope`.
pub fn eval(src: &str, scope: &dyn Scope) -> Result<Value, ExprError> {
    let t = lex(src)?;
    if t.is_empty() {
        return Err(ExprError::Syntax("empty expression".into()));
    }
    let mut p = P { t, i: 0, depth: 0, scope };
    let v = p.expr()?;
    if p.i != p.t.len() {
        return Err(ExprError::Syntax(format!("trailing input after token {}", p.i)));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct S;
    impl Scope for S {
        fn var(&self, name: &str) -> Option<Value> {
            match name {
                "F" => Some(Value::Num(240.0)),
                "FSTART" => Some(Value::Num(1.0)),
                _ => None,
            }
        }
        fn channel(&self, path: &str, string: bool) -> Result<Value, ExprError> {
            match (path, string) {
                ("../ctrl/width", false) => Ok(Value::Num(10.0)),
                ("../ctrl/depth", false) => Ok(Value::Num(2.0)),
                ("aperture", true) => Ok(Value::Str("setratio".into())),
                _ => Err(ExprError::Unsupported(format!("no channel {path}"))),
            }
        }
    }
    fn ev(s: &str) -> Result<Value, ExprError> {
        eval(s, &S)
    }
    #[test]
    fn channel_reference_expressions_evaluate() {
        let v = ev(r#"ch("../ctrl/width")/2+ch("../ctrl/depth")/2"#).expect("eval");
        assert_eq!(v, Value::Num(6.0));
        assert_eq!(ev(r#"ch("../ctrl/width")*2"#).expect("eval"), Value::Num(20.0));
        assert_eq!(ev("$FSTART").expect("eval"), Value::Num(1.0));
        assert_eq!(ev("${F} - 1").expect("eval"), Value::Num(239.0));
        assert_eq!(
            ev(r#"ifs(!strcmp(chs("aperture"), "setratio"), "set", chs("aperture"))"#).expect("eval"),
            Value::Str("set".into())
        );
        assert_eq!(ev("1.0 / 4").expect("eval"), Value::Num(0.25));
        // a trailing `\` line continuation is ignored (hython: channel `ch("w")/2 + 1\` = 6, oracle/houdini/hscript_continuation.py)
        assert!((ev(r#"ch("../ctrl/width")/2+ch("../ctrl/depth")/2\"#).expect("eval").num() - 6.0).abs() < 1e-12);
        assert!(matches!(ev("{\n if(1) { return 2; }\n}"), Err(ExprError::Unsupported(_))));
    }
    #[test]
    fn operators_precedence_and_degrees() {
        assert_eq!(ev("1 + 2 * 3 ^ 2").expect("eval"), Value::Num(19.0));
        // hython-checked (hou.hscriptExpression, Houdini 20.5.522)
        assert_eq!(ev("2 ^ 3 ^ 2").expect("eval"), Value::Num(64.0));
        assert_eq!(ev("-2 ^ 2").expect("eval"), Value::Num(4.0));
        assert_eq!(ev("round(-2.5)").expect("eval"), Value::Num(-2.0));
        assert_eq!(ev("round(2.5)").expect("eval"), Value::Num(3.0));
        assert_eq!(ev("smooth(1, 0, 0.25)").expect("eval"), Value::Num(1.0));
        assert_eq!(ev("smooth(0, 1, 0.5)").expect("eval"), Value::Num(0.0));
        assert_eq!(ev("fit(5, 10, 10, 0, 1)").expect("eval"), Value::Num(0.0));
        assert_eq!(ev("acos(2)").expect("eval"), Value::Num(0.0));
        assert_eq!(ev("frac(-0.25)").expect("eval"), Value::Num(0.75));
        assert_eq!(ev("2 + \"3\"").expect("eval"), Value::Str("23".into()));
        assert_eq!(ev("\"2\" * 3").expect("eval"), Value::Num(6.0));
        assert_eq!(ev("-7 % 3").expect("eval"), Value::Num(-1.0));
        assert_eq!(ev("1 < 2 && 3 >= 3 || 0").expect("eval"), Value::Num(1.0));
        assert!((ev("sin(90)").expect("eval").num() - 1.0).abs() < 1e-12);
        assert_eq!(ev("fit(5, 0, 10, 0, 1)").expect("eval"), Value::Num(0.5));
        assert_eq!(ev("fit(50, 0, 10, 0, 1)").expect("eval"), Value::Num(1.0));
        assert_eq!(ev("\"a\" + 1").expect("eval"), Value::Str("a1".into()));
        assert_eq!(ev("if(0, 1, 2)").expect("eval"), Value::Num(2.0));
        assert_eq!(ev("7 % 3 + 1/0").expect("eval"), Value::Num(1.0));
        assert_eq!(ev("strreplace(\"a_b\", \"_\", \"-\")").expect("eval"), Value::Str("a-b".into()));
    }
    #[test]
    fn geometry_queries_and_python_are_unsupported_not_errors() {
        for e in [
            "detail(0, \"x\", 0)",
            "$N",
            "@ropstart",
            "rand(3)",
            "point(\"../a\",0,\"P\",0)",
            "hasdetailattrib(0, 'r')",
        ] {
            assert!(matches!(ev(e), Err(ExprError::Unsupported(_))), "{e}");
        }
    }
    #[test]
    fn hostile_expressions_are_errors() {
        for e in ["", "(", "1 +", "1 ? 2 : 3", "ch(", "\"abc", "1 2", "$", "${F", "#"] {
            assert!(matches!(ev(e), Err(ExprError::Syntax(_))), "{e:?}");
        }
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert!(matches!(ev(&deep), Err(ExprError::Syntax(_))));
        let neg = "-".repeat(10_000) + "1";
        assert!(matches!(ev(&neg), Err(ExprError::Syntax(_))));
        assert!(matches!(ev(&"1+".repeat(40_000)), Err(ExprError::Syntax(_))));
        assert!(ev("2^2^2^2^2^2^2^2^2^2").is_ok());
    }
}
