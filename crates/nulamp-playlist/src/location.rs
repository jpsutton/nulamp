//! Entry locations: resolving what a playlist says into a usable location, and making
//! locations relative when writing (10 §3.7).

use crate::text::{percent_decode, percent_encode_path};

/// Path conventions used to interpret and produce locations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// `/` separators, case-sensitive names. `\` in relative paths is converted to `/`.
    Unix,
    /// `\` (or `/`) separators, drive letters and UNC roots, case-insensitive names.
    Windows,
}

impl PathStyle {
    /// Style of the platform this crate is compiled for.
    pub const fn native() -> Self {
        if cfg!(windows) {
            PathStyle::Windows
        } else {
            PathStyle::Unix
        }
    }

    fn sep(self) -> char {
        match self {
            PathStyle::Unix => '/',
            PathStyle::Windows => '\\',
        }
    }

    fn is_sep(self, c: char) -> bool {
        c == '/' || (self == PathStyle::Windows && c == '\\')
    }

    fn name_eq(self, a: &str, b: &str) -> bool {
        match self {
            PathStyle::Unix => a == b,
            PathStyle::Windows => eq_ignore_case(a, b),
        }
    }
}

/// File-system queries used by the case-insensitive fallback.
pub trait FileSystem {
    /// True when `path` exists.
    fn exists(&self, path: &str) -> bool;
    /// Names of the entries of the folder `path` (empty when it cannot be read).
    fn list_dir(&self, path: &str) -> Vec<String>;
}

/// The real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFs;

impl FileSystem for StdFs {
    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn list_dir(&self, path: &str) -> Vec<String> {
        std::fs::read_dir(path)
            .map(|rd| {
                rd.filter_map(|e| e.ok()?.file_name().into_string().ok())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }
}

/// A file system where every path exists; disables the case-insensitive fallback.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullFs;

impl FileSystem for NullFs {
    fn exists(&self, _path: &str) -> bool {
        true
    }

    fn list_dir(&self, _path: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Resolves entry locations against the playlist they come from.
#[derive(Clone, Copy)]
pub struct Resolver<'a> {
    base: Option<&'a str>,
    style: PathStyle,
    fs: &'a dyn FileSystem,
}

impl std::fmt::Debug for Resolver<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver")
            .field("base", &self.base)
            .field("style", &self.style)
            .finish()
    }
}

impl<'a> Resolver<'a> {
    /// `base` is the playlist's own location (file path or URL), if known.
    pub fn new(base: Option<&'a str>, style: PathStyle, fs: &'a dyn FileSystem) -> Self {
        Self { base, style, fs }
    }

    /// Native style and the real file system.
    pub fn native(base: Option<&'a str>) -> Self {
        Self::new(base, PathStyle::native(), &StdFs)
    }

    /// Native style without file-system access.
    pub fn plain(base: Option<&'a str>) -> Self {
        Self::new(base, PathStyle::native(), &NullFs)
    }

    pub fn base(&self) -> Option<&'a str> {
        self.base
    }

    pub fn style(&self) -> PathStyle {
        self.style
    }

