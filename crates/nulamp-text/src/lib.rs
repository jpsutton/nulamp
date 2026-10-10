//! 1-bit (non-antialiased) text rendering, font resolution and shaping.
//!
//! All glyphs are rendered with FreeType in monochrome mode with full hinting
//! (`FT_LOAD_TARGET_MONO`, `FT_RENDER_MODE_MONO`). Embedded bitmap strikes are used when a
//! font has them. No grey levels are ever produced (docs/decisions.md D2).
//!
//! Text is split into bidirectional runs (`unicode-bidi`), then into font runs (per-character
//! fallback), and shaped with `rustybuzz`. Glyph advances are FreeType's hinted, whole-pixel
//! advances so that text sits on the pixel grid.

use std::collections::HashMap;
use std::rc::Rc;

use freetype::face::LoadFlag;
use freetype::{Library, RenderMode};
use nulamp_skin::Bitmap;

/// Text direction preference (04 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Auto,
    Ltr,
    Rtl,
}

/// Substitutes for Windows font names that skins commonly request (01 §6.3).
const SUBSTITUTES: &[(&str, &[&str])] = &[
    (
        "arial",
        &["Liberation Sans", "Arimo", "Helvetica", "DejaVu Sans"],
    ),
    (
        "helvetica",
        &["Liberation Sans", "Arimo", "Arial", "DejaVu Sans"],
    ),
    (
        "ms sans serif",
        &["Liberation Sans", "Arimo", "DejaVu Sans"],
    ),
    (
        "microsoft sans serif",
        &["Liberation Sans", "Arimo", "DejaVu Sans"],
    ),
    ("tahoma", &["DejaVu Sans", "Liberation Sans", "Verdana"]),
    ("verdana", &["DejaVu Sans", "Liberation Sans", "Tahoma"]),
    (
        "segoe ui",
        &["Noto Sans", "Open Sans", "DejaVu Sans", "Liberation Sans"],
    ),
    ("trebuchet ms", &["DejaVu Sans", "Liberation Sans"]),
    (
        "times new roman",
        &["Liberation Serif", "Tinos", "DejaVu Serif"],
    ),
    ("georgia", &["DejaVu Serif", "Liberation Serif"]),
    (
        "courier new",
        &["Liberation Mono", "Cousine", "DejaVu Sans Mono"],
    ),
    (
        "courier",
        &["Liberation Mono", "Cousine", "DejaVu Sans Mono"],
    ),
    ("lucida console", &["DejaVu Sans Mono", "Liberation Mono"]),
    ("consolas", &["DejaVu Sans Mono", "Liberation Mono"]),
];

/// Default UI fonts, tried in order, and the fallback chain for missing glyphs.
const DEFAULT_FAMILIES: &[&str] = &[
    "Liberation Sans",
    "Arimo",
    "Arial",
    "DejaVu Sans",
    "Noto Sans",
    "Helvetica",
    "Segoe UI",
    "Tahoma",
];

const FALLBACK_FAMILIES: &[&str] = &[
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans Arabic",
    "Noto Sans Hebrew",
    "Noto Sans Devanagari",
    "Noto Sans Thai",
    "Arial Unicode MS",
    "Segoe UI Symbol",
    "Noto Sans Symbols",
    "Noto Sans Symbols2",
    "Noto Color Emoji",
];

/// A font request: family name (may be empty) and pixel size (em height).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontSpec {
    pub family: String,
    pub pixel_size: u32,
}

impl FontSpec {
    pub fn new(family: impl Into<String>, pixel_size: u32) -> Self {
        Self {
            family: family.into(),
            pixel_size: pixel_size.max(2),
        }
    }
}

struct LoadedFace {
    data: Rc<Vec<u8>>,
    index: u32,
    ft: freetype::Face,
    size: std::cell::Cell<u32>,
}

