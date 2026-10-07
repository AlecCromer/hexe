//! Lexer: source text → positioned tokens. Handles `//` and `/* */` comments, `@name` / `i@name` attribute refs,
//! int/float/string literals and the punctuation of the subset.
use crate::{Ty, VexError};

/// Source cap in bytes (hostile input).
pub const MAX_SRC: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    /// `@name` (prefix `None`) or `i@name` etc.
    Attr {
        prefix: Option<Ty>,
        name: String,
    },
    /// Punctuation / operator.
    P(&'static str),
    Eof,
}

impl Tok {
    /// Human text for error messages.
    pub fn show(&self) -> String {
        match self {
            Tok::Int(i) => i.to_string(),
            Tok::Float(f) => f.to_string(),
            Tok::Str(s) => format!("'{s}'"),
            Tok::Ident(s) => s.clone(),
            Tok::Attr { name, .. } => format!("@{name}"),
            Tok::P(p) => format!("`{p}`"),
            Tok::Eof => "end of input".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

const TWO: [&str; 12] = ["==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "++", "--"];
const ONE: [&str; 22] =
    ["+", "-", "*", "/", "%", "<", ">", "=", "!", "(", ")", "{", "}", ";", ",", ".", "?", ":", "[", "]", "^", "~"];

fn is_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

struct Lx {
    c: Vec<char>,
    i: usize,
    line: u32,
    col: u32,
}

impl Lx {
    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i.saturating_add(k)).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.c.get(self.i).copied()?;
        self.i += 1;
        if ch == '\n' {
            self.line = self.line.saturating_add(1);
            self.col = 1;
        } else {
            self.col = self.col.saturating_add(1);
        }
        Some(ch)
    }
    fn err<T>(&self, line: u32, col: u32, m: impl Into<String>) -> Result<T, VexError> {
        Err(VexError::parse(line, col, m))
    }
    fn ident(&mut self) -> String {
        let mut s = String::new();
        while let Some(ch) = self.peek(0).filter(|c| is_cont(*c)) {
            s.push(ch);
            self.bump();
        }
        s
    }
    fn number(&mut self, line: u32, col: u32) -> Result<Tok, VexError> {
        let mut s = String::new();
        let mut float = false;
        while let Some(ch) = self.peek(0).filter(char::is_ascii_digit) {
            s.push(ch);
            self.bump();
        }
        if self.peek(0) == Some('.') && self.peek(1).is_none_or(|c| !is_start(c)) {
            float = true;
            s.push('.');
            self.bump();
            while let Some(ch) = self.peek(0).filter(char::is_ascii_digit) {
                s.push(ch);
                self.bump();
            }
        }
        if matches!(self.peek(0), Some('e' | 'E'))
            && (self.peek(1).is_some_and(|c| c.is_ascii_digit())
                || (matches!(self.peek(1), Some('+' | '-')) && self.peek(2).is_some_and(|c| c.is_ascii_digit())))
        {
            float = true;
            s.push('e');
            self.bump();
            if let Some(sg) = self.peek(0).filter(|c| matches!(c, '+' | '-')) {
                s.push(sg);
                self.bump();
            }
            while let Some(ch) = self.peek(0).filter(char::is_ascii_digit) {
                s.push(ch);
                self.bump();
            }
        }
        if s.starts_with('.') {
            s.insert(0, '0');
        }
        if float {
            match s.parse::<f64>() {
                Ok(f) => Ok(Tok::Float(f)),
                Err(_) => self.err(line, col, format!("bad number `{s}`")),
            }
        } else {
            match s.parse::<i64>() {
                Ok(i) => Ok(Tok::Int(i)),
                Err(_) => self.err(line, col, format!("integer literal `{s}` out of range")),
            }
        }
    }
    fn string(&mut self, q: char, line: u32, col: u32) -> Result<Tok, VexError> {
        self.bump();
        let mut s = String::new();
        loop {
            match self.bump() {
                None | Some('\n') => return self.err(line, col, "unterminated string literal"),
                Some(ch) if ch == q => return Ok(Tok::Str(s)),
                Some('\\') => match self.bump() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(ch @ ('\\' | '\'' | '"')) => s.push(ch),
                    _ => return self.err(self.line, self.col.saturating_sub(1), "bad escape in string"),
                },
                Some(ch) => s.push(ch),
            }
        }
    }
    fn attr_name(&mut self, prefix: Option<Ty>, line: u32, col: u32) -> Result<Tok, VexError> {
        let name = self.ident();
        if name.is_empty() {
            return self.err(line, col, "`@` must be followed by an attribute name");
        }
        Ok(Tok::Attr { prefix, name })
    }
}

/// Tokenize `src`. The last token is always `Eof`.
pub fn lex(src: &str) -> Result<Vec<Token>, VexError> {
    if src.len() > MAX_SRC {
        return Err(VexError::parse(1, 1, format!("snippet longer than {MAX_SRC} bytes")));
    }
    let mut lx = Lx { c: src.chars().collect(), i: 0, line: 1, col: 1 };
    let mut out = Vec::new();
    while let Some(ch) = lx.peek(0) {
        let (line, col) = (lx.line, lx.col);
        if ch.is_whitespace() {
            lx.bump();
            continue;
        }
        if ch == '/' && lx.peek(1) == Some('/') {
            while lx.peek(0).is_some_and(|c| c != '\n') {
                lx.bump();
            }
            continue;
        }
        if ch == '/' && lx.peek(1) == Some('*') {
            lx.bump();
            lx.bump();
            loop {
                match lx.bump() {
                    None => return lx.err(line, col, "unterminated /* comment"),
                    Some('*') if lx.peek(0) == Some('/') => {
                        lx.bump();
                        break;
                    }
                    Some(_) => {}
                }
            }
            continue;
        }
        let tok = if ch.is_ascii_digit() || (ch == '.' && lx.peek(1).is_some_and(|c| c.is_ascii_digit())) {
            lx.number(line, col)?
        } else if is_start(ch) {
            let id = lx.ident();
            if lx.peek(0) == Some('@') {
                match Ty::from_prefix(&id) {
                    Some(t) => {
                        lx.bump();
                        lx.attr_name(Some(t), line, col)?
                    }
                    None => return lx.err(line, col, format!("unknown attribute type prefix `{id}@`")),
                }
            } else {
                Tok::Ident(id)
            }
        } else if ch == '@' {
            lx.bump();
            lx.attr_name(None, line, col)?
        } else if ch == '\'' || ch == '"' {
            lx.string(ch, line, col)?
        } else {
            let two: String = [ch, lx.peek(1).unwrap_or(' ')].iter().collect();
            if let Some(p) = TWO.iter().find(|p| **p == two) {
                lx.bump();
                lx.bump();
                Tok::P(p)
            } else if let Some(p) = ONE.iter().find(|p| p.starts_with(ch)) {
                lx.bump();
                Tok::P(p)
            } else {
                return lx.err(line, col, format!("unexpected character `{ch}`"));
            }
        };
        out.push(Token { tok, line, col });
    }
    out.push(Token { tok: Tok::Eof, line: lx.line, col: lx.col });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn toks(s: &str) -> Vec<Tok> {
        lex(s).expect("lexes").into_iter().map(|t| t.tok).collect()
    }
    #[test]
    fn attrs_prefixed_and_bare() {
        assert_eq!(
            toks("@P f@h v@Cd i@id s@n @x"),
            vec![
                Tok::Attr { prefix: None, name: "P".into() },
                Tok::Attr { prefix: Some(Ty::Float), name: "h".into() },
                Tok::Attr { prefix: Some(Ty::Vec3), name: "Cd".into() },
                Tok::Attr { prefix: Some(Ty::Int), name: "id".into() },
                Tok::Attr { prefix: Some(Ty::Str), name: "n".into() },
                Tok::Attr { prefix: None, name: "x".into() },
                Tok::Eof
            ]
        );
    }
    #[test]
    fn numbers_and_member_dot() {
        assert_eq!(
            toks("1 2.5 .5 1e3 2."),
            vec![Tok::Int(1), Tok::Float(2.5), Tok::Float(0.5), Tok::Float(1000.0), Tok::Float(2.0), Tok::Eof]
        );
        assert_eq!(toks("@P.y")[1], Tok::P("."));
        assert_eq!(toks("a.x")[2], Tok::Ident("x".into()));
    }
    #[test]
    fn comments_strings_ops_positions() {
        let t = lex("// c\n/* a\nb */ x += 'q\\n';").expect("lexes");
        assert_eq!((t[0].line, t[0].col), (3, 6));
        assert_eq!(t[0].tok, Tok::Ident("x".into()));
        assert_eq!(t[1].tok, Tok::P("+="));
        assert_eq!(t[2].tok, Tok::Str("q\n".into()));
    }
    #[test]
    fn errors_carry_positions() {
        for (src, line, col) in [
            ("a\n  $", 2, 3),
            ("/* x", 1, 1),
            ("'abc", 1, 1),
            ("@ ", 1, 1),
            ("q@x", 1, 1),
            ("99999999999999999999", 1, 1),
        ] {
            let e = lex(src).expect_err(src);
            assert_eq!((e.line, e.col), (line, col), "{src}");
        }
        assert!(lex(&"a".repeat(MAX_SRC + 1)).is_err());
    }
}
