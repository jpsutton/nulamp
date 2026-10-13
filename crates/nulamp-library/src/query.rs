//! Search queries and sort orders.
//!
//! A query string is a list of terms that must all match:
//!
//! - free text, split into tokens as in jump-to-file (spec 10 §6): runs of Unicode letters
//!   and digits, or `"quoted phrases"` kept whole; each token must be a case-insensitive
//!   substring of the title, artist, album, album artist, genre, comment or file name;
//! - `field:value` (substring) or `field=value` (whole value) for the text fields `title`,
//!   `artist`, `album`, `albumartist`, `genre`, `comment`, `composer`, `publisher`,
//!   `filename` and `path`; the value may be `"quoted"`;
//! - `field<op>number` with `op` one of `:`, `=`, `!=`, `<`, `<=`, `>`, `>=` for the number
//!   fields `year`, `rating`, `plays`, `length` (seconds), `bitrate`, `track` and `disc`.
//!
//! A field term must start a word (start of the string or after whitespace). Anything that
//! is not a valid field term is free text. The local provider compiles queries to SQL with
//! bound parameters only; [`Query::matches`] evaluates them in memory for providers that
//! filter themselves.

use std::cmp::Ordering;

use nulamp_core::playlist::natural_cmp;
use rusqlite::types::Value;

use crate::model::Track;

/// Case folding used for every comparison.
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// Splits text into folded tokens: runs of letters and digits, or `"quoted phrases"` in
/// which every character is kept (spec 10 §6). An unclosed quote runs to the end.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            quoted = !quoted;
        } else if quoted || c.is_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|t| fold(&t)).collect()
}

/// Text fields usable in `field:value` terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextField {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Comment,
    Composer,
    Publisher,
    /// Last component of the location.
    Filename,
    /// The whole location.
    Path,
}

impl TextField {
    /// Free-text tokens are searched in these fields.
    pub const FREE_TEXT: [TextField; 7] = [
        TextField::Title,
        TextField::Artist,
        TextField::Album,
        TextField::AlbumArtist,
        TextField::Genre,
        TextField::Comment,
        TextField::Filename,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "title" => Self::Title,
            "artist" => Self::Artist,
            "album" => Self::Album,
            "albumartist" => Self::AlbumArtist,
            "genre" => Self::Genre,
            "comment" => Self::Comment,
            "composer" => Self::Composer,
            "publisher" => Self::Publisher,
            "filename" => Self::Filename,
            "path" => Self::Path,
            _ => return None,
        })
    }

    /// The field's value in `track`.
    pub fn value(self, track: &Track) -> Option<&str> {
        match self {
            Self::Title => track.tag("title"),
            Self::Artist => track.tag("artist"),
            Self::Album => track.tag("album"),
            Self::AlbumArtist => track.tag("albumartist"),
            Self::Genre => track.tag("genre"),
            Self::Comment => track.tag("comment"),
            Self::Composer => track.tag("composer"),
            Self::Publisher => track.tag("publisher"),
            Self::Filename => Some(track.file_name()),
            Self::Path => Some(track.location.as_str()),
        }
    }

    /// Column of the local full-text table, when the field is indexed there.
    pub(crate) fn fts_column(self) -> Option<&'static str> {
        Some(match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::AlbumArtist => "albumartist",
            Self::Genre => "genre",
            Self::Comment => "comment",
            Self::Filename => "filename",
            Self::Composer | Self::Publisher | Self::Path => return None,
        })
    }

    /// Column of the local `tracks` table.
    pub(crate) fn column(self) -> &'static str {
        match self {
            Self::Title => "t.title",
            Self::Artist => "t.artist",
            Self::Album => "t.album",
            Self::AlbumArtist => "t.albumartist",
            Self::Genre => "t.genre",
            Self::Comment => "t.comment",
            Self::Composer => "t.composer",
            Self::Publisher => "t.publisher",
            Self::Filename => "t.filename",
            Self::Path => "t.path",
        }
    }
}

/// Number fields usable in comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumField {
    Year,
    Rating,
    Plays,
    /// Seconds.
    Length,
    Bitrate,
    Track,
    Disc,
}

