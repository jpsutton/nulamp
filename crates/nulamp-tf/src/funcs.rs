//! Built-in functions (spec 10 §5.3).
//!
//! Missing arguments are empty strings. Numbers are 32-bit signed: arguments are read
//! with saturation and arithmetic wraps.

use crate::eval::{State, Value};
use crate::path;
use crate::text::{self, count, int, len, str_eq};

const INVALID_IF: &str = "[INVALID $IF SYNTAX]";
const INVALID_IFLONGER: &str = "[INVALID $IFLONGER SYNTAX]";
const INVALID_IFGREATER: &str = "[INVALID $IFGREATER SYNTAX]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Func {
    If,
    If2,
    If3,
    Decode,
    Select,
    IfLonger,
    IfGreater,
    IfStrEqual,
    IfStrNotEqual,
    IfStrEqual2,
    Len,
    Longest,
    Shortest,
    Left,
    Right,
    Pad,
    LPad,
    PadCut,
    Repeat,
    Trim,
    Upper,
    Lower,
    Caps,
    Caps2,
    Abbr,
    StrChr,
    StrRChr,
    StrStr,
    Substr,
    Split,
    Replace,
    Char,
    Crlf,
    Tab,
    Num,
    Hex,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    MulDiv,
    Min,
    Max,
    FilePart,
    FileExt,
    Directory,
    PathLTrim,
    PathLPart,
    PathRTrim,
    PathRPart,
    PathSafe,
    Put,
    Puts,
    Get,
    Null,
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Unknown,
}

impl Func {
    /// Looks up a lower-case function name.
    pub(crate) fn from_name(name: &str) -> Self {
        use Func::*;
        match name {
            "if" => If,
            "if2" => If2,
            "if3" => If3,
            "decode" => Decode,
            "select" => Select,
            "iflonger" => IfLonger,
            "ifgreater" => IfGreater,
            "ifstrequal" => IfStrEqual,
            "ifstrnotequal" => IfStrNotEqual,
            "ifstrequal2" => IfStrEqual2,
            "len" => Len,
            "longest" => Longest,
            "shortest" => Shortest,
            "left" | "cut" => Left,
            "right" => Right,
            "pad" => Pad,
            "lpad" => LPad,
            "padcut" => PadCut,
            "repeat" => Repeat,
            "trim" => Trim,
            "upper" => Upper,
            "lower" => Lower,
            "caps" => Caps,
            "caps2" => Caps2,
            "abbr" => Abbr,
            "strchr" | "strlchr" => StrChr,
            "strrchr" => StrRChr,
            "strstr" => StrStr,
            "substr" => Substr,
            "split" => Split,
            "replace" => Replace,
            "char" => Char,
            "crlf" => Crlf,
            "tab" => Tab,
            "num" | "dec" => Num,
            "hex" => Hex,
            "add" => Add,
            "sub" => Sub,
            "mul" => Mul,
            "div" => Div,
            "mod" => Mod,
            "muldiv" => MulDiv,
            "min" => Min,
            "max" => Max,
            "filepart" | "filename" => FilePart,
            "fileext" | "ext" => FileExt,
            "directory" => Directory,
            "pathltrim" => PathLTrim,
            "pathlpart" => PathLPart,
            "pathrtrim" => PathRTrim,
            "pathrpart" => PathRPart,
            "pathsafe" => PathSafe,
            "put" => Put,
            "puts" => Puts,
            "get" => Get,
            "null" => Null,
            "systime_year" => Year,
            "systime_month" => Month,
            "systime_day" => Day,
            "systime_hour" => Hour,
            "systime_minute" => Minute,
            "systime_second" => Second,
            _ => Unknown,
        }
    }
}

