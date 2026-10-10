//! The expression language: parser and evaluator.
//!
//! A program is a list of expressions separated by `;`. Values are 64-bit floats; comparisons
//! and logic give 1 or 0, and any non-zero value is true. Variables spring into existence at 0
//! on first use and keep their value between runs. `buf[i]` is a shared array of
//! [`BUF_LEN`] values (the index is floored and wraps).
//!
//! ```text
//! expr    := assign
//! assign  := (ident | ident '[' expr ']') ('=' | '+=' | '-=' | '*=' | '/=') assign | cond
//! cond    := or ('?' expr ':' cond)?
//! or      := and ('||' and)*          and := cmp ('&&' cmp)*
//! cmp     := sum (('=='|'!='|'<'|'<='|'>'|'>=') sum)?
//! sum     := prod (('+'|'-') prod)*   prod := unary (('*'|'/'|'%') unary)*
//! unary   := ('-'|'!'|'+') unary | pow
//! pow     := atom ('^' unary)?
//! atom    := number | ident | ident '(' args ')' | ident '[' expr ']' | '(' expr ')'
//! ```

use std::collections::HashMap;

/// Length of the `buf` array.
pub const BUF_LEN: usize = 65_536;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sqrt,
    Pow,
    Exp,
    Log,
    Log10,
    Abs,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Sign,
    Clamp,
    Rand,
}