impl NumField {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "year" => Self::Year,
            "rating" => Self::Rating,
            "plays" | "playcount" => Self::Plays,
            "length" => Self::Length,
            "bitrate" => Self::Bitrate,
            "track" => Self::Track,
            "disc" => Self::Disc,
            _ => return None,
        })
    }

    /// The field's value in `track`; `None` when unknown.
    pub fn value(self, track: &Track) -> Option<i64> {
        match self {
            Self::Year => track.year().map(i64::from),
            Self::Rating => Some(i64::from(track.rating)),
            Self::Plays => i64::try_from(track.play_count).ok(),
            Self::Length => i64::try_from(track.length_ms / 1000).ok(),
            Self::Bitrate => track.bitrate.map(i64::from),
            Self::Track => track.track_number().map(i64::from),
            Self::Disc => track.disc_number().map(i64::from),
        }
    }

    pub(crate) fn column(self) -> &'static str {
        match self {
            Self::Year => "t.year_num",
            Self::Rating => "t.rating",
            Self::Plays => "t.play_count",
            Self::Length => "(t.length_ms / 1000)",
            Self::Bitrate => "t.bitrate",
            Self::Track => "t.track_num",
            Self::Disc => "t.disc_num",
        }
    }
}

/// Comparison of a number field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn sql(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ne => "<>",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
        }
    }

    fn test(self, a: i64, b: i64) -> bool {
        match self {
            Self::Eq => a == b,
            Self::Ne => a != b,
            Self::Lt => a < b,
            Self::Le => a <= b,
            Self::Gt => a > b,
            Self::Ge => a >= b,
        }
    }
}

/// One condition of a [`Query`]. Text values are folded ([`fold`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    /// Substring of any free-text field.
    Any(String),
    /// Substring of (or, with `exact`, equal to) one field.
    Text {
        field: TextField,
        value: String,
        exact: bool,
    },
    /// Number comparison; tracks with an unknown value never match.
    Number {
        field: NumField,
        op: CmpOp,
        value: i64,
    },
}

impl Term {
    /// Whether `track` satisfies the term.
    pub fn matches(&self, track: &Track) -> bool {
        match self {
            Term::Any(tok) => TextField::FREE_TEXT.iter().any(|f| {
                f.value(track)
                    .is_some_and(|v| fold(v).contains(tok.as_str()))
            }),
            Term::Text {
                field,
                value,
                exact,
            } => match field.value(track) {
                Some(v) => {
                    let v = fold(v);
                    if *exact {
                        v == *value
                    } else {
                        v.contains(value.as_str())
                    }
                }
                // An untagged field equals the empty text.
                None => *exact && value.is_empty(),
            },
            Term::Number { field, op, value } => {
                field.value(track).is_some_and(|v| op.test(v, *value))
            }
        }
    }
}

/// Terms that must all match. The empty query matches every track.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub terms: Vec<Term>,
}

impl Query {
    /// The query that matches everything.
    pub fn all() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Parses a search box string (see the module documentation).
    pub fn parse(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let mut terms = Vec::new();
        let mut free = String::new();
        let mut i = 0;
        while i < chars.len() {
            if (i == 0 || chars[i - 1].is_whitespace())
                && let Some((term, next)) = parse_field(&chars, i)
            {
                terms.push(term);
                free.push(' ');
                i = next;
                continue;
            }
            if chars[i] == '"' {
                // Copy a quoted phrase whole so its contents are never read as a field term;
                // an unclosed quote runs to the end, as in the tokenizer.
                let end = chars[i + 1..]
                    .iter()
                    .position(|&c| c == '"')
                    .map_or(chars.len(), |p| i + 1 + p + 1);
                free.extend(&chars[i..end]);
                i = end;
                continue;
            }
            free.push(chars[i]);
            i += 1;
        }
        terms.extend(tokenize(&free).into_iter().map(Term::Any));
        Self { terms }
    }

    /// Adds a term.
    pub fn and(mut self, term: Term) -> Self {
        self.terms.push(term);
        self
    }

    /// Whether `track` satisfies every term.
    pub fn matches(&self, track: &Track) -> bool {
        self.terms.iter().all(|t| t.matches(track))
    }

    /// SQL conditions over `tracks t` (and the `tracks_fts` table), with their parameters
    /// appended to `params`. Only fixed SQL text is produced; every value is a parameter.
    pub(crate) fn to_sql(&self, params: &mut Vec<Value>) -> Vec<String> {
        self.terms.iter().map(|t| term_sql(t, params)).collect()
    }
}

