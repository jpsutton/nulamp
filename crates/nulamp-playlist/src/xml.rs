//! Tolerant XML reader for the XML-like playlist formats.
//!
//! Builds a small element tree and never fails: stray `&` and `<` are text, attribute
//! values may be unquoted, unknown entities are kept as written, mismatched end tags close
//! the nearest open element of that name (or are ignored), and unclosed elements are
//! closed at the end of the input. Comments, processing instructions, doctypes and CDATA
//! are handled.

#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    /// Name without a namespace prefix.
    pub fn local_name(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }

    /// True when the local name is `name` (ASCII case-insensitive when `ci`).
    pub fn is(&self, name: &str, ci: bool) -> bool {
        let n = self.local_name();
        if ci {
            n.eq_ignore_ascii_case(name)
        } else {
            n == name
        }
    }

    /// Attribute value (name compared ASCII case-insensitively).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.rsplit(':').next().unwrap_or(k).eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Child elements in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> + '_ {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// Child elements named `name`.
    pub fn children_named<'a>(
        &'a self,
        name: &'a str,
        ci: bool,
    ) -> impl Iterator<Item = &'a Element> {
        self.elements().filter(move |e| e.is(name, ci))
    }

    /// First child element named `name`.
    pub fn child(&self, name: &str, ci: bool) -> Option<&Element> {
        self.elements().find(|e| e.is(name, ci))
    }

    /// First element named `name` in depth-first order, including `self`.
    pub fn find(&self, name: &str, ci: bool) -> Option<&Element> {
        if self.is(name, ci) {
            return Some(self);
        }
        self.elements().find_map(|e| e.find(name, ci))
    }

    /// All text inside the element, trimmed.
    pub fn text(&self) -> String {
        fn collect(e: &Element, out: &mut String) {
            for n in &e.children {
                match n {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(c) => collect(c, out),
                }
            }
        }
        let mut s = String::new();
        collect(self, &mut s);
        s.trim().to_string()
    }

    /// Text of the first child named `name`, if present and non-empty.
    pub fn child_text(&self, name: &str, ci: bool) -> Option<String> {
        self.child(name, ci)
            .map(Element::text)
            .filter(|t| !t.is_empty())
    }
}

/// Parses `text` into a synthetic root element (empty name) holding the top-level nodes.
/// Elements named in `void` (ASCII case-insensitive) never have children.
pub fn parse(text: &str, void: &[&str]) -> Element {
    let s = text;
    let mut stack = vec![Element::default()];
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if !rest.starts_with('<') {
            let end = rest.find('<').map_or(s.len(), |n| i + n);
            push_text(&mut stack, &decode_entities(&s[i..end]));
            i = end;
        } else if rest.starts_with("<!--") {
            i = skip_past(s, i + 4, "-->");
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").map_or(s.len(), |n| i + 9 + n);
            push_text(&mut stack, &s[i + 9..end]);
            i = (end + 3).min(s.len());
        } else if rest.starts_with("<?") {
            i = skip_past(s, i + 2, "?>");
        } else if rest.starts_with("<!") {
            i = skip_past(s, i + 2, ">");
        } else if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>').map_or(s.len(), |n| i + 2 + n);
            close(&mut stack, s[i + 2..end].trim());
            i = (end + 1).min(s.len());
        } else if rest[1..].starts_with(|c: char| c.is_alphabetic() || c == '_' || c == ':') {
            let (el, empty, next) = start_tag(s, i + 1);
            if empty || void.iter().any(|v| v.eq_ignore_ascii_case(&el.name)) {
                stack
                    .last_mut()
                    .expect("root")
                    .children
                    .push(Node::Element(el));
            } else {
                stack.push(el);
            }
            i = next;
        } else {
            push_text(&mut stack, "<");
            i += 1;
        }
    }
    while stack.len() > 1 {
        let el = stack.pop().expect("len > 1");
        stack
            .last_mut()
            .expect("root")
            .children
            .push(Node::Element(el));
    }
    stack.pop().expect("root")
}

fn skip_past(s: &str, from: usize, end: &str) -> usize {
    s[from..]
        .find(end)
        .map_or(s.len(), |n| from + n + end.len())
}

fn push_text(stack: &mut [Element], t: &str) {
    if t.is_empty() {
        return;
    }
    let top = stack.last_mut().expect("root");
    if let Some(Node::Text(prev)) = top.children.last_mut() {
        prev.push_str(t);
    } else {
        top.children.push(Node::Text(t.to_string()));
    }
}

/// Closes the nearest open element named `name` and everything opened after it.
fn close(stack: &mut Vec<Element>, name: &str) {
    let Some(k) = stack
        .iter()
        .rposition(|e| !e.name.is_empty() && e.name.eq_ignore_ascii_case(name))
    else {
        return;
    };
    while stack.len() > k {
        let el = stack.pop().expect("len > k");
        stack
            .last_mut()
            .expect("root")
            .children
            .push(Node::Element(el));
    }
}

