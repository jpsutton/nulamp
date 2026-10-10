//! One-time import of an original player's settings folder (11 §1.4): `winamp.ini`, EQ
//! presets, bookmarks and the default playlist.

use std::path::{Path, PathBuf};

use nulamp_skin::ini::Ini;

use crate::settings::{self, PlayerState};
use nulamp_ui::UiPrefs;

/// Maps Windows paths (`C:\…`) of a settings folder inside a Wine prefix to host paths, through
/// the prefix's `dosdevices` links. Outside Wine, paths are kept as they are.
#[derive(Debug, Clone, Default)]
struct PathMap {
    dosdevices: Option<PathBuf>,
}

impl PathMap {
    fn for_dir(dir: &Path) -> Self {
        let dosdevices = dir
            .ancestors()
            .map(|a| a.join("dosdevices"))
            .find(|d| d.is_dir());
        Self { dosdevices }
    }

    fn map(&self, p: &str) -> String {
        let b = p.as_bytes();
        let Some(dev) = &self.dosdevices else {
            return p.to_string();
        };
        if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
            let drive = format!("{}:", (b[0] as char).to_ascii_lowercase());
            let rest = p[3..].replace('\\', "/");
            let mapped = dev.join(drive).join(rest);
            return std::fs::canonicalize(&mapped)
                .unwrap_or(mapped)
                .to_string_lossy()
                .into_owned();
        }
        p.to_string()
    }
}

/// Translates `[Winamp]` keys into nulamp's own settings text, so the normal settings reader
/// validates and clamps them. Unknown and unmapped keys are dropped.
fn translate(text: &str, paths: &PathMap, skins: &Path) -> String {
    let ini = Ini::parse(text);
    let get = |k: &str| ini.get("Winamp", k).map(str::trim);
    let num = |k: &str| get(k).and_then(|v| v.parse::<i64>().ok());
    let mut out = vec!["[nulamp]".to_string()];
    let mut put = |k: &str, v: String| out.push(format!("{k}={v}"));
    let copy = [
        ("wx", "main_x"),
        ("wy", "main_y"),
        ("eq_wx", "eq_x"),
        ("eq_wy", "eq_y"),
        ("pe_wx", "pl_x"),
        ("pe_wy", "pl_y"),
        ("pe_width", "pl_width"),
        ("eq_open", "eq_visible"),
        ("pe_open", "pl_visible"),
        ("windowshade", "shade"),
        ("eq_ws", "eq_shade"),
        ("dsize", "double_size"),
        ("eqdsize", "eq_double"),
        ("aot", "always_on_top"),
        ("easymove", "easymove"),
        ("volume", "volume"),
        ("pan", "balance"),
        ("shuffle", "shuffle"),
        ("repeat", "repeat"),
        ("pilp", "position"),
        ("timeleftmode", "time_remaining"),
        ("preamp", "eq_preamp"),
        ("use_eq", "eq_enabled"),
        ("autoload_eq", "eq_autoload"),
        ("eq_limiter", "eq_limiter"),
        ("sa", "vis_mode"),
        ("sa_peaks", "vis_peaks"),
        ("safalloff", "vis_falloff"),
        ("sa_peak_falloff", "vis_peak_falloff"),
        ("snap", "snap"),
        ("snaplen", "snap_distance"),
        ("keeponscreen", "keep_on_screen"),
        ("autoscrollname", "autoscroll"),
        ("dotitlenum", "show_number_in_title"),
        ("shownumsinpl", "pl_show_numbers"),
        ("zeropadplnum", "pl_zero_pad"),
        ("plmw2xscroll", "pl_wheel_double"),
        ("plscrollsize", "pl_page_lines"),
        ("pe_fontsize", "pl_font_px"),
        ("replaygain", "rg_enabled"),
        ("replaygain_preferred_only", "rg_preferred_only"),
    ];
    for (from, to) in copy {
        if let Some(n) = num(from) {
            put(to, n.to_string());
        }
    }
    // Height 14 is the shaded playlist; the unshaded height is then `pe_height_ws`.
    match num("pe_height") {
        Some(14) => {
            put("pl_shade", "1".into());
            if let Some(h) = num("pe_height_ws") {
                put("pl_height", h.to_string());
            }
        }
        Some(h) => put("pl_height", h.to_string()),
        None => {}
    }
    if let Some(a) = num("pladv") {
        put("manual_advance", ((a == 0) as u8).to_string());
    }
    if let Some(d) = get("eq_data") {
        put("eq_bands", d.to_string());
    }
    if let Some(f) = num("eq_frequencies") {
        put("eq_iso", ((f != 0) as u8).to_string());
    }
    if let Some(f) = num("safire") {
        put("vis_style", (f & 3).to_string());
        put("vis_scope", ((f >> 2) & 3).to_string());
        put("vis_thick", ((f & 0x20 == 0) as u8).to_string());
    }
    if let Some(s) = num("replaygain_source") {
        put("rg_album", ((s == 1) as u8).to_string());
    }
    if let Some(m) = num("replaygain_mode") {
        put("rg_mode", m.to_string());
    }
    for (from, to) in [
        ("replaygain_preamp", "rg_preamp_db"),
        ("replaygain_non_rg_gain", "rg_no_tag_db"),
    ] {
        if let Some(milli) = num(from) {
            put(to, (milli as f64 / 1000.0).to_string());
        }
    }
    if let Some(t) = num("taskbar") {
        put("show_tray", ((t == 1 || t == 2) as u8).to_string());
    }
    if let Some(skin) = get("skin").filter(|s| !s.is_empty()) {
        let dir = get("SkinDir")
            .map(|d| PathBuf::from(paths.map(d)))
            .unwrap_or_else(|| skins.to_path_buf());
        let path = dir.join(skin);
        if path.exists() {
            put("skin", path.to_string_lossy().into_owned());
        }
    }
    out.join("\n")
}