/// A `field<op>value` term starting at `i`, and the index after it.
fn parse_field(chars: &[char], i: usize) -> Option<(Term, usize)> {
    let mut j = i;
    while j < chars.len() && chars[j].is_ascii_alphabetic() {
        j += 1;
    }
    if j == i {
        return None;
    }
    let name: String = chars[i..j].iter().collect::<String>().to_ascii_lowercase();
    let two: String = chars[j..chars.len().min(j + 2)].iter().collect();
    let (op, k) = match two.as_str() {
        ">=" => (">=", j + 2),
        "<=" => ("<=", j + 2),
        "!=" => ("!=", j + 2),
        _ => match chars.get(j)? {
            ':' => (":", j + 1),
            '=' => ("=", j + 1),
            '<' => ("<", j + 1),
            '>' => (">", j + 1),
            _ => return None,
        },
    };
    let (value, next) = if chars.get(k) == Some(&'"') {
        let end = chars[k + 1..]
            .iter()
            .position(|&c| c == '"')
            .map_or(chars.len(), |p| k + 1 + p);
        let next = (end + 1).min(chars.len());
        (chars[k + 1..end].iter().collect::<String>(), next)
    } else {
        let end = chars[k..]
            .iter()
            .position(|c| c.is_whitespace())
            .map_or(chars.len(), |p| k + p);
        (chars[k..end].iter().collect::<String>(), end)
    };
    if value.is_empty() {
        return None;
    }
    if let Some(field) = TextField::from_name(&name) {
        let exact = match op {
            ":" => false,
            "=" => true,
            _ => return None,
        };
        return Some((
            Term::Text {
                field,
                value: fold(&value),
                exact,
            },
            next,
        ));
    }
    let field = NumField::from_name(&name)?;
    let op = match op {
        ":" | "=" => CmpOp::Eq,
        "!=" => CmpOp::Ne,
        "<" => CmpOp::Lt,
        "<=" => CmpOp::Le,
        ">" => CmpOp::Gt,
        _ => CmpOp::Ge,
    };
    let value = value.parse().ok()?;
    Some((Term::Number { field, op, value }, next))
}

/// An FTS5 phrase for `text` (a bound parameter, never SQL text).
fn fts_phrase(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

/// The trigram index answers substrings of at least three characters.
fn fts_usable(token: &str) -> bool {
    token.chars().count() >= 3
}

const FTS_ALL: &str = "title || char(31) || artist || char(31) || album || char(31) || \
     albumartist || char(31) || genre || char(31) || comment || char(31) || filename";

fn term_sql(term: &Term, params: &mut Vec<Value>) -> String {
    match term {
        Term::Any(tok) if fts_usable(tok) => {
            params.push(Value::Text(fts_phrase(tok)));
            "t.id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)".to_string()
        }
        Term::Any(tok) => {
            params.push(Value::Text(tok.clone()));
            format!("t.id IN (SELECT rowid FROM tracks_fts WHERE instr({FTS_ALL}, ?) > 0)")
        }
        Term::Text {
            field,
            value,
            exact: false,
        } => match field.fts_column() {
            Some(col) if fts_usable(value) => {
                params.push(Value::Text(format!("{col} : {}", fts_phrase(value))));
                "t.id IN (SELECT rowid FROM tracks_fts WHERE tracks_fts MATCH ?)".to_string()
            }
            Some(col) => {
                params.push(Value::Text(value.clone()));
                format!("t.id IN (SELECT rowid FROM tracks_fts WHERE instr({col}, ?) > 0)")
            }
            None => {
                params.push(Value::Text(value.clone()));
                format!("instr(nl_fold({}), ?) > 0", field.column())
            }
        },
        Term::Text {
            field,
            value,
            exact: true,
        } => {
            params.push(Value::Text(value.clone()));
            format!("nl_fold(COALESCE({}, '')) = ?", field.column())
        }
        Term::Number { field, op, value } => {
            params.push(Value::Integer(*value));
            format!("{} {} ?", field.column(), op.sql())
        }
    }
}

/// Columns a list can be sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortKey {
    /// Artist, then album, disc, track number and title.
    Artist,
    /// Album, then artist, disc and track number.
    Album,
    /// Title (file name when untitled), then artist.
    Title,
    /// Disc and track number, compared as numbers.
    Track,
    Year,
    Genre,
    Length,
    Rating,
    PlayCount,
    LastPlayed,
    DateAdded,
    /// Location.
    Path,
    /// Playlist order on playlist nodes; location elsewhere.
    Position,
}

/// A sort key and direction. Only the key itself (disc and track number for
/// [`SortKey::Track`]) is reversed by `descending`; its tie-breakers stay ascending. Missing values sort last in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sort {
    pub key: SortKey,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self::asc(SortKey::Artist)
    }
}

#[derive(Debug, Clone, Copy)]
enum Col {
    Artist,
    Album,
    Title,
    Disc,
    TrackNo,
    Year,
    Genre,
    Length,
    Rating,
    Plays,
    LastPlayed,
    Added,
    Path,
}