impl Func {
    fn by_name(name: &str) -> Option<(Self, usize)> {
        use Func::*;
        Some(match name {
            "sin" => (Sin, 1),
            "cos" => (Cos, 1),
            "tan" => (Tan, 1),
            "asin" => (Asin, 1),
            "acos" => (Acos, 1),
            "atan" => (Atan, 1),
            "atan2" => (Atan2, 2),
            "sqrt" => (Sqrt, 1),
            "pow" => (Pow, 2),
            "exp" => (Exp, 1),
            "log" => (Log, 1),
            "log10" => (Log10, 1),
            "abs" => (Abs, 1),
            "min" => (Min, 2),
            "max" => (Max, 2),
            "floor" => (Floor, 1),
            "ceil" => (Ceil, 1),
            "round" => (Round, 1),
            "sign" => (Sign, 1),
            "clamp" => (Clamp, 3),
            "rand" => (Rand, 1),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Num(f64),
    Var(usize),
    Buf(Box<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
    SetVar(usize, Option<Op>, Box<Expr>),
    SetBuf(Box<Expr>, Option<Op>, Box<Expr>),
}

/// Variable names and their slots, shared by every section of a script.
#[derive(Debug, Default, Clone)]
pub struct Symbols {
    names: HashMap<String, usize>,
}

impl Symbols {
    pub fn slot(&mut self, name: &str) -> usize {
        let n = self.names.len();
        *self.names.entry(name.to_string()).or_insert(n)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// A compiled section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Program(Vec<Expr>);

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(&'static str),
}

fn lex(src: &str, first_line: usize) -> Result<Vec<(Tok, usize)>, ParseError> {
    const SYMS: [&str; 26] = [
        "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "+", "-", "*", "/", "%", "^",
        "<", ">", "=", "!", "?", ":", "(", ")", "[", "]",
    ];
    let b = src.as_bytes();
    let (mut i, mut line) = (0, first_line);
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            let end = src[i + 2..].find("*/").ok_or(ParseError {
                line,
                message: "unclosed comment".into(),
            })?;
            line += src[i..i + 2 + end].matches('\n').count();
            i += end + 4;
        } else if c == b';' || c == b',' {
            out.push((Tok::Sym(if c == b';' { ";" } else { "," }), line));
            i += 1;
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    i = j;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let v = src[start..i].parse().map_err(|_| ParseError {
                line,
                message: format!("bad number {}", &src[start..i]),
            })?;
            out.push((Tok::Num(v), line));
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Ident(src[start..i].to_ascii_lowercase()), line));
        } else if let Some(s) = SYMS.iter().find(|s| src[i..].starts_with(**s)) {
            out.push((Tok::Sym(s), line));
            i += s.len();
        } else {
            return Err(ParseError {
                line,
                message: format!(
                    "unexpected character {:?}",
                    src[i..].chars().next().unwrap()
                ),
            });
        }
    }
    Ok(out)
}

struct Parser<'a> {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    syms: &'a mut Symbols,
    last_line: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).map_or(self.last_line, |t| t.1)
    }

    fn err<T>(&self, message: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            line: self.line(),
            message: message.into(),
        })
    }

    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(t)) if *t == s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<(), ParseError> {
        if self.eat(s) {
            Ok(())
        } else {
            self.err(format!("expected `{s}`"))
        }
    }

    fn assign_op(&mut self) -> Option<Option<Op>> {
        let op = match self.peek() {
            Some(Tok::Sym("=")) => None,
            Some(Tok::Sym("+=")) => Some(Op::Add),
            Some(Tok::Sym("-=")) => Some(Op::Sub),
            Some(Tok::Sym("*=")) => Some(Op::Mul),
            Some(Tok::Sym("/=")) => Some(Op::Div),
            _ => return None,
        };
        self.pos += 1;
        Some(op)
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        if let Some(Tok::Ident(name)) = self.peek().cloned() {
            self.pos += 1;
            if name == "buf" && self.eat("[") {
                let index = self.expr()?;
                self.expect("]")?;
                if let Some(op) = self.assign_op() {
                    let value = self.expr()?;
                    return Ok(Expr::SetBuf(Box::new(index), op, Box::new(value)));
                }
            } else if Func::by_name(&name).is_none()
                && let Some(op) = self.assign_op()
            {
                let slot = self.syms.slot(&name);
                let value = self.expr()?;
                return Ok(Expr::SetVar(slot, op, Box::new(value)));
            }
            self.pos = start;
        }
        self.cond()
    }

    fn cond(&mut self) -> Result<Expr, ParseError> {
        let c = self.or()?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.cond()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(
        &mut self,
        ops: &[(&str, Op)],
        next: fn(&mut Self) -> Result<Expr, ParseError>,
        once: bool,
    ) -> Result<Expr, ParseError> {
        let mut left = next(self)?;
        'outer: loop {
            for (s, op) in ops {
                if self.eat(s) {
                    let right = next(self)?;
                    left = Expr::Bin(*op, Box::new(left), Box::new(right));
                    if once {
                        break 'outer;
                    }
                    continue 'outer;
                }
            }
            break;
        }
        Ok(left)
    }

    fn or(&mut self) -> Result<Expr, ParseError> {
        self.binary(&[("||", Op::Or)], Self::and, false)
    }

    fn and(&mut self) -> Result<Expr, ParseError> {
        self.binary(&[("&&", Op::And)], Self::cmp, false)
    }

    fn cmp(&mut self) -> Result<Expr, ParseError> {
        self.binary(
            &[
                ("==", Op::Eq),
                ("!=", Op::Ne),
                ("<=", Op::Le),
                (">=", Op::Ge),
                ("<", Op::Lt),
                (">", Op::Gt),
            ],
            Self::sum,
            true,
        )
    }

    fn sum(&mut self) -> Result<Expr, ParseError> {
        self.binary(&[("+", Op::Add), ("-", Op::Sub)], Self::prod, false)
    }

    fn prod(&mut self) -> Result<Expr, ParseError> {
        self.binary(
            &[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)],
            Self::unary,
            false,
        )
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat("-") {
            return Ok(Expr::Neg(Box::new(self.unary()?)));
        }
        if self.eat("!") {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        let base = self.atom()?;
        if self.eat("^") {
            let exp = self.unary()?;
            return Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.pos += 1;
                Ok(Expr::Num(v))
            }
            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                if name == "buf" && self.eat("[") {
                    let index = self.expr()?;
                    self.expect("]")?;
                    return Ok(Expr::Buf(Box::new(index)));
                }
                if self.eat("(") {
                    let Some((f, arity)) = Func::by_name(&name) else {
                        return self.err(format!("unknown function {name}"));
                    };
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.expr()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    if args.len() != arity {
                        return self.err(format!("{name} takes {arity} argument(s)"));
                    }
                    return Ok(Expr::Call(f, args));
                }
                if name == "pi" {
                    return Ok(Expr::Num(std::f64::consts::PI));
                }
                Ok(Expr::Var(self.syms.slot(&name)))
            }
            Some(t) => self.err(format!("unexpected {t:?}")),
            None => self.err("unexpected end"),
        }
    }
}

