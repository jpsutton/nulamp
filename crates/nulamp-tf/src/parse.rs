//! Parser: format string → syntax tree.
//!
//! Syntax errors are confined to the innermost function argument or `[ ]` section whose
//! closing delimiter can still be found; that part becomes [`Node::Error`]. When no such
//! part exists (for example an unmatched `(` or `'`), the whole tree is one error.
//!
//! An apostrophe between two alphanumeric characters (`don't`) is literal and does not
//! start a quote; spec 12 §2.3 requires `$caps(don't stop)` to work.

use crate::funcs::Func;

/// Arguments after this many are parsed but ignored.
pub(crate) const MAX_ARGS: usize = 64;

/// Calls and sections nested deeper than this are syntax errors.
pub(crate) const MAX_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Text(String),
    /// Lower-case field name.
    Field(String),
    Section(Vec<Node>),
    Call(Func, Vec<Vec<Node>>),
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Top,
    Section,
    Arg,
}

/// A syntax error; the parser position marks where recovery resumes.
struct Invalid;

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    depth: usize,
}

pub(crate) fn parse(s: &str) -> Vec<Node> {
    let mut p = Parser {
        s,
        b: s.as_bytes(),
        pos: 0,
        depth: 0,
    };
    p.seq(Scope::Top)
        .unwrap_or_else(|Invalid| vec![Node::Error])
}

fn is_special(b: u8) -> bool {
    matches!(b, b'%' | b'$' | b'[' | b']' | b'(' | b')' | b'\'' | b',')
}

fn push_text(nodes: &mut Vec<Node>, t: &str) {
    if let Some(Node::Text(prev)) = nodes.last_mut() {
        prev.push_str(t);
    } else {
        nodes.push(Node::Text(t.to_string()));
    }
}