    /// Resolves a location as written in a playlist. Returns `None` when it is empty.
    ///
    /// - URLs (`scheme://…`) are returned unchanged; `file:` URIs are percent-decoded.
    /// - Windows drive (`C:\…`) and UNC (`\\server\…`) paths are normalised on Windows and
    ///   kept as given elsewhere (they fail at play time).
    /// - Drive-relative paths (`\Music\a.mp3`) take the playlist's drive on Windows; on
    ///   Unix they are read as absolute paths with `\` separators.
    /// - Relative paths are joined to the playlist folder and normalised (`.`, `..`). On
    ///   Unix `\` becomes `/`, and a path that does not exist with the exact case is
    ///   matched case-insensitively component by component.
    /// - With a URL base, relative and root-relative paths are joined to the URL.
    pub fn resolve(&self, raw: &str) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        if let Some(path) = file_uri_to_path(raw, self.style) {
            return (!path.is_empty()).then(|| self.resolve_path(&path));
        }
        if is_url(raw) {
            return Some(raw.to_string());
        }
        Some(self.resolve_path(raw))
    }

    /// Resolves a plain path (no URI decoding).
    pub fn resolve_path(&self, p: &str) -> String {
        let url_base = self.base.filter(|b| is_url(b));
        match self.style {
            PathStyle::Unix => {
                if p.starts_with("\\\\") || is_drive(p) {
                    return p.to_string();
                }
                let p = p.replace('\\', "/");
                if let Some(base) = url_base {
                    return url_join(base, &p);
                }
                if p.starts_with('/') {
                    return normalize(&p, PathStyle::Unix);
                }
                self.join_relative(&p)
            }
            PathStyle::Windows => {
                let (root, rest) = split_root(p, PathStyle::Windows);
                match root.kind {
                    RootKind::Drive | RootKind::Unc => normalize(p, PathStyle::Windows),
                    RootKind::DriveCwd => p.to_string(),
                    RootKind::Slash => {
                        if let Some(base) = url_base {
                            return url_join(base, &p.replace('\\', "/"));
                        }
                        match self.base.map(|b| split_root(b, PathStyle::Windows).0) {
                            Some(
                                r @ Root {
                                    kind: RootKind::Drive | RootKind::Unc,
                                    ..
                                },
                            ) => normalize(&format!("{}{rest}", r.text), PathStyle::Windows),
                            _ => normalize(p, PathStyle::Windows),
                        }
                    }
                    RootKind::None => match url_base {
                        Some(base) => url_join(base, &p.replace('\\', "/")),
                        None => self.join_relative(p),
                    },
                }
            }
        }
    }

    fn join_relative(&self, rel: &str) -> String {
        let Some(base) = self.base else {
            return normalize(rel, self.style);
        };
        let dir = parent(base, self.style);
        let joined = if dir.is_empty() {
            normalize(rel, self.style)
        } else {
            normalize(&format!("{dir}{}{rel}", self.style.sep()), self.style)
        };
        case_fallback(&joined, self.style, self.fs)
    }
}

