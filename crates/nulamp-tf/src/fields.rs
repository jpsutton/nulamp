//! Field values supplied by the caller, and the derived fields of spec 10 §5.3.

use std::collections::HashMap;

use crate::path;

/// Metadata for one item.
///
/// `name` is always lower-case. Missing and empty values are the same to the formatter.
pub trait FieldSource {
    fn field(&self, name: &str) -> Option<String>;
}

/// Keys must be lower-case.
impl FieldSource for HashMap<String, String> {
    fn field(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

/// Value of field `name` (lower-case), or `None` when missing or empty.
///
/// When the source has no value: `tracknumber` and `track` stand in for each other, and
/// `folder` is the immediate parent folder of `filename`.
pub(crate) fn resolve(src: &dyn FieldSource, name: &str) -> Option<String> {
    let value = |n: &str| src.field(n).filter(|v| !v.is_empty());
    value(name).or_else(|| match name {
        "tracknumber" => value("track"),
        "track" => value("tracknumber"),
        "folder" => value("filename")
            .map(|f| path::directory(&f, 1).to_string())
            .filter(|v| !v.is_empty()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn derived_fields() {
        let m = map(&[("track", "3/12"), ("filename", "C:\\music\\album\\x.mp3")]);
        assert_eq!(resolve(&m, "tracknumber").as_deref(), Some("3/12"));
        assert_eq!(resolve(&m, "folder").as_deref(), Some("album"));
        let m = map(&[("tracknumber", "7"), ("folder", "own"), ("title", "")]);
        assert_eq!(resolve(&m, "track").as_deref(), Some("7"));
        assert_eq!(resolve(&m, "folder").as_deref(), Some("own"));
        assert_eq!(resolve(&m, "title"), None);
        assert_eq!(resolve(&m, "missing"), None);
    }
}