/// Compiles one section. `first_line` numbers error messages.
pub fn compile(src: &str, syms: &mut Symbols, first_line: usize) -> Result<Program, ParseError> {
    let toks = lex(src, first_line)?;
    let last_line = toks.last().map_or(first_line, |t| t.1);
    let mut p = Parser {
        toks,
        pos: 0,
        syms,
        last_line,
    };
    let mut out = Vec::new();
    while p.peek().is_some() {
        if p.eat(";") {
            continue;
        }
        out.push(p.expr()?);
        if p.peek().is_some() {
            p.expect(";")?;
        }
    }
    Ok(Program(out))
}

/// Variables and `buf` of a running script.
#[derive(Debug, Clone)]
pub struct Machine {
    pub vars: Vec<f64>,
    buf: Vec<f64>,
    seed: u64,
}

impl Machine {
    pub fn new(slots: usize) -> Self {
        Self {
            vars: vec![0.0; slots],
            buf: vec![0.0; BUF_LEN],
            seed: 0x2545_f491_4f6c_dd1d,
        }
    }

    /// Zeroes variables and `buf`.
    pub fn clear(&mut self) {
        self.vars.iter_mut().for_each(|v| *v = 0.0);
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }

    pub fn run(&mut self, p: &Program) {
        for e in &p.0 {
            self.eval(e);
        }
    }

    fn index(v: f64) -> usize {
        if v.is_finite() {
            (v.floor() as i64).rem_euclid(BUF_LEN as i64) as usize
        } else {
            0
        }
    }

    fn apply(op: Op, a: f64, b: f64) -> f64 {
        let t = |c: bool| if c { 1.0 } else { 0.0 };
        match op {
            Op::Add => a + b,
            Op::Sub => a - b,
            Op::Mul => a * b,
            Op::Div => {
                if b == 0.0 {
                    0.0
                } else {
                    a / b
                }
            }
            Op::Rem => {
                if b == 0.0 {
                    0.0
                } else {
                    a % b
                }
            }
            Op::Pow => a.powf(b),
            Op::Eq => t(a == b),
            Op::Ne => t(a != b),
            Op::Lt => t(a < b),
            Op::Le => t(a <= b),
            Op::Gt => t(a > b),
            Op::Ge => t(a >= b),
            Op::And | Op::Or => unreachable!("short-circuit"),
        }
    }

