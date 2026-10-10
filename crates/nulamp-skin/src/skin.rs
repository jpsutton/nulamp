//! A loaded skin: bitmaps with independent fallback, text resources and cursors (01 §2–3).

use std::collections::HashMap;

use crate::bitmap::Bitmap;
use crate::container::{ContainerKind, SkinFiles};
use crate::cursor::{self, CursorImage};
use crate::fonts::{GenExColors, GenFont, TitleColors, title_colors};
use crate::ini::decode_text;
use crate::pledit::PlEdit;
use crate::region::Regions;
use crate::{bmp, viscolor};

/// Skin bitmaps by role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SkinBitmap {
    Main,
    Titlebar,
    Cbuttons,
    Playpaus,
    Monoster,
    Shufrep,
    Numbers,
    NumsEx,
    Volume,
    Balance,
    Posbar,
    Text,
    Eqmain,
    EqmainIso,
    EqEx,
    Pledit,
    Gen,
    Genex,
    Video,
    Mb,
    Avs,
}

impl SkinBitmap {
    pub const ALL: [SkinBitmap; 21] = [
        Self::Main,
        Self::Titlebar,
        Self::Cbuttons,
        Self::Playpaus,
        Self::Monoster,
        Self::Shufrep,
        Self::Numbers,
        Self::NumsEx,
        Self::Volume,
        Self::Balance,
        Self::Posbar,
        Self::Text,
        Self::Eqmain,
        Self::EqmainIso,
        Self::EqEx,
        Self::Pledit,
        Self::Gen,
        Self::Genex,
        Self::Video,
        Self::Mb,
        Self::Avs,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Main => "main.bmp",
            Self::Titlebar => "titlebar.bmp",
            Self::Cbuttons => "cbuttons.bmp",
            Self::Playpaus => "playpaus.bmp",
            Self::Monoster => "monoster.bmp",
            Self::Shufrep => "shufrep.bmp",
            Self::Numbers => "numbers.bmp",
            Self::NumsEx => "nums_ex.bmp",
            Self::Volume => "volume.bmp",
            Self::Balance => "balance.bmp",
            Self::Posbar => "posbar.bmp",
            Self::Text => "text.bmp",
            Self::Eqmain => "eqmain.bmp",
            Self::EqmainIso => "eqmain_iso.bmp",
            Self::EqEx => "eq_ex.bmp",
            Self::Pledit => "pledit.bmp",
            Self::Gen => "gen.bmp",
            Self::Genex => "genex.bmp",
            Self::Video => "video.bmp",
            Self::Mb => "mb.bmp",
            Self::Avs => "avs.bmp",
        }
    }

    /// Size of the built-in default bitmap (01 §2.2).
    pub fn default_size(self) -> (u32, u32) {
        match self {
            Self::Main => (275, 116),
            Self::Titlebar => (344, 87),
            Self::Cbuttons => (136, 36),
            Self::Playpaus => (42, 9),
            Self::Monoster => (58, 24),
            Self::Shufrep => (92, 85),
            Self::Numbers => (99, 13),
            Self::NumsEx => (108, 13),
            Self::Volume | Self::Balance => (68, 433),
            Self::Posbar => (307, 10),
            Self::Text => (155, 74),
            Self::Eqmain | Self::EqmainIso => (275, 315),
            Self::EqEx => (275, 82),
            Self::Pledit => (280, 186),
            Self::Gen => (194, 109),
            Self::Genex => (130, 75),
            Self::Video | Self::Mb => (234, 119),
            Self::Avs => (97, 188),
        }
    }
}

/// Cursor roles and their file names (01 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorRole {
    Normal,
    TitleBar,
    MainMenu,
    Min,
    WinBut,
    Close,
    VolBal,
    Posbar,
    SongName,
    MMenu,
    WsPosbar,
    WsNormal,
    EqSlid,
    EqClose,
    EqTitle,
    EqNormal,
    PtBar,
    PClose,
    PWinBut,
    PvScroll,
    PSize,
    PNormal,
    PwsSize,
    PwsNorm,
}

