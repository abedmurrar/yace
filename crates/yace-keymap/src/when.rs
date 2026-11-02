//! `when` clauses: boolean expressions over context keys, VS Code syntax
//! subset — identifiers, `!`, `&&`, `||` and parentheses.

use std::collections::HashSet;

use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum When {
    #[default]
    Always,
    Key(String),
    Not(Box<When>),
    And(Vec<When>),
    Or(Vec<When>),
}

/// Something that can answer "is context key X true?".
pub trait Context {
    fn is_set(&self, key: &str) -> bool;
}

/// A simple set of true context keys.
#[derive(Clone, Debug, Default)]
pub struct ContextSet(HashSet<String>);

impl ContextSet {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, key: &str, value: bool) {
        if value {
            self.0.insert(key.to_string());
        } else {
            self.0.remove(key);
        }
    }
    pub fn with(mut self, key: &str) -> Self {
        self.set(key, true);
        self
    }
}

impl Context for ContextSet {
    fn is_set(&self, key: &str) -> bool {
        self.0.contains(key)
    }
}

impl When {
    pub fn eval(&self, ctx: &dyn Context) -> bool {
        match self {
            When::Always => true,
            When::Key(k) => ctx.is_set(k),
            When::Not(w) => !w.eval(ctx),
            When::And(ws) => ws.iter().all(|w| w.eval(ctx)),
            When::Or(ws) => ws.iter().any(|w| w.eval(ctx)),
        }
    }

    pub fn parse(s: &str) -> Result<When, WhenError> {
        let tokens = tokenize(s)?;
        if tokens.is_empty() {
            return Ok(When::Always);
        }
        let mut p = Parser { tokens, pos: 0 };
        let w = p.or()?;
        if p.pos != p.tokens.len() {
            return Err(WhenError::Unexpected(format!("{:?}", p.tokens[p.pos])));
        }
        Ok(w)
    }
}

impl std::fmt::Display for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            When::Always => Ok(()),
            When::Key(k) => f.write_str(k),
            When::Not(w) => match **w {
                When::Key(_) => write!(f, "!{w}"),
                _ => write!(f, "!({w})"),
            },
            When::And(ws) => join(f, ws, " && "),
            When::Or(ws) => join(f, ws, " || "),
        }
    }
}

fn join(f: &mut std::fmt::Formatter<'_>, ws: &[When], sep: &str) -> std::fmt::Result {
    for (i, w) in ws.iter().enumerate() {
        if i > 0 {
            f.write_str(sep)?;
        }
        match w {
            When::And(_) | When::Or(_) => write!(f, "({w})")?,
            _ => write!(f, "{w}")?,
        }
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WhenError {
    #[error("unexpected character `{0}` in when clause")]
    BadChar(char),
    #[error("unexpected token {0} in when clause")]
    Unexpected(String),
    #[error("unexpected end of when clause")]
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Not,
    And,
    Or,
    LParen,
    RParen,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, WhenError> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' => {
                chars.next();
            }
            '!' => {
                chars.next();
                out.push(Tok::Not);
            }
            '(' => {
                chars.next();
                out.push(Tok::LParen);
            }
            ')' => {
                chars.next();
                out.push(Tok::RParen);
            }
            '&' | '|' => {
                chars.next();
                if chars.next() != Some(c) {
                    return Err(WhenError::BadChar(c));
                }
                out.push(if c == '&' { Tok::And } else { Tok::Or });
            }
            c if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' => {
                let mut id = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' {
                        id.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Ident(id));
            }
            other => return Err(WhenError::BadChar(other)),
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }
    fn or(&mut self) -> Result<When, WhenError> {
        let mut terms = vec![self.and()?];
        while self.peek() == Some(&Tok::Or) {
            self.pos += 1;
            terms.push(self.and()?);
        }
        Ok(if terms.len() == 1 { terms.pop().unwrap() } else { When::Or(terms) })
    }
    fn and(&mut self) -> Result<When, WhenError> {
        let mut terms = vec![self.unary()?];
        while self.peek() == Some(&Tok::And) {
            self.pos += 1;
            terms.push(self.unary()?);
        }
        Ok(if terms.len() == 1 { terms.pop().unwrap() } else { When::And(terms) })
    }
    fn unary(&mut self) -> Result<When, WhenError> {
        match self.tokens.get(self.pos).cloned() {
            Some(Tok::Not) => {
                self.pos += 1;
                Ok(When::Not(Box::new(self.unary()?)))
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let w = self.or()?;
                if self.peek() != Some(&Tok::RParen) {
                    return Err(WhenError::Eof);
                }
                self.pos += 1;
                Ok(w)
            }
            Some(Tok::Ident(id)) => {
                self.pos += 1;
                Ok(match id.as_str() {
                    "true" => When::Always,
                    "false" => When::Not(Box::new(When::Always)),
                    _ => When::Key(id),
                })
            }
            Some(t) => Err(WhenError::Unexpected(format!("{t:?}"))),
            None => Err(WhenError::Eof),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_eval() {
        let w = When::parse("editorFocus && !editorHasSelection || explorerFocus").unwrap();
        let ctx = ContextSet::new().with("editorFocus");
        assert!(w.eval(&ctx));
        let ctx2 = ContextSet::new().with("editorFocus").with("editorHasSelection");
        assert!(!w.eval(&ctx2));
        assert!(w.eval(&ContextSet::new().with("explorerFocus")));
        let p = When::parse("!(a || b) && c").unwrap();
        assert!(p.eval(&ContextSet::new().with("c")));
        assert!(!p.eval(&ContextSet::new().with("c").with("a")));
        assert_eq!(When::parse("").unwrap(), When::Always);
        assert!(When::parse("a &").is_err());
        assert!(When::parse("(a").is_err());
        assert_eq!(When::parse("a && !b").unwrap().to_string(), "a && !b");
    }
}