impl LoadedFace {
    fn set_size(&self, px: u32) {
        if self.size.get() != px {
            if self.ft.set_pixel_sizes(0, px).is_err() {
                // Bitmap-only fonts: pick the nearest strike.
                let _ = self.ft.set_pixel_sizes(px, px);
            }
            self.size.set(px);
        }
    }
}

#[derive(Clone)]
struct GlyphImage {
    left: i32,
    top: i32,
    width: u32,
    rows: u32,
    /// One byte per pixel, 0 or 1.
    bits: Vec<u8>,
    advance: i32,
}

/// One positioned glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacedGlyph {
    face: usize,
    gid: u32,
    /// Pen x relative to the layout origin.
    pub x: i32,
    pub y_offset: i32,
    pub advance: i32,
}

/// Vertical metrics of a font at a size, in whole pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub ascent: i32,
    pub descent: i32,
    pub line_height: i32,
}

/// A shaped single line of text in visual order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub glyphs: Vec<PlacedGlyph>,
    pub width: i32,
    pub metrics: Metrics,
    pub pixel_size: u32,
}

/// Loads fonts, shapes and draws text. Not thread-safe; owned by the UI thread.
pub struct FontSystem {
    lib: Library,
    db: fontdb::Database,
    faces: Vec<LoadedFace>,
    by_id: HashMap<fontdb::ID, usize>,
    family_cache: HashMap<String, Option<usize>>,
    fallback_cache: HashMap<(usize, char), Option<usize>>,
    glyph_cache: HashMap<(usize, u32, u32), Option<GlyphImage>>,
    default_face: Option<usize>,
}

