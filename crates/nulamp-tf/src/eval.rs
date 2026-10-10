//! Evaluator: syntax tree + fields → title.

use std::collections::HashMap;

use chrono::{Datelike, Timelike};

use crate::fields::{self, FieldSource};
use crate::funcs;
use crate::parse::Node;
use crate::text::push_capped;

/// Local wall-clock time as seen by `$systime_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SysTime {
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl SysTime {
    /// The current local time.
    pub fn now() -> Self {
        let t = chrono::Local::now();
        Self {
            year: t.year(),
            month: t.month(),
            day: t.day(),
            hour: t.hour(),
            minute: t.minute(),
            second: t.second(),
        }
    }
}

/// An evaluated argument: its text and whether it referenced a non-empty field.
#[derive(Debug, Clone, Default)]
pub(crate) struct Value {
    pub text: String,
    pub found: bool,
}

impl Value {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            found: false,
        }
    }
}

/// State for one formatting call.
pub(crate) struct State<'a> {
    fields: &'a dyn FieldSource,
    /// `$put`/`$puts` variables, keyed by lower-case name.
    pub vars: HashMap<String, Value>,
    now: Option<SysTime>,
}

impl State<'_> {
    pub fn now(&mut self) -> SysTime {
        *self.now.get_or_insert_with(SysTime::now)
    }

    /// Appends the output of `nodes`; returns whether any non-empty field was referenced.
    fn seq(&mut self, nodes: &[Node], out: &mut String) -> bool {
        let mut found = false;
        for node in nodes {
            match node {
                Node::Text(t) => push_capped(out, t),
                Node::Field(name) => {
                    if let Some(v) = fields::resolve(self.fields, name) {
                        push_capped(out, &v);
                        found = true;
                    }
                }
                Node::Section(inner) => {
                    let mut s = String::new();
                    if self.seq(inner, &mut s) {
                        push_capped(out, &s);
                        found = true;
                    }
                }
                Node::Call(func, args) => {
                    let mut values = Vec::with_capacity(args.len());
                    for arg in args {
                        let mut text = String::new();
                        let found = self.seq(arg, &mut text);
                        values.push(Value { text, found });
                    }
                    let result = funcs::call(self, *func, &values);
                    push_capped(out, &result.text);
                    found |= result.found || values.iter().any(|v| v.found);
                }
                Node::Error => push_capped(out, crate::SYNTAX_ERROR),
            }
        }
        found
    }
}

pub(crate) fn run(nodes: &[Node], fields: &dyn FieldSource, now: Option<SysTime>) -> String {
    let mut state = State {
        fields,
        vars: HashMap::new(),
        now,
    };
    let mut out = String::new();
    state.seq(nodes, &mut out);
    out.replace(['\r', '\n'], " ")
}