/// Built-in cursor shape used when a skin cursor is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorShape {
    Arrow,
    Move,
    Close,
    HorizontalResize,
    VerticalResize,
    DiagonalResize,
}

impl CursorRole {
    pub const ALL: [CursorRole; 24] = [
        Self::Normal,
        Self::TitleBar,
        Self::MainMenu,
        Self::Min,
        Self::WinBut,
        Self::Close,
        Self::VolBal,
        Self::Posbar,
        Self::SongName,
        Self::MMenu,
        Self::WsPosbar,
        Self::WsNormal,
        Self::EqSlid,
        Self::EqClose,
        Self::EqTitle,
        Self::EqNormal,
        Self::PtBar,
        Self::PClose,
        Self::PWinBut,
        Self::PvScroll,
        Self::PSize,
        Self::PNormal,
        Self::PwsSize,
        Self::PwsNorm,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            Self::Normal => "normal.cur",
            Self::TitleBar => "titlebar.cur",
            Self::MainMenu => "mainmenu.cur",
            Self::Min => "min.cur",
            Self::WinBut => "winbut.cur",
            Self::Close => "close.cur",
            Self::VolBal => "volbal.cur",
            Self::Posbar => "posbar.cur",
            Self::SongName => "songname.cur",
            Self::MMenu => "mmenu.cur",
            Self::WsPosbar => "wsposbar.cur",
            Self::WsNormal => "wsnormal.cur",
            Self::EqSlid => "eqslid.cur",
            Self::EqClose => "eqclose.cur",
            Self::EqTitle => "eqtitle.cur",
            Self::EqNormal => "eqnormal.cur",
            Self::PtBar => "ptbar.cur",
            Self::PClose => "pclose.cur",
            Self::PWinBut => "pwinbut.cur",
            Self::PvScroll => "pvscroll.cur",
            Self::PSize => "psize.cur",
            Self::PNormal => "pnormal.cur",
            Self::PwsSize => "pwssize.cur",
            Self::PwsNorm => "pwsnorm.cur",
        }
    }

    pub fn shape(self) -> CursorShape {
        use CursorShape::*;
        match self {
            Self::TitleBar | Self::EqTitle | Self::PtBar => Move,
            Self::Close | Self::EqClose | Self::PClose => Close,
            Self::VolBal | Self::Posbar | Self::SongName | Self::WsPosbar | Self::PwsSize => {
                HorizontalResize
            }
            Self::EqSlid | Self::PvScroll => VerticalResize,
            Self::PSize => DiagonalResize,
            _ => Arrow,
        }
    }
}

/// A fully resolved skin, ready for drawing.
#[derive(Debug, Clone)]
pub struct Skin {
    /// True for the built-in default skin.
    pub is_builtin: bool,
    bitmaps: HashMap<SkinBitmap, Bitmap>,
    /// Bitmaps that came from the skin itself (not the default).
    from_skin: Vec<SkinBitmap>,
    /// The large time digits come from `nums_ex.bmp` (with a minus cell).
    pub nums_ex: bool,
    /// Skin has `eqmain.bmp` but no `eq_ex.bmp`: do not draw the pressed EQ shade button (03 §1.1).
    pub eq_shade_pressed_hidden: bool,
    pub regions: Regions,
    pub vis_colors: [u32; 24],
    pub pledit: PlEdit,
    pub title_colors: TitleColors,
    pub gen_font: GenFont,
    pub genex: GenExColors,
    pub cursors: HashMap<CursorRole, CursorImage>,
    pub readme: Option<String>,
}

fn decode_bmp(files: &SkinFiles, which: SkinBitmap) -> Option<Bitmap> {
    files
        .get(which.file_name())
        .and_then(|d| bmp::decode(d).ok())
}

