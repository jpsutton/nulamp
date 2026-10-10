//! Lenient INI parsing used by skin text files.
//!
//! Section and key names are case-insensitive. Lines starting with `;` are comments.
//! Values are not length-limited. The first occurrence of a key wins.

use std::collections::HashMap;

/// Parsed INI document: section (lower-case) → key (lower-case) → raw value (trimmed).
#[derive(Debug, Default, Clone)]
pub struct Ini {
    sections: HashMap<String, HashMap<String, String>>,
}

impl Ini {
    pub fn parse(text: &str) -> Self {
        let mut ini = Ini::default();
        let mut current: Option<String> = None;
        for line in text.split(['\n', '\r']) {
            let line = line.trim_start_matches(['\u{feff}', ' ', '\t']);
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[') {
                if let Some(end) = rest.find(']') {
                    let name = rest[..end].trim().to_lowercase();
                    ini.sections.entry(name.clone()).or_default();
                    current = Some(name);
                }
                continue;
            }
            let (Some(sec), Some(eq)) = (current.as_ref(), line.find('=')) else {
                continue;
            };
            let key = line[..eq].trim().to_lowercase();
            let value = line[eq + 1..].trim().to_string();
            ini.sections
                .get_mut(sec)
                .expect("section exists")
                .entry(key)
                .or_insert(value);
        }
        ini
    }

    /// Value of `key` in `section` (both case-insensitive).
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(&section.to_lowercase())
            .and_then(|s| s.get(&key.to_lowercase()))
            .map(String::as_str)
    }

    pub fn has_section(&self, section: &str) -> bool {
        self.sections.contains_key(&section.to_lowercase())
    }
}

/// Decodes skin text: UTF-8 BOM → UTF-8, `FF FE` → UTF-16LE, valid UTF-8 → UTF-8,
/// otherwise Windows-1252.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return encoding_rs::UTF_16LE
            .decode_without_bom_handling(rest)
            .0
            .into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_and_comments() {
        let ini =
            Ini::parse("; c\n[Text]\r\nNormal = #00ff00\nnormal=ignored\n[MISC]\nUseGenNums=1");
        assert_eq!(ini.get("text", "NORMAL"), Some("#00ff00"));
        assert_eq!(ini.get("Misc", "usegennums"), Some("1"));
        assert!(ini.get("text", "missing").is_none());
    }

    #[test]
    fn long_values() {
        let v = "1, ".repeat(30_000);
        let ini = Ini::parse(&format!("[Normal]\nPointList={v}"));
        assert!(ini.get("normal", "pointlist").unwrap().len() > 64 * 1024);
    }

    #[test]
    fn text_decoding() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi"), "hi");
        assert_eq!(decode_text(b"\xFF\xFEh\0i\0"), "hi");
        assert_eq!(decode_text(b"caf\xE9"), "café");
        assert_eq!(decode_text("café".as_bytes()), "café");
    }
}
