//! Minimal INI reader for PLS and ASX v2 (10 §3.3).
//!
//! Section and key names are case-insensitive, values are trimmed and lose matching
//! surrounding quotes, lines starting with `;` are comments. The first occurrence of a key
//! wins.

use std::collections::HashMap;

/// Keys (lower-case) and values of one section.
pub type Section = HashMap<String, String>;

/// The section named `name`, or `None` when the text has no such section.
pub fn section(text: &str, name: &str) -> Option<Section> {
    let mut found: Option<Section> = None;
    let mut inside = false;
    for line in text.split(['\n', '\r']) {
        let line = line.trim_start_matches(['\u{feff}', ' ', '\t']);
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                inside = rest[..end].trim().eq_ignore_ascii_case(name);
                if inside {
                    found.get_or_insert_default();
                }
            }
            continue;
        }
        let (true, Some(eq)) = (inside, line.find('=')) else {
            continue;
        };
        let key = line[..eq].trim().to_ascii_lowercase();
        let value = unquote(line[eq + 1..].trim()).to_string();
        found
            .as_mut()
            .expect("inside a section")
            .entry(key)
            .or_insert(value);
    }
    found
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_keys_quotes() {
        let t = "; c\nfoo=bar\n[Other]\nfile1=x\n[PlayList]\r\nFILE1 = \"a b\" \nfile1=dup\nTitle1='t'\n;File2=c";
        let s = section(t, "playlist").unwrap();
        assert_eq!(s.get("file1").unwrap(), "a b");
        assert_eq!(s.get("title1").unwrap(), "t");
        assert!(!s.contains_key("file2"));
        assert!(!s.contains_key("foo"));
        assert!(section(t, "reference").is_none());
    }
}