    fn eval(&mut self, e: &Expr) -> f64 {
        match e {
            Expr::Num(v) => *v,
            Expr::Var(s) => self.vars[*s],
            Expr::Buf(i) => {
                let i = Self::index(self.eval(i));
                self.buf[i]
            }
            Expr::Neg(a) => -self.eval(a),
            Expr::Not(a) => (self.eval(a) == 0.0) as u8 as f64,
            Expr::Bin(Op::And, a, b) => (self.eval(a) != 0.0 && self.eval(b) != 0.0) as u8 as f64,
            Expr::Bin(Op::Or, a, b) => (self.eval(a) != 0.0 || self.eval(b) != 0.0) as u8 as f64,
            Expr::Bin(op, a, b) => {
                let (a, b) = (self.eval(a), self.eval(b));
                Self::apply(*op, a, b)
            }
            Expr::Cond(c, a, b) => {
                if self.eval(c) != 0.0 {
                    self.eval(a)
                } else {
                    self.eval(b)
                }
            }
            Expr::Call(f, args) => {
                let x = |m: &mut Self, i: usize| m.eval(&args[i]);
                use Func::*;
                match f {
                    Sin => x(self, 0).sin(),
                    Cos => x(self, 0).cos(),
                    Tan => x(self, 0).tan(),
                    Asin => x(self, 0).asin(),
                    Acos => x(self, 0).acos(),
                    Atan => x(self, 0).atan(),
                    Atan2 => {
                        let y = x(self, 0);
                        y.atan2(x(self, 1))
                    }
                    Sqrt => x(self, 0).max(0.0).sqrt(),
                    Pow => {
                        let a = x(self, 0);
                        a.powf(x(self, 1))
                    }
                    Exp => x(self, 0).exp(),
                    Log => x(self, 0).ln(),
                    Log10 => x(self, 0).log10(),
                    Abs => x(self, 0).abs(),
                    Min => {
                        let a = x(self, 0);
                        a.min(x(self, 1))
                    }
                    Max => {
                        let a = x(self, 0);
                        a.max(x(self, 1))
                    }
                    Floor => x(self, 0).floor(),
                    Ceil => x(self, 0).ceil(),
                    Round => x(self, 0).round(),
                    Sign => {
                        let a = x(self, 0);
                        if a > 0.0 {
                            1.0
                        } else if a < 0.0 {
                            -1.0
                        } else {
                            0.0
                        }
                    }
                    Clamp => {
                        let (v, lo, hi) = (x(self, 0), x(self, 1), x(self, 2));
                        v.max(lo).min(hi)
                    }
                    Rand => {
                        let n = x(self, 0);
                        // xorshift64*
                        self.seed ^= self.seed >> 12;
                        self.seed ^= self.seed << 25;
                        self.seed ^= self.seed >> 27;
                        let r = self.seed.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11;
                        r as f64 / (1u64 << 53) as f64 * n
                    }
                }
            }
            Expr::SetVar(s, op, v) => {
                let v = self.eval(v);
                let new = match op {
                    None => v,
                    Some(op) => Self::apply(*op, self.vars[*s], v),
                };
                self.vars[*s] = new;
                new
            }
            Expr::SetBuf(i, op, v) => {
                let i = Self::index(self.eval(i));
                let v = self.eval(v);
                let new = match op {
                    None => v,
                    Some(op) => Self::apply(*op, self.buf[i], v),
                };
                self.buf[i] = new;
                new
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> (Symbols, Machine) {
        let mut syms = Symbols::default();
        let p = compile(src, &mut syms, 1).unwrap();
        let mut m = Machine::new(syms.len());
        m.run(&p);
        (syms, m)
    }

    fn var(src: &str, name: &str) -> f64 {
        let (mut syms, m) = run(src);
        m.vars[syms.slot(name)]
    }

    #[test]
    fn precedence_and_operators() {
        assert_eq!(var("x = 1 + 2 * 3 - 4 / 2", "x"), 5.0);
        assert_eq!(var("x = -2 ^ 2", "x"), -4.0);
        assert_eq!(var("x = 2 ^ 3 ^ 2", "x"), 512.0);
        assert_eq!(var("x = 7 % 3 + (1 < 2) + (2 <= 1)", "x"), 2.0);
        assert_eq!(var("x = 1 && 0 || !0", "x"), 1.0);
        assert_eq!(var("x = 0 ? 5 : 1 ? 6 : 7", "x"), 6.0);
        assert_eq!(var("x = 3; x += 2; x *= 4; x -= 1; x /= 19", "x"), 1.0);
        assert_eq!(var("a = b = 4; x = a + b", "x"), 8.0);
        assert_eq!(var("x = 1 / 0", "x"), 0.0);
        assert_eq!(
            var("x = clamp(5, 0, 2) + max(1, 3) + abs(-1) + sign(-3)", "x"),
            5.0
        );
        assert!((var("x = sin(pi / 2)", "x") - 1.0).abs() < 1e-12);
        assert_eq!(var("X = 2; x = x * 3", "x"), 6.0);
        assert_eq!(var("x = 1.5e2 + .5", "x"), 150.5);
    }

    #[test]
    fn buffer_and_comments() {
        let src = "buf[3] = 7; // a comment\n/* block\ncomment */ buf[3] += 1; x = buf[65539] + buf[-65533]";
        assert_eq!(var(src, "x"), 16.0);
    }

    #[test]
    fn rand_is_in_range() {
        let (mut syms, mut m) = run("x = 0");
        let p = compile("r = rand(10)", &mut syms, 1).unwrap();
        m.vars.resize(syms.len(), 0.0);
        for _ in 0..1000 {
            m.run(&p);
            let r = m.vars[syms.slot("r")];
            assert!((0.0..10.0).contains(&r));
        }
    }

    #[test]
    fn errors_have_lines() {
        let mut syms = Symbols::default();
        let e = compile("x = 1;\ny = (2 +;", &mut syms, 10).unwrap_err();
        assert_eq!(e.line, 11);
        assert!(compile("x = foo(1)", &mut syms, 1).is_err());
        assert!(compile("x = sin(1, 2)", &mut syms, 1).is_err());
        assert!(compile("x = 1 y = 2", &mut syms, 1).is_err());
        assert!(compile("x = @", &mut syms, 1).is_err());
    }
}