impl Parser<'_> {
    fn peek(&self, off: usize) -> Option<u8> {
        self.b.get(self.pos + off).copied()
    }

    /// Parses up to the scope's terminator (not consumed): `]` for a section, `,` or `)`
    /// for an argument, the end for the top level.
    fn seq(&mut self, scope: Scope) -> Result<Vec<Node>, Invalid> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek(0) {
            match c {
                b'%' if self.peek(1) == Some(b'%') => {
                    push_text(&mut nodes, "%");
                    self.pos += 2;
                }
                b'%' => {
                    let name = self.field(scope)?;
                    nodes.push(Node::Field(name));
                }
                b'$' if self.peek(1) == Some(b'$') => {
                    push_text(&mut nodes, "$");
                    self.pos += 2;
                }
                b'$' => {
                    let call = self.call()?;
                    nodes.push(call);
                }
                b'[' => {
                    let section = self.section()?;
                    nodes.push(section);
                }
                b']' if scope == Scope::Section => return Ok(nodes),
                b')' | b',' if scope == Scope::Arg => return Ok(nodes),
                b']' | b')' => {
                    self.pos += 1;
                    return Err(Invalid);
                }
                b'(' => return Err(Invalid),
                b'\'' => {
                    let t = self.quote()?;
                    push_text(&mut nodes, t);
                }
                _ => {
                    let start = self.pos;
                    self.pos += 1;
                    while self.peek(0).is_some_and(|b| !is_special(b)) {
                        self.pos += 1;
                    }
                    push_text(&mut nodes, &self.s[start..self.pos]);
                }
            }
        }
        if scope == Scope::Top {
            Ok(nodes)
        } else {
            Err(Invalid)
        }
    }

    /// Parses a scope, replacing it with [`Node::Error`] when it is malformed but its end
    /// can be found.
    fn scope(&mut self, scope: Scope) -> Result<Vec<Node>, Invalid> {
        match self.seq(scope) {
            Ok(nodes) => Ok(nodes),
            Err(Invalid) if self.skip(scope) => Ok(vec![Node::Error]),
            Err(Invalid) => Err(Invalid),
        }
    }

    /// `%name%`. A name ends at the next `%` and cannot contain characters that are special
    /// in the current scope.
    fn field(&mut self, scope: Scope) -> Result<String, Invalid> {
        let start = self.pos + 1;
        for (i, &b) in self.b[start..].iter().enumerate() {
            match b {
                b'%' => {
                    self.pos = start + i + 1;
                    return Ok(self.s[start..start + i].to_lowercase());
                }
                b'(' | b')' | b'[' | b']' | b'\'' => break,
                b',' if scope == Scope::Arg => break,
                _ => {}
            }
        }
        self.pos = start;
        Err(Invalid)
    }

    /// `'text'`, `''`, or a literal apostrophe inside a word.
    fn quote(&mut self) -> Result<&str, Invalid> {
        if self.peek(1) == Some(b'\'') {
            self.pos += 2;
            return Ok("'");
        }
        if self.in_word() {
            self.pos += 1;
            return Ok("'");
        }
        let start = self.pos + 1;
        match self.b[start..].iter().position(|&b| b == b'\'') {
            Some(i) => {
                self.pos = start + i + 1;
                Ok(&self.s[start..start + i])
            }
            None => {
                self.pos = self.b.len();
                Err(Invalid)
            }
        }
    }

    /// The apostrophe at the current position sits between two alphanumeric characters.
    fn in_word(&self) -> bool {
        let before = self.s[..self.pos].chars().next_back();
        let after = self.s[self.pos + 1..].chars().next();
        before.is_some_and(char::is_alphanumeric) && after.is_some_and(char::is_alphanumeric)
    }

    /// `$name(args)`.
    fn call(&mut self) -> Result<Node, Invalid> {
        self.pos += 1;
        let start = self.pos;
        while self
            .peek(0)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            self.pos += 1;
        }
        if self.pos == start || self.peek(0) != Some(b'(') || self.depth >= MAX_DEPTH {
            return Err(Invalid);
        }
        let func = Func::from_name(&self.s[start..self.pos].to_ascii_lowercase());
        self.pos += 1;
        self.depth += 1;
        let args = self.args();
        self.depth -= 1;
        Ok(Node::Call(func, args?))
    }

    fn args(&mut self) -> Result<Vec<Vec<Node>>, Invalid> {
        let mut args = Vec::new();
        if self.peek(0) == Some(b')') {
            self.pos += 1;
            return Ok(args);
        }
        loop {
            let arg = self.scope(Scope::Arg)?;
            if args.len() < MAX_ARGS {
                args.push(arg);
            }
            let end = self.peek(0);
            self.pos += 1;
            if end == Some(b')') {
                return Ok(args);
            }
        }
    }

    /// `[ … ]`. A malformed section becomes a single [`Node::Error`].
    fn section(&mut self) -> Result<Node, Invalid> {
        if self.depth >= MAX_DEPTH {
            return Err(Invalid);
        }
        self.pos += 1;
        self.depth += 1;
        let inner = self.scope(Scope::Section);
        self.depth -= 1;
        let inner = inner?;
        self.pos += 1;
        Ok(match inner.as_slice() {
            [Node::Error] => Node::Error,
            _ => Node::Section(inner),
        })
    }

    /// Skips balanced text up to the scope's terminator (left unconsumed). Returns `false`,
    /// leaving the position at the offending character, when the scope cannot be closed.
    fn skip(&mut self, scope: Scope) -> bool {
        if scope == Scope::Top {
            return false;
        }
        let mut open: Vec<u8> = Vec::new();
        while let Some(c) = self.peek(0) {
            match c {
                b'\'' => {
                    if self.quote().is_err() {
                        return false;
                    }
                    continue;
                }
                b'(' | b'[' => open.push(c),
                b')' | b']' => match open.last() {
                    Some(&o) if (o, c) == (b'(', b')') || (o, c) == (b'[', b']') => {
                        open.pop();
                    }
                    Some(_) => return false,
                    None => {
                        return (scope == Scope::Arg && c == b')')
                            || (scope == Scope::Section && c == b']');
                    }
                },
                b',' if open.is_empty() && scope == Scope::Arg => return true,
                _ => {}
            }
            self.pos += 1;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Node {
        Node::Text(s.to_string())
    }

    #[test]
    fn basic_tree() {
        assert_eq!(
            parse("a%B%[c]$x(1,)"),
            vec![
                text("a"),
                Node::Field("b".into()),
                Node::Section(vec![text("c")]),
                Node::Call(Func::Unknown, vec![vec![text("1")], vec![]]),
            ]
        );
        assert_eq!(parse("$len()"), vec![Node::Call(Func::Len, vec![])]);
        assert_eq!(parse("%%$$'''(,)'"), vec![text("%$'(,)")]);
        assert_eq!(parse("a,b"), vec![text("a,b")]);
        assert_eq!(parse("don't"), vec![text("don't")]);
    }

    #[test]
    fn whole_string_errors() {
        for s in [
            "%a", "'abc", "[abc", "$len(a", "a)", "a]", "(a)", "$len", "$", "$(a)",
        ] {
            assert_eq!(parse(s), vec![Node::Error], "{s}");
        }
    }

    #[test]
    fn confined_errors() {
        assert_eq!(parse("[%a]b"), vec![Node::Error, text("b")]);
        assert_eq!(parse("[a)b]c"), vec![Node::Error, text("c")]);
        assert_eq!(
            parse("$len(%a,x(y)z)"),
            vec![Node::Call(
                Func::Len,
                vec![vec![Node::Error], vec![Node::Error]]
            )]
        );
        assert_eq!(
            parse("$len([a)b)"),
            vec![Node::Call(Func::Len, vec![vec![Node::Error]])]
        );
        assert_eq!(parse("$len(a]b,c)"), parse("$len(%x,c)"));
    }

    #[test]
    fn argument_limit() {
        let s = format!("$len({})", vec!["x"; 100].join(","));
        let nodes = parse(&s);
        let [Node::Call(_, args)] = nodes.as_slice() else {
            panic!("{nodes:?}")
        };
        assert_eq!(args.len(), MAX_ARGS);
    }

    #[test]
    fn depth_limit() {
        let ok = format!("{}{}", "$len(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
        assert!(!format!("{:?}", parse(&ok)).contains("Error"));
        let deep = format!(
            "{}{}",
            "$len(".repeat(MAX_DEPTH + 1),
            ")".repeat(MAX_DEPTH + 1)
        );
        assert!(format!("{:?}", parse(&deep)).contains("Error"));
        let huge = "[$len(".repeat(100_000);
        assert_eq!(parse(&huge), vec![Node::Error]);
    }
}