impl FontSystem {
    /// Creates a font system with the platform's installed fonts.
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self::with_database(db)
    }

    /// Creates a font system from a prepared font database (tests, bundled fonts).
    pub fn with_database(db: fontdb::Database) -> Self {
        let lib = Library::init().expect("FreeType initialises");
        let mut s = Self {
            lib,
            db,
            faces: Vec::new(),
            by_id: HashMap::new(),
            family_cache: HashMap::new(),
            fallback_cache: HashMap::new(),
            glyph_cache: HashMap::new(),
            default_face: None,
        };
        s.default_face = DEFAULT_FAMILIES
            .iter()
            .find_map(|f| s.family_face(f))
            .or_else(|| {
                let id = s.db.faces().next().map(|f| f.id)?;
                s.load(id)
            });
        s
    }

    /// Whether any usable font was found.
    pub fn has_fonts(&self) -> bool {
        self.default_face.is_some()
    }

    fn load(&mut self, id: fontdb::ID) -> Option<usize> {
        if let Some(&i) = self.by_id.get(&id) {
            return Some(i);
        }
        let (data, index) = self.db.with_face_data(id, |d, i| (d.to_vec(), i))?;
        let data = Rc::new(data);
        let ft = self
            .lib
            .new_memory_face(data.clone(), index as isize)
            .ok()?;
        self.faces.push(LoadedFace {
            data,
            index,
            ft,
            size: std::cell::Cell::new(0),
        });
        let i = self.faces.len() - 1;
        self.by_id.insert(id, i);
        Some(i)
    }

    fn family_face(&mut self, family: &str) -> Option<usize> {
        let key = family.to_lowercase();
        if let Some(&c) = self.family_cache.get(&key) {
            return c;
        }
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        // Only accept an exact family match; fontdb's query never substitutes names.
        let found = self.db.query(&query).and_then(|id| self.load(id));
        self.family_cache.insert(key, found);
        found
    }

    /// Resolves a family name to a face, with Windows-font substitution and the default font.
    fn resolve(&mut self, family: &str) -> Option<usize> {
        let family = family.trim();
        if !family.is_empty() {
            if let Some(f) = self.family_face(family) {
                return Some(f);
            }
            let lower = family.to_lowercase();
            if let Some((_, subs)) = SUBSTITUTES.iter().find(|(n, _)| *n == lower)
                && let Some(f) = subs.iter().find_map(|s| self.family_face(s))
            {
                return Some(f);
            }
        }
        self.default_face
    }

    /// Name of the family a request resolves to (for diagnostics and preferences).
    pub fn resolved_family(&mut self, family: &str) -> Option<String> {
        let face = self.resolve(family)?;
        self.faces[face].ft.family_name()
    }

    fn has_char(&self, face: usize, c: char) -> bool {
        self.faces[face]
            .ft
            .get_char_index(c as usize)
            .is_some_and(|g| g != 0)
    }

    fn face_for_char(&mut self, primary: usize, c: char) -> usize {
        if self.has_char(primary, c) || c.is_control() {
            return primary;
        }
        if let Some(&f) = self.fallback_cache.get(&(primary, c)) {
            return f.unwrap_or(primary);
        }
        let mut found = None;
        for fam in FALLBACK_FAMILIES {
            if let Some(f) = self.family_face(fam)
                && self.has_char(f, c)
            {
                found = Some(f);
                break;
            }
        }
        if found.is_none() {
            let ids: Vec<fontdb::ID> = self.db.faces().map(|f| f.id).collect();
            for id in ids {
                if let Some(f) = self.load(id)
                    && self.has_char(f, c)
                {
                    found = Some(f);
                    break;
                }
            }
        }
        self.fallback_cache.insert((primary, c), found);
        found.unwrap_or(primary)
    }

    /// Vertical metrics for a font request.
    pub fn metrics(&mut self, spec: &FontSpec) -> Metrics {
        let Some(face) = self.resolve(&spec.family) else {
            let px = spec.pixel_size as i32;
            return Metrics {
                ascent: px,
                descent: px / 4,
                line_height: px + px / 4,
            };
        };
        self.face_metrics(face, spec.pixel_size)
    }

    fn face_metrics(&self, face: usize, px: u32) -> Metrics {
        let f = &self.faces[face];
        f.set_size(px);
        match f.ft.size_metrics() {
            Some(m) => {
                let ascent = ((m.ascender + 32) >> 6) as i32;
                let descent = ((-m.descender + 32) >> 6) as i32;
                let line_height = (((m.height + 32) >> 6) as i32).max(ascent + descent);
                Metrics {
                    ascent,
                    descent,
                    line_height,
                }
            }
            None => {
                let px = px as i32;
                Metrics {
                    ascent: px,
                    descent: px / 4,
                    line_height: px + px / 4,
                }
            }
        }
    }

    fn glyph(&mut self, face: usize, gid: u32, px: u32) -> Option<GlyphImage> {
        if let Some(g) = self.glyph_cache.get(&(face, gid, px)) {
            return g.clone();
        }
        let f = &self.faces[face];
        f.set_size(px);
        let img = (|| {
            f.ft.load_glyph(gid, LoadFlag::TARGET_MONO | LoadFlag::COLOR)
                .ok()?;
            let slot = f.ft.glyph();
            slot.render_glyph(RenderMode::Mono).ok()?;
            let bmp = slot.bitmap();
            let (w, rows, pitch) = (bmp.width() as u32, bmp.rows() as u32, bmp.pitch());
            let buf = bmp.buffer();
            let mode = bmp.pixel_mode().ok()?;
            let mut bits = vec![0u8; (w * rows) as usize];
            for y in 0..rows {
                let row_start = if pitch >= 0 {
                    y as i64 * pitch as i64
                } else {
                    (rows - 1 - y) as i64 * (-pitch) as i64
                } as usize;
                for x in 0..w {
                    let on = match mode {
                        freetype::bitmap::PixelMode::Mono => buf
                            .get(row_start + x as usize / 8)
                            .is_some_and(|b| b & (0x80 >> (x % 8)) != 0),
                        // Colour (emoji) or grey strikes: threshold, never blend.
                        freetype::bitmap::PixelMode::Bgra => buf
                            .get(row_start + x as usize * 4 + 3)
                            .is_some_and(|a| *a >= 128),
                        _ => buf.get(row_start + x as usize).is_some_and(|v| *v >= 128),
                    };
                    bits[(y * w + x) as usize] = on as u8;
                }
            }
            Some(GlyphImage {
                left: slot.bitmap_left(),
                top: slot.bitmap_top(),
                width: w,
                rows,
                bits,
                advance: ((slot.advance().x + 32) >> 6) as i32,
            })
        })();
        self.glyph_cache.insert((face, gid, px), img.clone());
        img
    }

    /// Shapes one line of text.
    pub fn layout(&mut self, text: &str, spec: &FontSpec, dir: Direction) -> Layout {
        let px = spec.pixel_size;
        let Some(primary) = self.resolve(&spec.family) else {
            return Layout {
                glyphs: Vec::new(),
                width: 0,
                metrics: self.metrics(spec),
                pixel_size: px,
            };
        };
        let metrics = self.face_metrics(primary, px);
        let text: String = text
            .chars()
            .map(|c| {
                if c == '\n' || c == '\r' || c == '\t' {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let level = match dir {
            Direction::Auto => None,
            Direction::Ltr => Some(unicode_bidi::Level::ltr()),
            Direction::Rtl => Some(unicode_bidi::Level::rtl()),
        };
        let bidi = unicode_bidi::BidiInfo::new(&text, level);
        let mut glyphs = Vec::new();
        let mut pen = 0i32;
        for para in &bidi.paragraphs {
            let (levels, runs) = bidi.visual_runs(para, para.range.clone());
            for run in runs {
                let rtl = levels[run.start].is_rtl();
                // Split the bidi run into font runs.
                let mut font_runs: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
                for (i, c) in text[run.clone()].char_indices() {
                    let at = run.start + i;
                    let f = self.face_for_char(primary, c);
                    match font_runs.last_mut() {
                        Some((lf, r)) if *lf == f => r.end = at + c.len_utf8(),
                        _ => font_runs.push((f, at..at + c.len_utf8())),
                    }
                }
                if rtl {
                    font_runs.reverse();
                }
                for (face, range) in font_runs {
                    self.shape_run(face, &text[range], rtl, px, &mut pen, &mut glyphs);
                }
            }
        }
        Layout {
            glyphs,
            width: pen,
            metrics,
            pixel_size: px,
        }
    }

    fn shape_run(
        &mut self,
        face: usize,
        s: &str,
        rtl: bool,
        px: u32,
        pen: &mut i32,
        out: &mut Vec<PlacedGlyph>,
    ) {
        let shaped: Vec<(u32, i32, f32)> = {
            let lf = &self.faces[face];
            let Some(hb) = rustybuzz::Face::from_slice(&lf.data, lf.index) else {
                return;
            };
            let scale = px as f32 / hb.units_per_em().max(1) as f32;
            let mut buf = rustybuzz::UnicodeBuffer::new();
            buf.push_str(s);
            buf.set_direction(if rtl {
                rustybuzz::Direction::RightToLeft
            } else {
                rustybuzz::Direction::LeftToRight
            });
            buf.guess_segment_properties();
            let gb = rustybuzz::shape(&hb, &[], buf);
            gb.glyph_infos()
                .iter()
                .zip(gb.glyph_positions())
                .map(|(i, p)| {
                    (
                        i.glyph_id,
                        (p.y_offset as f32 * scale).round() as i32,
                        p.x_advance as f32 * scale,
                    )
                })
                .collect()
        };
        for (gid, y_offset, hb_advance) in shaped {
            let advance = match self.glyph(face, gid, px) {
                Some(g) if g.advance > 0 || hb_advance == 0.0 => g.advance,
                _ => hb_advance.round() as i32,
            };
            out.push(PlacedGlyph {
                face,
                gid,
                x: *pen,
                y_offset,
                advance,
            });
            *pen += advance;
        }
    }

    /// Width in pixels of a line of text.
    pub fn measure(&mut self, text: &str, spec: &FontSpec) -> i32 {
        self.layout(text, spec, Direction::Auto).width
    }

    /// Lays out `text` so that it fits `max_width`, cutting it and appending `…` when needed.
    pub fn layout_fit(
        &mut self,
        text: &str,
        spec: &FontSpec,
        dir: Direction,
        max_width: i32,
    ) -> Layout {
        let full = self.layout(text, spec, dir);
        if full.width <= max_width {
            return full;
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        let mut best = self.layout("…", spec, dir);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = chars[..mid]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string()
                + "…";
            let l = self.layout(&candidate, spec, dir);
            if l.width <= max_width {
                best = l;
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        best
    }

    /// Draws a layout with its pen origin at (x, baseline). Pixels outside `clip`
    /// (x0, y0, x1, y1; exclusive) are not touched.
    pub fn draw(
        &mut self,
        frame: &mut Bitmap,
        layout: &Layout,
        x: i32,
        baseline: i32,
        color: u32,
        clip: (i32, i32, i32, i32),
    ) {
        for g in &layout.glyphs {
            let Some(img) = self.glyph(g.face, g.gid, layout.pixel_size) else {
                continue;
            };
            let ox = x + g.x + img.left;
            let oy = baseline - img.top - g.y_offset;
            for yy in 0..img.rows as i32 {
                for xx in 0..img.width as i32 {
                    if img.bits[(yy as u32 * img.width + xx as u32) as usize] == 0 {
                        continue;
                    }
                    let (px, py) = (ox + xx, oy + yy);
                    if px >= clip.0 && px < clip.2 && py >= clip.1 && py < clip.3 {
                        frame.set(px, py, color);
                    }
                }
            }
        }
    }
}

impl Default for FontSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> FontSystem {
        FontSystem::new()
    }

    #[test]
    fn renders_only_two_colours() {
        let mut fs = fs();
        if !fs.has_fonts() {
            eprintln!("no system fonts; skipping");
            return;
        }
        let mut f = Bitmap::new(200, 20, 0x000000);
        for px in [9, 10, 12, 14] {
            let spec = FontSpec::new("Arial", px);
            let l = fs.layout("Hello, World! Ünïcödé — עברית", &spec, Direction::Auto);
            assert!(l.width > 0);
            fs.draw(&mut f, &l, 2, 14, 0x00FF00, (0, 0, 200, 20));
        }
        assert!(f.pixels().iter().all(|&p| p == 0 || p == 0x00FF00));
        assert!(f.pixels().contains(&0x00FF00));
    }

    #[test]
    fn substitution_and_default() {
        let mut fs = fs();
        if !fs.has_fonts() {
            return;
        }
        assert!(fs.resolved_family("Tahoma").is_some());
        assert!(fs.resolved_family("No Such Font 123").is_some());
        assert!(fs.resolved_family("").is_some());
    }

    #[test]
    fn fit_adds_ellipsis() {
        let mut fs = fs();
        if !fs.has_fonts() {
            return;
        }
        let spec = FontSpec::new("", 12);
        let long = "A very long playlist title that does not fit";
        let full = fs.measure(long, &spec);
        let fit = fs.layout_fit(long, &spec, Direction::Ltr, full / 2);
        assert!(fit.width <= full / 2);
        assert!(fit.width > 0);
        let fits = fs.layout_fit("Hi", &spec, Direction::Ltr, 500);
        assert_eq!(fits.width, fs.measure("Hi", &spec));
    }

    #[test]
    fn metrics_are_sane() {
        let mut fs = fs();
        if !fs.has_fonts() {
            return;
        }
        let m = fs.metrics(&FontSpec::new("", 12));
        assert!(m.ascent >= 9 && m.ascent <= 14, "{m:?}");
        assert!(m.line_height >= 12 && m.line_height <= 18, "{m:?}");
    }
}