/// True for `scheme://…` locations (scheme of two or more characters).
pub fn is_url(s: &str) -> bool {
    let Some(pos) = s.find("://") else {
        return false;
    };
    let scheme = &s[..pos];
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// True for `C:`, `C:\…`, `C:/…` and `C:…`.
fn is_drive(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Converts a `file:` URI to a path in `style`. `None` when `uri` is not a `file:` URI.
///
/// `file:///C:/x` gives `C:\x` (Windows) or `C:/x` (Unix); `file://host/share/x` gives a
/// UNC path; `file://localhost/x` is the same as `file:///x`.
pub fn file_uri_to_path(uri: &str, style: PathStyle) -> Option<String> {
    let rest = strip_prefix_ci(uri, "file:")?;
    let (host, path) = match rest.strip_prefix("//") {
        Some(r) => match r.find('/') {
            Some(i) => (&r[..i], &r[i..]),
            None => (r, ""),
        },
        None => ("", rest),
    };
    let path = percent_decode(path);
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return Some(match style {
            PathStyle::Windows => format!("\\\\{host}{}", path.replace('/', "\\")),
            PathStyle::Unix => format!("//{host}{path}"),
        });
    }
    let b = path.as_bytes();
    let path = if b.len() >= 3
        && b[0] == b'/'
        && b[1].is_ascii_alphabetic()
        && matches!(b[2], b':' | b'|')
    {
        format!("{}:{}", &path[1..2], &path[3..])
    } else {
        path
    };
    Some(match style {
        PathStyle::Windows => path.replace('/', "\\"),
        PathStyle::Unix => path,
    })
}

/// Converts an absolute path to a percent-encoded `file://` URI. A relative path becomes a
/// percent-encoded relative URI reference with `/` separators.
pub fn path_to_file_uri(path: &str, style: PathStyle) -> String {
    let p = match style {
        PathStyle::Windows => path.replace('\\', "/"),
        PathStyle::Unix => path.to_string(),
    };
    if style == PathStyle::Windows
        && let Some(unc) = p.strip_prefix("//")
    {
        return format!("file://{}", percent_encode_path(unc));
    }
    if is_drive(&p) {
        return format!("file:///{}", percent_encode_path(&p));
    }
    if p.starts_with('/') {
        return format!("file://{}", percent_encode_path(&p));
    }
    percent_encode_path(&p)
}

/// Location to write for a playlist in `folder`: a relative path when `location` is a
/// file under `folder`, otherwise `location` unchanged. Never produces `..`.
pub fn relativize(location: &str, folder: &str, style: PathStyle) -> String {
    if is_url(location) {
        return location.to_string();
    }
    let (loc_root, loc_rest) = split_root(location, style);
    let (dir_root, dir_rest) = split_root(folder, style);
    if loc_root.kind == RootKind::None
        || !style.name_eq(&loc_root.canonical(style), &dir_root.canonical(style))
    {
        return location.to_string();
    }
    let loc: Vec<&str> = components(loc_rest, style).collect();
    let dir: Vec<&str> = components(dir_rest, style).collect();
    if loc.contains(&"..") || loc.contains(&".") || dir.contains(&"..") || dir.contains(&".") {
        return location.to_string();
    }
    if loc.len() <= dir.len() || !dir.iter().zip(&loc).all(|(a, b)| style.name_eq(a, b)) {
        return location.to_string();
    }
    loc[dir.len()..].join(&style.sep().to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootKind {
    /// Relative path.
    None,
    /// `/` (Unix absolute) or `\` (Windows drive-relative).
    Slash,
    /// `C:\`.
    Drive,
    /// `C:` without a separator.
    DriveCwd,
    /// `\\server\share\`.
    Unc,
}

#[derive(Debug, Clone)]
struct Root<'a> {
    kind: RootKind,
    text: &'a str,
}

impl Root<'_> {
    /// Root text with separators in `style`.
    fn canonical(&self, style: PathStyle) -> String {
        match style {
            PathStyle::Unix => self.text.to_string(),
            PathStyle::Windows => self.text.replace('/', "\\"),
        }
    }
}

fn split_root(p: &str, style: PathStyle) -> (Root<'_>, &str) {
    let root = |kind, n: usize| {
        (
            Root {
                kind,
                text: &p[..n],
            },
            &p[n..],
        )
    };
    if style == PathStyle::Unix {
        return if p.starts_with('/') {
            root(RootKind::Slash, 1)
        } else {
            root(RootKind::None, 0)
        };
    }
    let mut chars = p.char_indices();
    let first_two_seps = p.len() >= 2 && p.chars().take(2).all(|c| style.is_sep(c));
    if first_two_seps {
        // `\\server\share\`: the root ends after the separator that follows the share name.
        let mut seps = 0;
        let mut end = p.len();
        for (i, c) in p.char_indices().skip(2) {
            if style.is_sep(c) {
                seps += 1;
                if seps == 2 {
                    end = i + 1;
                    break;
                }
            }
        }
        return root(RootKind::Unc, end);
    }
    if is_drive(p) {
        return match p[2..].chars().next() {
            Some(c) if style.is_sep(c) => root(RootKind::Drive, 3),
            _ => root(RootKind::DriveCwd, 2),
        };
    }
    match chars.next() {
        Some((_, c)) if style.is_sep(c) => root(RootKind::Slash, 1),
        _ => root(RootKind::None, 0),
    }
}

fn components(rest: &str, style: PathStyle) -> impl Iterator<Item = &str> {
    rest.split(move |c| style.is_sep(c))
        .filter(|c| !c.is_empty())
}

/// Removes `.` and `..` components and duplicate separators. `..` never climbs above a
/// root; in relative paths leading `..` components are kept.
fn normalize(p: &str, style: PathStyle) -> String {
    let (root, rest) = split_root(p, style);
    let mut out: Vec<&str> = Vec::new();
    for c in components(rest, style) {
        match c {
            "." => {}
            ".." => match out.last() {
                Some(&last) if last != ".." => {
                    out.pop();
                }
                _ if root.kind == RootKind::None => out.push(".."),
                _ => {}
            },
            c => out.push(c),
        }
    }
    let mut s = root.canonical(style);
    if root.kind == RootKind::Unc && !s.ends_with(style.sep()) && !out.is_empty() {
        s.push(style.sep());
    }
    s.push_str(&out.join(&style.sep().to_string()));
    s
}

/// Folder part of a location (everything before the last separator).
fn parent(p: &str, style: PathStyle) -> String {
    let (root, rest) = split_root(p, style);
    match rest.rfind(|c| style.is_sep(c)) {
        Some(i) => format!("{}{}", root.text, &rest[..i]),
        None => root.text.to_string(),
    }
}

/// Joins a relative or root-relative reference to a URL.
fn url_join(base: &str, rel: &str) -> String {
    let base = &base[..base.find(['?', '#']).unwrap_or(base.len())];
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let auth_end = base[scheme_end..]
        .find('/')
        .map_or(base.len(), |i| scheme_end + i);
    if let Some(r) = rel.strip_prefix("//") {
        return format!("{}//{r}", &base[..scheme_end - 2]);
    }
    if rel.starts_with('/') {
        return format!("{}{rel}", &base[..auth_end]);
    }
    let dir_end = base[auth_end..]
        .rfind('/')
        .map_or(base.len(), |i| auth_end + i);
    format!("{}/{rel}", &base[..dir_end])
}

/// Returns `path` when it exists; otherwise matches each missing component
/// case-insensitively. Falls back to `path` when no match is found.
fn case_fallback(path: &str, style: PathStyle, fs: &dyn FileSystem) -> String {
    if fs.exists(path) {
        return path.to_string();
    }
    let (root, rest) = split_root(path, style);
    let sep = style.sep();
    let mut cur = root.text.to_string();
    for comp in components(rest, style) {
        let join = |name: &str| {
            if cur.is_empty() || cur.ends_with(|c| style.is_sep(c)) {
                format!("{cur}{name}")
            } else {
                format!("{cur}{sep}{name}")
            }
        };
        let exact = join(comp);
        if fs.exists(&exact) {
            cur = exact;
            continue;
        }
        let dir = if cur.is_empty() { "." } else { cur.as_str() };
        match fs
            .list_dir(dir)
            .into_iter()
            .find(|n| eq_ignore_case(n, comp))
        {
            Some(name) => cur = join(&name),
            None => return path.to_string(),
        }
    }
    cur
}

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

pub(crate) fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    fn unix(base: &str, raw: &str) -> String {
        Resolver::new(Some(base), PathStyle::Unix, &NullFs)
            .resolve(raw)
            .unwrap()
    }

    fn win(base: &str, raw: &str) -> String {
        Resolver::new(Some(base), PathStyle::Windows, &NullFs)
            .resolve(raw)
            .unwrap()
    }

    #[test]
    fn unix_resolution() {
        let b = "/music/lists/a.m3u";
        assert_eq!(unix(b, "../a.mp3"), "/music/a.mp3");
        assert_eq!(unix(b, "sub\\b.mp3"), "/music/lists/sub/b.mp3");
        assert_eq!(unix(b, "./x/../y.mp3"), "/music/lists/y.mp3");
        assert_eq!(unix(b, "../../../../z.mp3"), "/z.mp3");
        assert_eq!(unix(b, "/abs//p/./q.mp3"), "/abs/p/q.mp3");
        assert_eq!(unix(b, "\\Music\\a.mp3"), "/Music/a.mp3");
        assert_eq!(unix(b, "C:\\Music\\a.mp3"), "C:\\Music\\a.mp3");
        assert_eq!(unix(b, "\\\\srv\\share\\a.mp3"), "\\\\srv\\share\\a.mp3");
        assert_eq!(unix(b, "  http://h/s?x=1  "), "http://h/s?x=1");
        assert_eq!(unix(b, "file:///home/a%20b/c.mp3"), "/home/a b/c.mp3");
        assert_eq!(unix(b, "file://localhost/x.mp3"), "/x.mp3");
        assert_eq!(unix(b, "file:///C:/M/a.mp3"), "C:/M/a.mp3");
        assert_eq!(unix(b, "cda://D,3"), "cda://D,3");
        assert!(Resolver::plain(None).resolve("  ").is_none());
    }

    #[test]
    fn colon_in_relative_name_is_not_a_url() {
        assert_eq!(unix("/m/l.m3u", "song: remix.mp3"), "/m/song: remix.mp3");
    }

    #[test]
    fn windows_resolution() {
        let b = "C:\\Music\\Lists\\a.m3u";
        assert_eq!(win(b, "../a.mp3"), "C:\\Music\\a.mp3");
        assert_eq!(win(b, "sub\\b.mp3"), "C:\\Music\\Lists\\sub\\b.mp3");
        assert_eq!(win(b, "\\Other\\c.mp3"), "C:\\Other\\c.mp3");
        assert_eq!(win(b, "/Other/c.mp3"), "C:\\Other\\c.mp3");
        assert_eq!(win(b, "D:/x/../y.mp3"), "D:\\y.mp3");
        assert_eq!(
            win(b, "\\\\srv\\share\\d\\..\\e.mp3"),
            "\\\\srv\\share\\e.mp3"
        );
        assert_eq!(
            win("\\\\srv\\share\\l.m3u", "\\x.mp3"),
            "\\\\srv\\share\\x.mp3"
        );
        assert_eq!(
            win("\\\\srv\\share\\l.m3u", "x.mp3"),
            "\\\\srv\\share\\x.mp3"
        );
        assert_eq!(win(b, "file:///D:/a%20b.mp3"), "D:\\a b.mp3");
        assert_eq!(win(b, "file://srv/share/a.mp3"), "\\\\srv\\share\\a.mp3");
    }

    #[test]
    fn url_base() {
        let b = "http://h:8000/dir/list.m3u?k=v";
        assert_eq!(unix(b, "a.mp3"), "http://h:8000/dir/a.mp3");
        assert_eq!(unix(b, "/root.mp3"), "http://h:8000/root.mp3");
        assert_eq!(unix(b, "sub\\b.mp3"), "http://h:8000/dir/sub/b.mp3");
        assert_eq!(win(b, "a.mp3"), "http://h:8000/dir/a.mp3");
        assert_eq!(unix("http://h", "a.mp3"), "http://h/a.mp3");
    }

    #[test]
    fn case_insensitive_fallback() {
        let t = TempDir::new("case");
        t.write("Music/Sub Dir/Song.MP3", b"x");
        let base = t.path("lists/x.m3u");
        let r = Resolver::new(Some(&base), PathStyle::Unix, &StdFs);
        assert_eq!(
            r.resolve("../music/sub dir/song.mp3").unwrap(),
            t.path("Music/Sub Dir/Song.MP3")
        );
        // No match: the path is kept as resolved.
        assert_eq!(
            r.resolve("../music/none.mp3").unwrap(),
            t.path("music/none.mp3")
        );
        // Exact case wins.
        t.write("lists/a.mp3", b"x");
        assert_eq!(r.resolve("a.mp3").unwrap(), t.path("lists/a.mp3"));
    }

    #[test]
    fn relative_writing() {
        let u = PathStyle::Unix;
        assert_eq!(relativize("/m/l/a.mp3", "/m/l", u), "a.mp3");
        assert_eq!(relativize("/m/l/s/a.mp3", "/m/l/", u), "s/a.mp3");
        assert_eq!(relativize("/m/a.mp3", "/m/l", u), "/m/a.mp3");
        assert_eq!(relativize("/m/lx/a.mp3", "/m/l", u), "/m/lx/a.mp3");
        assert_eq!(relativize("/M/l/a.mp3", "/m/l", u), "/M/l/a.mp3");
        assert_eq!(relativize("http://h/m/l/a", "/m/l", u), "http://h/m/l/a");
        let w = PathStyle::Windows;
        assert_eq!(relativize("C:\\M\\L\\s\\a.mp3", "c:\\m\\l", w), "s\\a.mp3");
        assert_eq!(
            relativize("D:\\M\\L\\a.mp3", "C:\\M\\L", w),
            "D:\\M\\L\\a.mp3"
        );
        assert_eq!(relativize("C:\\M\\a.mp3", "C:\\M\\L", w), "C:\\M\\a.mp3");
    }

    #[test]
    fn file_uris() {
        let u = PathStyle::Unix;
        let w = PathStyle::Windows;
        assert_eq!(
            path_to_file_uri("/a b/é.mp3", u),
            "file:///a%20b/%C3%A9.mp3"
        );
        assert_eq!(
            path_to_file_uri("C:\\a b\\x.mp3", w),
            "file:///C:/a%20b/x.mp3"
        );
        assert_eq!(
            path_to_file_uri("\\\\srv\\sh\\x.mp3", w),
            "file://srv/sh/x.mp3"
        );
        assert_eq!(path_to_file_uri("sub/a b.mp3", u), "sub/a%20b.mp3");
        for (p, s) in [
            ("/a b/é#?.mp3", u),
            ("C:\\a b\\x.mp3", w),
            ("\\\\srv\\sh\\x.mp3", w),
        ] {
            assert_eq!(file_uri_to_path(&path_to_file_uri(p, s), s).unwrap(), p);
        }
        assert!(file_uri_to_path("http://x", u).is_none());
    }
}