/// Runs `func` on already evaluated arguments.
///
/// The returned `found` flag adds to the arguments' own; only `$get` sets it.
pub(crate) fn call(state: &mut State, func: Func, a: &[Value]) -> Value {
    let s = |i: usize| a.get(i).map_or("", |v| v.text.as_str());
    let n = |i: usize| int(s(i));
    let n_or = |i: usize, default: i32| if i < a.len() { n(i) } else { default };
    let true_at = |i: usize| a.get(i).is_some_and(|v| v.found);
    let pick = |i: usize| Value::text(s(i));
    let num = |v: i32| Value::text(v.to_string());
    let ints = || a.iter().map(|v| int(&v.text));

    use Func::*;
    match func {
        If if a.len() != 3 => Value::text(INVALID_IF),
        If => pick(if true_at(0) { 1 } else { 2 }),
        If2 if a.len() != 2 => Value::text(INVALID_IF),
        If2 => pick(if true_at(0) { 0 } else { 1 }),
        If3 => {
            let last = a.len().saturating_sub(1);
            pick((0..last).find(|&i| true_at(i)).unwrap_or(last))
        }
        Decode => {
            let v = s(0);
            for pair in a.get(1..).unwrap_or_default().chunks(2) {
                match pair {
                    [k, r] if str_eq(v, &k.text) => return Value::text(r.text.as_str()),
                    [default] => return Value::text(default.text.as_str()),
                    _ => {}
                }
            }
            Value::default()
        }
        Select => match n(0) {
            0 => Value::text("0"),
            i => usize::try_from(i).map_or_else(|_| Value::default(), pick),
        },
        IfLonger if a.len() != 4 => Value::text(INVALID_IFLONGER),
        IfLonger => {
            let longer = i64::try_from(len(s(0))).unwrap_or(i64::MAX) > i64::from(n(1));
            pick(if longer { 2 } else { 3 })
        }
        IfGreater if a.len() != 4 => Value::text(INVALID_IFGREATER),
        IfGreater => pick(if n(0) > n(1) { 2 } else { 3 }),
        IfStrEqual => Value::text(if str_eq(s(0), s(1)) { s(2) } else { "" }),
        IfStrNotEqual => Value::text(if str_eq(s(0), s(1)) { "" } else { s(2) }),
        IfStrEqual2 => pick(if str_eq(s(0), s(1)) { 2 } else { 3 }),

        Len => num(i32::try_from(len(s(0))).unwrap_or(i32::MAX)),
        Longest => extreme(a, |l, best| l > best),
        Shortest => extreme(a, |l, best| l < best),
        Left => Value::text(text::left(s(0), n(1))),
        Right => Value::text(text::right(s(0), n(1))),
        Pad => Value::text(text::pad(s(0), n(1), s(2), false)),
        LPad => Value::text(text::pad(s(0), n(1), s(2), true)),
        PadCut => Value::text(text::pad(&text::left(s(0), n(1)), n(1), s(2), false)),
        Repeat => Value::text(text::repeat(s(0), n(1))),
        Trim => Value::text(s(0).trim()),
        Upper => Value::text(s(0).to_uppercase()),
        Lower => Value::text(s(0).to_lowercase()),
        Caps => Value::text(text::caps(s(0), true)),
        Caps2 => Value::text(text::caps(s(0), false)),
        Abbr if a.len() >= 2 && i64::try_from(len(s(0))).unwrap_or(i64::MAX) < i64::from(n(1)) => {
            pick(0)
        }
        Abbr => Value::text(text::abbr(s(0))),
        StrChr | StrRChr => {
            let Some(c) = s(1).chars().next() else {
                return num(0);
            };
            let mut hits = s(0)
                .chars()
                .enumerate()
                .filter(|&(_, x)| x == c)
                .map(|(i, _)| i);
            let hit = if func == StrChr {
                hits.next()
            } else {
                hits.last()
            };
            num(hit.map_or(0, |i| i32::try_from(i + 1).unwrap_or(i32::MAX)))
        }
        StrStr => {
            let (hay, needle) = (s(0), s(1));
            let pos = match hay.find(needle) {
                Some(i) if !needle.is_empty() => len(&hay[..i]) + 1,
                _ => 0,
            };
            num(i32::try_from(pos).unwrap_or(i32::MAX))
        }
        Substr => Value::text(text::substr(s(0), n(1), n_or(2, n(1)))),
        Split => {
            let (hay, sep) = (s(0), s(1));
            let at = if sep.is_empty() { None } else { hay.find(sep) };
            match (at, n(2)) {
                (Some(i), 0) => Value::text(&hay[..i]),
                (None, 0) => pick(0),
                (Some(i), _) => Value::text(&hay[i + sep.len()..]),
                (None, _) => Value::default(),
            }
        }
        Replace => Value::text(text::replace(s(0), s(1), s(2))),
        Char => u32::try_from(n(0))
            .ok()
            .filter(|&c| c != 0)
            .and_then(char::from_u32)
            .map_or_else(Value::default, Value::text),
        Crlf => Value::text("\r\n"),
        Tab if a.is_empty() => Value::default(),
        Tab => Value::text(text::repeat("\t", n(0))),

        Num => {
            let w = count(n(1)).min(text::MAX_LEN);
            Value::text(format!("{:0w$}", n(0)))
        }
        Hex => {
            let w = count(n(1)).min(text::MAX_LEN);
            Value::text(format!("{:0w$x}", n(0).cast_unsigned()))
        }
        Add => num(ints().fold(0, i32::wrapping_add)),
        Sub => num(ints().skip(1).fold(n(0), i32::wrapping_sub)),
        Mul => num(ints().skip(1).fold(n(0), i32::wrapping_mul)),
        Div => num(ints()
            .skip(1)
            .filter(|&d| d != 0)
            .fold(n(0), i32::wrapping_div)),
        Mod => num(ints()
            .skip(1)
            .filter(|&d| d != 0)
            .fold(n(0), i32::wrapping_rem)),
        MulDiv => num(match n(2) {
            0 => -1,
            c => {
                let q = text::round_div(i64::from(n(0)) * i64::from(n(1)), i64::from(c));
                i32::try_from(q).unwrap_or(-1)
            }
        }),
        Min => num(ints().min().unwrap_or(0)),
        Max => num(ints().max().unwrap_or(0)),

        FilePart => Value::text(path::filepart(s(0))),
        FileExt => Value::text(path::fileext(s(0))),
        Directory => Value::text(path::directory(s(0), n_or(1, 1))),
        PathLTrim => Value::text(path::ltrim(s(0), n_or(1, 1))),
        PathLPart => Value::text(path::lpart(s(0), n_or(1, 1))),
        PathRTrim => Value::text(path::rtrim(s(0), n_or(1, 1))),
        PathRPart => Value::text(path::rpart(s(0), n_or(1, 1))),
        PathSafe => Value::text(path::safe(s(0))),

        Put | Puts => {
            let value = a.get(1).cloned().unwrap_or_default();
            let out = if func == Put {
                value.text.clone()
            } else {
                String::new()
            };
            state.vars.insert(s(0).to_lowercase(), value);
            Value::text(out)
        }
        Get => state
            .vars
            .get(&s(0).to_lowercase())
            .cloned()
            .unwrap_or_default(),
        Null => Value::default(),
        Year => num(state.now().year),
        Month => two_digits(state.now().month),
        Day => two_digits(state.now().day),
        Hour => two_digits(state.now().hour),
        Minute => two_digits(state.now().minute),
        Second => two_digits(state.now().second),

        Unknown => Value::text(crate::UNKNOWN_FUNCTION),
    }
}

fn two_digits(v: u32) -> Value {
    Value::text(format!("{v:02}"))
}

/// The first argument whose length beats every earlier one under `better`.
fn extreme(a: &[Value], better: impl Fn(usize, usize) -> bool) -> Value {
    let mut best: Option<(&str, usize)> = None;
    for v in a {
        let l = len(&v.text);
        if best.is_none_or(|(_, b)| better(l, b)) {
            best = Some((&v.text, l));
        }
    }
    Value::text(best.map_or("", |(t, _)| t))
}