/// What an import brought over.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub settings: bool,
    pub files: Vec<String>,
}

/// Imports `dir` (a folder holding `winamp.ini`) into the settings and the config folder.
/// Files already present in the config folder are not overwritten.
pub fn import(dir: &Path, config: &Path) -> std::io::Result<(UiPrefs, PlayerState, Report)> {
    let ini_path = dir.join("winamp.ini");
    let bytes = std::fs::read(&ini_path)?;
    let paths = PathMap::for_dir(dir);
    // `utf8=1` marks UTF-8 strings; older files are in the system code page (Windows-1252).
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => encoding_rs::WINDOWS_1252
            .decode(e.as_bytes())
            .0
            .into_owned(),
    };
    let ours = translate(&text, &paths, &dir.join("Skins"));
    let (prefs, state) = settings::from_ini(&ours);
    let mut report = Report {
        settings: true,
        files: Vec::new(),
    };
    std::fs::create_dir_all(config)?;
    for name in [
        nulamp_eq::preset::PRESET_LIBRARY_FILE,
        nulamp_eq::preset::AUTOLOAD_LIBRARY_FILE,
    ] {
        let (from, to) = (dir.join(name), config.join(name));
        if from.is_file() && !to.exists() && std::fs::copy(&from, &to).is_ok() {
            report.files.push(name.to_string());
        }
    }
    let bm = config.join("nulamp.bm8");
    if !bm.exists() {
        let utf8 = std::fs::read(dir.join("winamp.bm8")).ok();
        let legacy = std::fs::read(dir.join("winamp.bm")).ok().map(|b| {
            encoding_rs::WINDOWS_1252
                .decode(&b)
                .0
                .into_owned()
                .into_bytes()
        });
        if let Some(b) = utf8.or(legacy) {
            let list: Vec<_> = nulamp_playlist::read_bookmarks(&b)
                .into_iter()
                .map(|mut m| {
                    m.location = paths.map(&m.location);
                    m
                })
                .collect();
            if !list.is_empty()
                && std::fs::write(&bm, nulamp_playlist::write_bookmarks(&list)).is_ok()
            {
                report.files.push("bookmarks".into());
            }
        }
    }
    let pl = settings::playlist_path(config);
    if !pl.exists()
        && let Some(src) = ["winamp.m3u8", "winamp.m3u"]
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
        && let Ok(entries) = nulamp_playlist::read_file(&src.to_string_lossy())
    {
        let entries: Vec<_> = entries
            .into_iter()
            .map(|mut e| {
                e.location = paths.map(&e.location);
                e
            })
            .collect();
        if let Ok(b) = nulamp_playlist::write(
            &entries,
            nulamp_playlist::Format::M3u8,
            &nulamp_playlist::WriteOptions::default(),
        ) && std::fs::write(&pl, b).is_ok()
        {
            report.files.push("playlist".into());
        }
    }
    Ok((prefs, state, report))
}

