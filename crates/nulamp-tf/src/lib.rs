//! Title formatting language (spec 10 §5.3).
//!
//! A format string mixes literal text, `%field%` references, `$function(…)` calls and
//! `[…]` conditional sections. [`Formatter`] parses a string once and formats many items;
//! [`format`] does both in one call. Formatting never fails and never panics: malformed
//! input produces the error strings the spec defines.

mod eval;
mod fields;
mod funcs;
mod parse;
mod path;
mod text;

pub use eval::SysTime;
pub use fields::FieldSource;

/// Default title format (spec 10 §5.2).
pub const DEFAULT_FORMAT: &str = "[%artist% - ]$if2(%title%,$filepart(%filename%))";

/// Output for a malformed format string, or for the malformed argument or section.
pub const SYNTAX_ERROR: &str = "[SYNTAX ERROR IN FORMATTING STRING]";

/// Output of a call to a function that does not exist.
pub const UNKNOWN_FUNCTION: &str = "[UNKNOWN FUNCTION]";

/// Longest string (in bytes) any part of a formatting result may reach; longer values are cut.
pub const MAX_LEN: usize = text::MAX_LEN;

/// Parses `fmt` and formats it with `fields`, using the current local time.
pub fn format(fmt: &str, fields: &dyn FieldSource) -> String {
    Formatter::new(fmt).format(fields)
}

/// A parsed format string.
#[derive(Debug, Clone, PartialEq)]
pub struct Formatter {
    nodes: Vec<parse::Node>,
}

impl Formatter {
    /// Parses `fmt`. Syntax errors are kept in the result and appear in the output.
    pub fn new(fmt: &str) -> Self {
        Self {
            nodes: parse::parse(fmt),
        }
    }

    /// Formats one item. `$systime_*` read the local clock.
    pub fn format(&self, fields: &dyn FieldSource) -> String {
        eval::run(&self.nodes, fields, None)
    }

    /// Formats one item with `now` as the time seen by `$systime_*`.
    pub fn format_at(&self, fields: &dyn FieldSource, now: SysTime) -> String {
        eval::run(&self.nodes, fields, Some(now))
    }
}

impl Default for Formatter {
    /// The [`DEFAULT_FORMAT`].
    fn default() -> Self {
        Self::new(DEFAULT_FORMAT)
    }
}