enum SortVal {
    Text(Option<String>),
    Num(Option<i64>),
}

impl Col {
    /// SQL expression and whether it is text.
    fn sql(self) -> (&'static str, bool) {
        match self {
            Col::Artist => ("t.artist", true),
            Col::Album => ("t.album", true),
            Col::Title => ("COALESCE(NULLIF(t.title, ''), t.filename)", true),
            Col::Disc => ("t.disc_num", false),
            Col::TrackNo => ("t.track_num", false),
            Col::Year => ("t.year_num", false),
            Col::Genre => ("t.genre", true),
            Col::Length => ("t.length_ms", false),
            Col::Rating => ("t.rating", false),
            Col::Plays => ("t.play_count", false),
            Col::LastPlayed => ("t.last_played", false),
            Col::Added => ("t.date_added", false),
            Col::Path => ("t.path", true),
        }
    }

    fn value(self, t: &Track) -> SortVal {
        let text = |v: Option<&str>| SortVal::Text(v.map(str::to_string));
        let num = |v: Option<i64>| SortVal::Num(v);
        match self {
            Col::Artist => text(t.artist()),
            Col::Album => text(t.album()),
            Col::Title => SortVal::Text(Some(t.display_title())),
            Col::Disc => num(t.disc_number().map(i64::from)),
            Col::TrackNo => num(t.track_number().map(i64::from)),
            Col::Year => num(t.year().map(i64::from)),
            Col::Genre => text(t.genre()),
            Col::Length => num(i64::try_from(t.length_ms).ok()),
            Col::Rating => num(Some(i64::from(t.rating))),
            Col::Plays => num(i64::try_from(t.play_count).ok()),
            Col::LastPlayed => num(t.last_played),
            Col::Added => num(t.date_added),
            Col::Path => SortVal::Text(Some(t.location.clone())),
        }
    }
}

impl Sort {
    pub const fn asc(key: SortKey) -> Self {
        Self {
            key,
            descending: false,
        }
    }

    pub const fn desc(key: SortKey) -> Self {
        Self {
            key,
            descending: true,
        }
    }