/// Reads a start tag whose name begins at `i`. Returns the element, whether it is
/// self-closing, and the index after the tag.
fn start_tag(s: &str, mut i: usize) -> (Element, bool, usize) {
    let is_space = |c: char| c.is_whitespace();
    let name_end = s[i..]
        .find(|c: char| is_space(c) || c == '/' || c == '>')
        .map_or(s.len(), |n| i + n);
    let mut el = Element {
        name: s[i..name_end].to_string(),
        ..Element::default()
    };
    i = name_end;
    loop {
        i += s[i..].find(|c: char| !is_space(c)).unwrap_or(s.len() - i);
        let rest = &s[i..];
        if rest.is_empty() {
            return (el, false, i);
        }
        if rest.starts_with("/>") {
            return (el, true, i + 2);
        }
        if rest.starts_with('>') {
            return (el, false, i + 1);
        }
        let key_end = rest
            .find(|c: char| is_space(c) || matches!(c, '=' | '>' | '/'))
            .unwrap_or(rest.len());
        if key_end == 0 {
            i += rest.chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let key = rest[..key_end].to_string();
        i += key_end;
        let after = s[i..]
            .find(|c: char| !is_space(c))
            .map_or(s.len(), |n| i + n);
        if !s[after..].starts_with('=') {
            el.attrs.push((key, String::new()));
            continue;
        }
        i = after + 1;
        i += s[i..].find(|c: char| !is_space(c)).unwrap_or(s.len() - i);
        let rest = &s[i..];
        let value;
        if let Some(q) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) {
            match rest[1..].find(q) {
                Some(n) => {
                    value = &rest[1..n + 1];
                    i += n + 2;
                }
                None => {
                    let end = rest.find('>').unwrap_or(rest.len());
                    value = &rest[1..end.max(1)];
                    i += end;
                }
            }
        } else {
            let end = rest
                .find(|c: char| is_space(c) || c == '>')
                .unwrap_or(rest.len());
            let raw = &rest[..end];
            i += end;
            if raw.len() > 1 && raw.ends_with('/') && s[i..].starts_with('>') {
                el.attrs.push((key, decode_entities(&raw[..raw.len() - 1])));
                return (el, true, i + 1);
            }
            value = raw;
        }
        el.attrs.push((key, decode_entities(value)));
    }
}

/// Decodes the predefined and numeric entities. Anything else (including a stray `&`) is
/// kept as written.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&n| n <= 10)
            .and_then(|n| entity(&rest[1..1 + n]).map(|c| (c, n + 2)));
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = lower.strip_prefix('#')?;
            let code = match num.strip_prefix('x') {
                Some(h) => u32::from_str_radix(h, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// Decodes an XML document: a byte-order mark decides; else the encoding declared in the
/// XML declaration (or by `extra_label`, which takes precedence); else `default`, or
/// strict UTF-8 then Windows-1252 when `default` is `None`.
pub fn decode(
    bytes: &[u8],
    default: Option<&'static encoding_rs::Encoding>,
    extra_label: impl Fn(&str) -> Option<String>,
) -> String {
    use crate::text::{bom, decode_with};
    if let Some((enc, rest)) = bom(bytes) {
        return decode_with(enc, rest);
    }
    let head = decode_with(encoding_rs::WINDOWS_1252, bytes);
    let label = extra_label(&head).or_else(|| declared_encoding(&head[..head.len().min(1024)]));
    if let Some(enc) = label.and_then(|l| encoding_rs::Encoding::for_label(l.trim().as_bytes())) {
        // A UTF-16 label without a byte-order mark cannot be right for ASCII-compatible text.
        if enc != encoding_rs::UTF_16LE && enc != encoding_rs::UTF_16BE {
            return decode_with(enc, bytes);
        }
    }
    match default {
        Some(enc) => decode_with(enc, bytes),
        None => crate::text::decode_utf8_or_1252(bytes),
    }
}

/// Encoding label from an XML declaration (`<?xml … encoding="…"?>`) at the start.
pub fn declared_encoding(head: &str) -> Option<String> {
    let start = head.find("<?xml")?;
    let end = head[start..].find("?>").map_or(head.len(), |n| start + n);
    let decl = parse(&format!("<{} />", &head[start + 2..end]), &[]);
    decl.elements().next()?.attr("encoding").map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_parsing() {
        let doc = parse(
            "<?xml version='1.0'?><!-- c --><A x=1 Y=\"a&amp;b\" z='q'><b>T &amp; U & V &#65;&#x42;</B>\
             <ref href=http://h/x/><c><![CDATA[<raw>]]></c><d>unclosed</a> tail < x",
            &["ref"],
        );
        let a = doc.child("a", true).unwrap();
        assert_eq!(a.attr("X"), Some("1"));
        assert_eq!(a.attr("y"), Some("a&b"));
        assert_eq!(a.attr("z"), Some("q"));
        assert_eq!(a.child_text("B", true).unwrap(), "T & U & V AB");
        assert_eq!(
            a.child("REF", true).unwrap().attr("href"),
            Some("http://h/x")
        );
        assert_eq!(a.child_text("c", true).unwrap(), "<raw>");
        assert_eq!(a.child_text("d", true).unwrap(), "unclosed");
        assert!(a.child("a", false).is_none());
        assert_eq!(doc.text(), "T & U & V AB<raw>unclosed tail < x");
    }

    #[test]
    fn void_elements_and_mismatch() {
        let doc = parse("<e><ref href=\"a\"><title>t</title></x></e>", &["ref"]);
        let e = doc.child("e", false).unwrap();
        assert_eq!(e.child_text("title", false).unwrap(), "t");
    }

    #[test]
    fn declaration() {
        assert_eq!(
            declared_encoding("<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>"),
            Some("ISO-8859-1".into())
        );
        assert_eq!(declared_encoding("<a/>"), None);
    }
}