impl Skin {
    /// Loads the built-in skin from its own files.
    pub fn builtin(files: &SkinFiles) -> Self {
        let mut s = Self::load(files, files);
        s.is_builtin = true;
        s.from_skin.clear();
        s.pledit.use_gen_nums = true;
        s.regions = Regions::default();
        s
    }

    /// Loads a skin, falling back to `defaults` for each missing bitmap (01 §3).
    pub fn load(files: &SkinFiles, defaults: &SkinFiles) -> Self {
        let mut bitmaps = HashMap::new();
        let mut from_skin = Vec::new();
        for which in SkinBitmap::ALL {
            if let Some(b) = decode_bmp(files, which) {
                bitmaps.insert(which, b);
                from_skin.push(which);
            }
        }
        let has = |w: SkinBitmap| from_skin.contains(&w);
        let nums_ex = has(SkinBitmap::NumsEx);
        let eq_shade_pressed_hidden = has(SkinBitmap::Eqmain) && !has(SkinBitmap::EqEx);

        // Balance falls back to the skin's own volume.bmp (02 §6.2).
        if !has(SkinBitmap::Balance)
            && let Some(v) = bitmaps.get(&SkinBitmap::Volume).cloned()
        {
            bitmaps.insert(SkinBitmap::Balance, v);
        }
        // Defaults for everything else. nums_ex has no default.
        for which in SkinBitmap::ALL {
            if which == SkinBitmap::NumsEx || bitmaps.contains_key(&which) {
                continue;
            }
            let fallback = match which {
                SkinBitmap::Balance => decode_bmp(defaults, SkinBitmap::Balance)
                    .or_else(|| decode_bmp(defaults, SkinBitmap::Volume)),
                _ => decode_bmp(defaults, which),
            };
            let (w, h) = which.default_size();
            bitmaps.insert(which, fallback.unwrap_or_else(|| Bitmap::new(w, h, 0)));
        }

        let text_of = |name: &str| files.get(name).map(decode_text);
        let is_real = files.kind() != ContainerKind::BuiltIn;
        let regions = text_of("region.txt")
            .map(|t| Regions::parse(&t))
            .unwrap_or_default();
        let vis_colors = text_of("viscolor.txt")
            .map(|t| viscolor::parse(&t))
            .unwrap_or(viscolor::DEFAULT_VIS_COLORS);
        let pledit = text_of("pledit.txt")
            .map(|t| PlEdit::parse(&t))
            .unwrap_or_default();

        let mut cursors = HashMap::new();
        if is_real {
            for role in CursorRole::ALL {
                if let Some(c) = files.get(role.file_name()).and_then(cursor::decode) {
                    cursors.insert(role, c);
                }
            }
        }

        let text = &bitmaps[&SkinBitmap::Text];
        let title_colors = title_colors(text);
        let gen_font = GenFont::scan(&bitmaps[&SkinBitmap::Gen]);
        let genex = GenExColors::read(&bitmaps[&SkinBitmap::Genex]);
        Skin {
            is_builtin: false,
            bitmaps,
            from_skin,
            nums_ex,
            eq_shade_pressed_hidden,
            regions,
            vis_colors,
            pledit,
            title_colors,
            gen_font,
            genex,
            cursors,
            readme: files.readme(),
        }
    }

    /// The resolved bitmap for a role.
    ///
    /// `Numbers` returns `nums_ex.bmp` when the skin has it (01 §2). `NumsEx` is never
    /// requested directly by drawing code.
    pub fn bitmap(&self, which: SkinBitmap) -> &Bitmap {
        let which = match which {
            SkinBitmap::Numbers | SkinBitmap::NumsEx if self.nums_ex => SkinBitmap::NumsEx,
            SkinBitmap::NumsEx => SkinBitmap::Numbers,
            w => w,
        };
        &self.bitmaps[&which]
    }

    /// Whether the skin itself (not the default) supplied a bitmap.
    pub fn has_own(&self, which: SkinBitmap) -> bool {
        self.from_skin.contains(&which)
    }