    /// Columns compared in turn; the first `n` follow `descending` (decision ML9).
    fn chain(self) -> (&'static [Col], usize) {
        use Col::*;
        let cols: &'static [Col] = match self.key {
            SortKey::Artist => &[Artist, Album, Disc, TrackNo, Title, Path],
            SortKey::Album => &[Album, Artist, Disc, TrackNo, Title, Path],
            SortKey::Title => &[Title, Artist, Path],
            SortKey::Track => &[Disc, TrackNo, Title, Path],
            SortKey::Year => &[Year, Artist, Album, Disc, TrackNo, Path],
            SortKey::Genre => &[Genre, Artist, Album, Disc, TrackNo, Path],
            SortKey::Length => &[Length, Title, Path],
            SortKey::Rating => &[Rating, Artist, Album, Disc, TrackNo, Path],
            SortKey::PlayCount => &[Plays, Artist, Album, Disc, TrackNo, Path],
            SortKey::LastPlayed => &[LastPlayed, Path],
            SortKey::DateAdded => &[Added, Path],
            SortKey::Path | SortKey::Position => &[Path],
        };
        (cols, if self.key == SortKey::Track { 2 } else { 1 })
    }

    /// Compares two tracks in this order (for providers that sort in memory).
    pub fn compare(&self, a: &Track, b: &Track) -> Ordering {
        let (cols, primary) = self.chain();
        for (i, col) in cols.iter().enumerate() {
            let desc = i < primary && self.descending;
            let ord = match (col.value(a), col.value(b)) {
                (SortVal::Text(x), SortVal::Text(y)) => {
                    let x = x.filter(|s| !s.is_empty());
                    let y = y.filter(|s| !s.is_empty());
                    cmp_missing_last(x, y, desc, |x, y| natural_cmp(x, y))
                }
                (SortVal::Num(x), SortVal::Num(y)) => cmp_missing_last(x, y, desc, Ord::cmp),
                _ => Ordering::Equal,
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.id.cmp(&b.id)
    }

    /// `ORDER BY` items for `tracks t` (fixed SQL text), ending with `t.id`.
    pub(crate) fn to_sql(self) -> String {
        let mut parts = Vec::new();
        let (cols, primary) = self.chain();
        for (i, col) in cols.iter().enumerate() {
            let dir = if i < primary && self.descending {
                "DESC"
            } else {
                "ASC"
            };
            let (e, text) = col.sql();
            if text {
                parts.push(format!(
                    "(COALESCE({e}, '') = '') ASC, {e} COLLATE nl_natural {dir}"
                ));
            } else {
                parts.push(format!("({e} IS NULL) ASC, {e} {dir}"));
            }
        }
        parts.push("t.id ASC".to_string());
        parts.join(", ")
    }
}

fn cmp_missing_last<T>(
    x: Option<T>,
    y: Option<T>,
    desc: bool,
    cmp: impl Fn(&T, &T) -> Ordering,
) -> Ordering {
    match (x, y) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let o = cmp(&x, &y);
            if desc { o.reverse() } else { o }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any(s: &str) -> Term {
        Term::Any(s.to_string())
    }

    #[test]
    fn tokens_follow_jump_to_file_rules() {
        assert_eq!(tokenize("Björk, homo-genic"), ["björk", "homo", "genic"]);
        assert_eq!(tokenize("\"Hey Jude\" x"), ["hey jude", "x"]);
        assert_eq!(tokenize("  ,;  "), Vec::<String>::new());
        assert_eq!(tokenize("\"open"), ["open"]);
    }

    #[test]
    fn free_text_and_fields() {
        let q = Query::parse("beat artist:\"The Beatles\" year>1965 album:abbey rating>=4 x");
        assert_eq!(
            q.terms,
            vec![
                Term::Text {
                    field: TextField::Artist,
                    value: "the beatles".into(),
                    exact: false
                },
                Term::Number {
                    field: NumField::Year,
                    op: CmpOp::Gt,
                    value: 1965
                },
                Term::Text {
                    field: TextField::Album,
                    value: "abbey".into(),
                    exact: false
                },
                Term::Number {
                    field: NumField::Rating,
                    op: CmpOp::Ge,
                    value: 4
                },
                any("beat"),
                any("x"),
            ]
        );
    }

    #[test]
    fn invalid_fields_are_text() {
        let q = Query::parse("year>abc foo:bar artist: \"genre:x\"");
        assert_eq!(
            q.terms,
            vec![
                any("year"),
                any("abc"),
                any("foo"),
                any("bar"),
                any("artist"),
                any("genre:x")
            ]
        );
        let q = Query::parse("'; DROP TABLE tracks; --");
        assert_eq!(q.terms, vec![any("drop"), any("table"), any("tracks")]);
    }

    #[test]
    fn number_ops() {
        for (s, op, v) in [
            ("plays>0", CmpOp::Gt, 0),
            ("year<=1999", CmpOp::Le, 1999),
            ("track:3", CmpOp::Eq, 3),
            ("disc!=2", CmpOp::Ne, 2),
            ("length<-1", CmpOp::Lt, -1),
        ] {
            match &Query::parse(s).terms[..] {
                [Term::Number { op: o, value, .. }] => assert_eq!((*o, *value), (op, v), "{s}"),
                t => panic!("{s}: {t:?}"),
            }
        }
    }

    #[test]
    fn in_memory_matching() {
        let mut t = Track {
            location: "/m/Björk/01 Hunter.flac".into(),
            rating: 4,
            ..Track::default()
        };
        t.tags.insert("artist".into(), "Björk".into());
        t.tags.insert("year".into(), "1997-09-22".into());
        assert!(Query::parse("BJÖRK hunter").matches(&t));
        assert!(Query::parse("year=1997 rating>3 filename:hunter").matches(&t));
        assert!(!Query::parse("year<1997").matches(&t));
        assert!(!Query::parse("plays>0").matches(&t));
        assert!(Query::parse("artist=björk").matches(&t));
        assert!(!Query::parse("artist=bjö").matches(&t));
    }

    #[test]
    fn sort_numeric_track_and_missing_last() {
        let mk = |n: &str, track: Option<&str>| {
            let mut t = Track::default();
            t.id.id = n.into();
            t.location = n.into();
            if let Some(tr) = track {
                t.tags.insert("track".into(), tr.into());
            }
            t
        };
        let mut v = [mk("a", Some("10")), mk("b", None), mk("c", Some("2/12"))];
        v.sort_by(|a, b| Sort::asc(SortKey::Track).compare(a, b));
        assert_eq!(
            v.iter().map(|t| t.location.as_str()).collect::<Vec<_>>(),
            ["c", "a", "b"]
        );
        v.sort_by(|a, b| Sort::desc(SortKey::Track).compare(a, b));
        assert_eq!(
            v.iter().map(|t| t.location.as_str()).collect::<Vec<_>>(),
            ["a", "c", "b"]
        );
    }
}
