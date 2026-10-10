//! SHOUTcast/Icecast in-band metadata (07 §8.5).

use nulamp_playlist::text::decode_utf8_or_1252;

/// Separates audio bytes from the metadata blocks inserted every `metaint` bytes.
#[derive(Debug, Clone)]
pub struct IcyParser {
    metaint: usize,
    /// Audio bytes left before the next length byte.
    audio_left: usize,
    /// Metadata bytes still expected for the current block.
    meta_left: Option<usize>,
    meta: Vec<u8>,
}

impl IcyParser {
    /// `metaint` 0 means the stream carries no metadata.
    pub fn new(metaint: usize) -> Self {
        Self {
            metaint,
            audio_left: metaint,
            meta_left: None,
            meta: Vec::new(),
        }
    }

    /// Splits `input`: audio goes to `audio`, each complete non-empty metadata block to
    /// `blocks` with the length of `audio` where it ends.
    pub fn feed(
        &mut self,
        mut input: &[u8],
        audio: &mut Vec<u8>,
        blocks: &mut Vec<(usize, Vec<u8>)>,
    ) {
        if self.metaint == 0 {
            audio.extend_from_slice(input);
            return;
        }
        while !input.is_empty() {
            match self.meta_left {
                None if self.audio_left > 0 => {
                    let n = self.audio_left.min(input.len());
                    audio.extend_from_slice(&input[..n]);
                    self.audio_left -= n;
                    input = &input[n..];
                }
                None => {
                    let len = input[0] as usize * 16;
                    input = &input[1..];
                    if len == 0 {
                        self.audio_left = self.metaint;
                    } else {
                        self.meta_left = Some(len);
                        self.meta.clear();
                    }
                }
                Some(left) => {
                    let n = left.min(input.len());
                    self.meta.extend_from_slice(&input[..n]);
                    input = &input[n..];
                    if n == left {
                        self.meta_left = None;
                        self.audio_left = self.metaint;
                        blocks.push((audio.len(), std::mem::take(&mut self.meta)));
                    } else {
                        self.meta_left = Some(left - n);
                    }
                }
            }
        }
    }
}

/// `Key='Value';` pairs of a metadata block. A value runs from `='` to the next `';` (or the
/// end). Text is UTF-8 when valid, else Windows-1252.
pub fn parse_metadata(block: &[u8]) -> Vec<(String, String)> {
    let end = block.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let mut rest = &block[..end];
    let mut out = Vec::new();
    while let Some(eq) = find(rest, b"='") {
        let key = decode_utf8_or_1252(&rest[..eq]).trim().to_string();
        let value_start = &rest[eq + 2..];
        let (value, next) = match find(value_start, b"';") {
            Some(i) => (&value_start[..i], &value_start[i + 2..]),
            None => {
                let v = value_start.strip_suffix(b"'").unwrap_or(value_start);
                (v, &[][..])
            }
        };
        if !key.is_empty() {
            out.push((key, decode_utf8_or_1252(value)));
        }
        rest = next;
    }
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The displayed title (07 §8.5): `StreamTitle (Station)`, `StreamTitle`, `(Station)` when
/// the stream title is empty, or the station name alone before any stream title arrived.
/// `fallback` is used when neither is known.
pub fn display_title(
    stream_title: Option<&str>,
    station: Option<&str>,
    show_station: bool,
    fallback: &str,
) -> String {
    let station = station.map(str::trim).filter(|s| !s.is_empty());
    match (stream_title.map(str::trim), station) {
        (Some(""), Some(st)) => format!("({st})"),
        (Some(""), None) | (None, None) => fallback.to_string(),
        (Some(t), Some(st)) if show_station => format!("{t} ({st})"),
        (Some(t), _) => t.to_string(),
        (None, Some(st)) => st.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(p: &mut IcyParser, data: &[u8], step: usize) -> (Vec<u8>, Vec<(usize, Vec<u8>)>) {
        let (mut a, mut m) = (Vec::new(), Vec::new());
        for c in data.chunks(step) {
            p.feed(c, &mut a, &mut m);
        }
        (a, m)
    }

    #[test]
    fn strips_metadata_at_any_split() {
        let mut data = b"abcd".to_vec();
        data.push(1);
        data.extend_from_slice(b"StreamTitle='x';");
        data.extend_from_slice(b"efgh");
        data.push(0);
        data.extend_from_slice(b"ij");
        for step in 1..8 {
            let (a, m) = split(&mut IcyParser::new(4), &data, step);
            assert_eq!(a, b"abcdefghij");
            assert_eq!(m.len(), 1);
            assert_eq!(m[0].1, b"StreamTitle='x';");
        }
        let (a, m) = split(&mut IcyParser::new(0), &data, 3);
        assert_eq!(a, data);
        assert!(m.is_empty());
    }

    #[test]
    fn parses_pairs() {
        let b = b"StreamTitle='It's A - B';StreamUrl='http://x/?a=1';\0\0\0";
        assert_eq!(
            parse_metadata(b),
            vec![
                ("StreamTitle".into(), "It's A - B".into()),
                ("StreamUrl".into(), "http://x/?a=1".into()),
            ]
        );
        assert_eq!(
            parse_metadata(b"StreamTitle='Caf\xe9'"),
            vec![("StreamTitle".into(), "Caf\u{e9}".into())]
        );
        assert_eq!(
            parse_metadata("StreamTitle='Café ☕';".as_bytes()),
            vec![("StreamTitle".into(), "Café ☕".into())]
        );
        assert!(parse_metadata(b"\0\0").is_empty());
    }

    #[test]
    fn titles() {
        assert_eq!(display_title(Some("S"), Some("R"), true, "f"), "S (R)");
        assert_eq!(display_title(Some("S"), Some("R"), false, "f"), "S");
        assert_eq!(display_title(Some(""), Some("R"), true, "f"), "(R)");
        assert_eq!(display_title(Some(""), Some("R"), false, "f"), "(R)");
        assert_eq!(display_title(None, Some("R"), true, "f"), "R");
        assert_eq!(display_title(Some("S"), None, true, "f"), "S");
        assert_eq!(display_title(None, Some(" "), true, "f"), "f");
    }
}