    /// The equalizer bitmap, honouring the ISO-label preference (03 §1.1).
    pub fn eqmain(&self, iso: bool) -> &Bitmap {
        let which = if !self.is_builtin && iso && self.has_own(SkinBitmap::EqmainIso) {
            SkinBitmap::EqmainIso
        } else if !self.is_builtin && self.has_own(SkinBitmap::Eqmain) {
            SkinBitmap::Eqmain
        } else if iso {
            SkinBitmap::EqmainIso
        } else {
            SkinBitmap::Eqmain
        };
        &self.bitmaps[&which]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bmp::encode_24;

    fn bmp_bytes(w: u32, h: u32, c: u32) -> Vec<u8> {
        encode_24(&Bitmap::new(w, h, c))
    }

    fn defaults() -> SkinFiles {
        let entries = SkinBitmap::ALL
            .iter()
            .filter(|w| **w != SkinBitmap::NumsEx)
            .map(|w| {
                let (x, y) = w.default_size();
                (w.file_name().to_string(), bmp_bytes(x, y, 0xD0D0D0))
            });
        SkinFiles::from_entries(ContainerKind::BuiltIn, entries)
    }

    #[test]
    fn independent_fallback() {
        let skin_files = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![("main.bmp".into(), bmp_bytes(275, 116, 0x112233))],
        );
        let s = Skin::load(&skin_files, &defaults());
        assert_eq!(s.bitmap(SkinBitmap::Main).pixel(0, 0), 0x112233);
        assert_eq!(s.bitmap(SkinBitmap::Cbuttons).pixel(0, 0), 0xD0D0D0);
        assert!(s.has_own(SkinBitmap::Main));
        assert!(!s.has_own(SkinBitmap::Cbuttons));
    }

    #[test]
    fn nums_ex_wins_over_numbers() {
        let skin_files = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![
                ("numbers.bmp".into(), bmp_bytes(99, 13, 0x010101)),
                ("NUMS_EX.BMP".into(), bmp_bytes(108, 13, 0x020202)),
            ],
        );
        let s = Skin::load(&skin_files, &defaults());
        assert!(s.nums_ex);
        assert_eq!(s.bitmap(SkinBitmap::Numbers).pixel(0, 0), 0x020202);
    }

    #[test]
    fn balance_from_volume() {
        let skin_files = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![("volume.bmp".into(), bmp_bytes(68, 420, 0x030303))],
        );
        let s = Skin::load(&skin_files, &defaults());
        assert_eq!(s.bitmap(SkinBitmap::Balance).pixel(0, 0), 0x030303);
        assert_eq!(s.bitmap(SkinBitmap::Balance).height(), 420);
        // Short volume.bmp: thumb rows at y = 422 are black.
        assert_eq!(s.bitmap(SkinBitmap::Volume).pixel(15, 422), 0);
    }

    #[test]
    fn eq_quirks() {
        let skin_files = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![("eqmain.bmp".into(), bmp_bytes(275, 315, 0x040404))],
        );
        let s = Skin::load(&skin_files, &defaults());
        assert!(s.eq_shade_pressed_hidden);
        assert_eq!(s.eqmain(true).pixel(0, 0), 0x040404);
    }

    #[test]
    fn corrupt_bitmap_is_missing() {
        let skin_files = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![("main.bmp".into(), b"BMgarbage".to_vec())],
        );
        let s = Skin::load(&skin_files, &defaults());
        assert!(!s.has_own(SkinBitmap::Main));
        assert_eq!(s.bitmap(SkinBitmap::Main).pixel(0, 0), 0xD0D0D0);
    }

    #[test]
    fn cursors_load() {
        let skin_files = SkinFiles::from_entries(
            ContainerKind::Archive,
            vec![("NORMAL.CUR".into(), crate::cursor::tests::sample_cur())],
        );
        let s = Skin::load(&skin_files, &defaults());
        assert!(s.cursors.contains_key(&CursorRole::Normal));
        assert!(!s.cursors.contains_key(&CursorRole::Close));
    }
}