/// Settings folders of an original installation, most likely first.
pub fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(appdata).join("Winamp"));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let prefix = std::env::var_os("WINEPREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".wine"));
        let c = prefix.join("drive_c");
        if let Ok(users) = std::fs::read_dir(c.join("users")) {
            for u in users.flatten() {
                v.push(u.path().join("AppData/Roaming/Winamp"));
                v.push(u.path().join("Application Data/Winamp"));
            }
        }
        v.push(c.join("Program Files (x86)/Winamp"));
        v.push(c.join("Program Files/Winamp"));
    }
    for p in [
        "C:\\Program Files (x86)\\Winamp",
        "C:\\Program Files\\Winamp",
    ] {
        v.push(PathBuf::from(p));
    }
    v.retain(|d| d.join("winamp.ini").is_file());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = "[Winamp]\nutf8=1\nwx=10\nwy=20\npe_wx=10\npe_wy=136\npe_width=300\n\
        pe_height=14\npe_height_ws=232\nvolume=180\npan=-20\nshuffle=1\npladv=0\npilp=3\n\
        eq_data=1,2,3,4,5,6,7,8,9,10\npreamp=40\nuse_eq=1\neq_frequencies=1\nsa=2\n\
        safire=38\nreplaygain=1\nreplaygain_source=1\nreplaygain_preamp=-3500\n\
        taskbar=2\nskin=Missing.wsz\n";

    #[test]
    fn translates_settings() {
        let text = translate(INI, &PathMap::default(), Path::new("/nonexistent"));
        let (p, st) = settings::from_ini(&text);
        assert_eq!(p.main_pos, (10, 20));
        assert_eq!(p.pl_pos, (10, 136));
        assert_eq!(p.pl_size, (300, 232));
        assert!(p.pl_shade);
        assert_eq!((st.volume, st.balance, st.position), (180, -20, 3));
        assert!(st.shuffle && st.manual_advance);
        assert_eq!(p.eq.bands, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(p.eq.preamp, 40);
        assert!(p.eq.enabled && p.eq.iso);
        assert_eq!(p.vis.mode, nulamp_vis::VisMode::Oscilloscope);
        // safire 38 = 0b100110: style 2 (line), scope 1 (lines), thin bands.
        assert_eq!(p.vis.style, nulamp_vis::AnalyzerStyle::Line);
        assert_eq!(p.vis.scope, nulamp_vis::ScopeStyle::Lines);
        assert!(!p.vis.thick_bands);
        assert!(p.replaygain.enabled);
        assert_eq!(p.replaygain.source, nulamp_eq::replaygain::Source::Album);
        assert_eq!(p.replaygain.preamp_db, -3.5);
        assert!(p.show_tray);
        assert_eq!(p.skin_path, None);
    }

    #[test]
    fn imports_files_from_a_wine_prefix() {
        let root = std::env::temp_dir().join(format!("nulamp-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let drive = root.join("drive_c");
        let wa = drive.join("Program Files/Winamp");
        let music = drive.join("Music");
        std::fs::create_dir_all(wa.join("Skins")).unwrap();
        std::fs::create_dir_all(&music).unwrap();
        std::fs::create_dir_all(root.join("dosdevices")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&drive, root.join("dosdevices/c:")).unwrap();
        std::fs::write(music.join("a.mp3"), b"x").unwrap();
        std::fs::write(wa.join("Skins/Cool.wsz"), b"zip").unwrap();
        std::fs::write(
            wa.join("winamp.ini"),
            "[Winamp]\nvolume=99\nskin=Cool.wsz\nSkinDir=C:\\Program Files\\Winamp\\Skins\n",
        )
        .unwrap();
        std::fs::write(wa.join("winamp.m3u8"), "#EXTM3U\nC:\\Music\\a.mp3\n").unwrap();
        std::fs::write(wa.join("winamp.q1"), b"preset bytes").unwrap();
        let config = root.join("config");
        let (p, st, report) = import(&wa, &config).unwrap();
        assert_eq!(st.volume, 99);
        assert!(report.files.contains(&"winamp.q1".to_string()));
        #[cfg(unix)]
        {
            assert!(p.skin_path.as_deref().unwrap().ends_with("Skins/Cool.wsz"));
            let pl = std::fs::read_to_string(settings::playlist_path(&config)).unwrap();
            assert!(pl.contains("drive_c/Music/a.mp3"), "{pl}");
            assert!(report.files.contains(&"playlist".to_string()));
        }
        let _ = p;
        let _ = std::fs::remove_dir_all(&root);
    }
}
