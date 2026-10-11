//! The settings file (11 §1): UTF-8 INI, written atomically on exit.

use std::path::{Path, PathBuf};

use nulamp_skin::ini::Ini;
use nulamp_ui::UiPrefs;
use nulamp_vis::{AnalyzerStyle, RefreshRate, ScopeStyle, Speed, VisMode};

/// Player state stored with the preferences.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    pub volume: u8,
    pub balance: i8,
    pub shuffle: bool,
    pub repeat: bool,
    pub manual_advance: bool,
    pub position: usize,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            volume: 200,
            balance: 0,
            shuffle: false,
            repeat: false,
            manual_advance: false,
            position: 0,
        }
    }
}

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join("nulamp.ini")
}

pub fn playlist_path(dir: &Path) -> PathBuf {
    dir.join("nulamp.m3u8")
}

fn b(v: bool) -> String {
    if v { "1".into() } else { "0".into() }
}

/// Serialises preferences and player state.
pub fn to_ini(p: &UiPrefs, st: &PlayerState) -> String {
    let eq = &p.eq;
    let v = &p.vis;
    let bands: Vec<String> = eq.bands.iter().map(|x| x.to_string()).collect();
    let rows: Vec<(&str, String)> = vec![
        ("main_x", p.main_pos.0.to_string()),
        ("main_y", p.main_pos.1.to_string()),
        ("eq_x", p.eq_pos.0.to_string()),
        ("eq_y", p.eq_pos.1.to_string()),
        ("pl_x", p.pl_pos.0.to_string()),
        ("pl_y", p.pl_pos.1.to_string()),
        ("pl_width", p.pl_size.0.to_string()),
        ("pl_height", p.pl_size.1.to_string()),
        ("pl_shade", b(p.pl_shade)),
        ("pl_font_px", p.pl_font_px.to_string()),
        ("pl_bitmap_font", b(p.pl_bitmap_font)),
        ("pl_bitmap_scale", p.pl_bitmap_scale.to_string()),
        ("pl_show_numbers", b(p.pl_show_numbers)),
        ("pl_zero_pad", b(p.pl_zero_pad)),
        ("pl_page_lines", p.pl_page_lines.to_string()),
        ("pl_wheel_double", b(p.pl_wheel_double)),
        ("pl_old_school_buttons", b(p.pl_old_school_buttons)),
        ("recent_urls", p.recent_urls.join("\t")),
        ("eq_visible", b(p.eq_visible)),
        ("pl_visible", b(p.pl_visible)),
        ("double_size", b(p.double_size)),
        ("eq_double", b(p.eq_double)),
        ("shade", b(p.shade)),
        ("eq_shade", b(p.eq_shade)),
        ("always_on_top", b(p.always_on_top)),
        ("easymove", b(p.easymove)),
        ("snap", b(p.snap)),
        ("snap_distance", p.snap_distance.to_string()),
        ("keep_on_screen", b(p.keep_on_screen)),
        ("dim_inactive", b(p.dim_inactive)),
        ("clutterbar_always", b(p.clutterbar_always)),
        ("bitmap_title", b(p.bitmap_title)),
        ("show_number_in_title", b(p.show_number_in_title)),
        ("autoscroll", b(p.autoscroll)),
        ("time_remaining", b(p.time_remaining)),
        ("user_font", p.user_font.clone().unwrap_or_default()),
        ("skin", p.skin_path.clone().unwrap_or_default()),
        ("skins_dir", p.skins_dir.to_string_lossy().into_owned()),
        (
            "last_dir",
            p.last_dir
                .as_ref()
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        ("eq_bands", bands.join(",")),
        ("eq_preamp", eq.preamp.to_string()),
        ("eq_enabled", b(eq.enabled)),
        ("eq_autoload", b(eq.auto_load)),
        ("eq_iso", b(eq.iso)),
        ("eq_limiter", b(eq.limiter)),
        ("vis_mode", (v.mode as u8).to_string()),
        ("vis_style", (v.style as u8).to_string()),
        ("vis_thick", b(v.thick_bands)),
        ("vis_peaks", b(v.peaks)),
        ("vis_falloff", (v.falloff as u8).to_string()),
        ("vis_peak_falloff", (v.peak_falloff as u8).to_string()),
        ("vis_scope", (v.scope as u8).to_string()),
        ("vis_refresh", (v.refresh as u8).to_string()),
        ("rg_enabled", b(p.replaygain.enabled)),
        (
            "rg_album",
            b(p.replaygain.source == nulamp_eq::replaygain::Source::Album),
        ),
        ("rg_mode", (p.replaygain.mode as u8).to_string()),
        ("rg_preferred_only", b(p.replaygain.preferred_only)),
        ("rg_preamp_db", p.replaygain.preamp_db.to_string()),
        ("rg_no_tag_db", p.replaygain.no_tag_db.to_string()),
        ("fade_pause_stop", b(p.fade_pause_stop)),
        ("fade_seek", b(p.fade_seek)),
        ("buffer_ms", p.buffer_ms.to_string()),
        ("crossfade", b(p.crossfade)),
        ("crossfade_ms", p.crossfade_ms.to_string()),
        ("crossfade_equal_power", b(p.crossfade_equal_power)),
        ("crossfade_manual", b(p.crossfade_manual)),
        ("gapless_albums", b(p.gapless_albums)),
        ("show_tray", b(p.show_tray)),
        ("lib_x", p.lib_pos.0.to_string()),
        ("lib_y", p.lib_pos.1.to_string()),
        ("lib_width", p.lib_size.0.to_string()),
        ("lib_height", p.lib_size.1.to_string()),
        ("lib_visible", b(p.lib_visible)),
        ("lib_tree_width", p.lib_tree_w.to_string()),
        (
            "lib_columns",
            p.lib_columns
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ),
        (
            "library_folders",
            p.library
                .folders
                .iter()
                .map(|(path, rec)| format!("{}|{path}", b(*rec)))
                .collect::<Vec<_>>()
                .join("\t"),
        ),
        ("library_rescan_on_start", b(p.library.rescan_on_start)),
        ("library_watch", b(p.library.watch)),
        // Video window keys as named in 11 §1.3.
        ("video_wx", p.video_pos.0.to_string()),
        ("video_wy", p.video_pos.1.to_string()),
        ("video_width", p.video_size.0.to_string()),
        ("video_height", p.video_size.1.to_string()),
        ("video_open", b(p.video_visible)),
        ("video_enabled", b(p.video.enabled)),
        ("video_open_on_start", b(p.video.open_on_start)),
        ("video_close_on_end", b(p.video.close_on_end)),
        ("video_stop_on_close", b(p.video.stop_on_close)),
        ("video_resize", b(p.video.resize_to_video)),
        ("video_fullscreen_on_start", b(p.video.fullscreen_on_start)),
        (
            "video_leave_fullscreen",
            b(p.video.leave_fullscreen_on_stop),
        ),
        ("video_lock_aspect", b(p.video.lock_aspect)),
        (
            "video_aspect",
            nulamp_ui::video::format_aspect(p.video.aspect),
        ),
        ("video_logo", b(p.video.logo_when_idle)),
        ("video_no_screensaver", b(p.video.inhibit_screensaver)),
        ("volume", st.volume.to_string()),
        ("balance", st.balance.to_string()),
        ("shuffle", b(st.shuffle)),
        ("repeat", b(st.repeat)),
        ("manual_advance", b(st.manual_advance)),
        ("position", st.position.to_string()),
    ];
    let mut rows: Vec<(String, String)> =
        rows.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    if let Some(a) = &p.library.jellyfin {
        rows.extend(jellyfin_rows(a));
    }
    rows.push(("hotkeys_enabled".into(), b(p.hotkeys.enabled)));
    for (i, hk) in p.hotkeys.bindings.iter().enumerate() {
        rows.push((
            format!("hotkey_{i}"),
            format!("{} {}", hk.action.name(), hk.hotkey),
        ));
    }
    let mut out = String::from("[nulamp]\n");
    for (k, v) in rows {
        out.push_str(&format!("{k}={}\n", v.replace(['\n', '\r'], " ")));
    }
    out
}

/// A value of a tab- and `|`-separated list.
fn list_value(s: &str) -> String {
    s.replace(['\t', '|', '\n', '\r'], " ")
}

/// The signed-in Jellyfin account (`jellyfin_*`). The password is never stored, only the
/// access token.
fn jellyfin_rows(a: &nulamp_library::jellyfin::Account) -> Vec<(String, String)> {
    // Each library is `<0|1>|<collection type>|<id>|<name>` (shown, type, id, name);
    // libraries are separated by tabs.
    let libraries = a
        .libraries
        .iter()
        .map(|l| {
            format!(
                "{}|{}|{}|{}",
                b(l.enabled),
                list_value(&l.collection_type),
                list_value(&l.id),
                list_value(&l.name)
            )
        })
        .collect::<Vec<_>>()
        .join("\t");
    [
        ("jellyfin_server", a.server_url.clone()),
        ("jellyfin_server_id", a.server_id.clone()),
        ("jellyfin_server_name", a.server_name.clone()),
        ("jellyfin_user", a.user_name.clone()),
        ("jellyfin_user_id", a.user_id.clone()),
        ("jellyfin_token", a.token.clone()),
        ("jellyfin_device_id", a.device_id.clone()),
        ("jellyfin_libraries", libraries),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// The Jellyfin account of the settings file, when one is signed in.
fn jellyfin_account(
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<nulamp_library::jellyfin::Account> {
    let server_url = get("jellyfin_server").filter(|v| !v.trim().is_empty())?;
    let user_id = get("jellyfin_user_id").filter(|v| !v.trim().is_empty())?;
    let token = get("jellyfin_token").filter(|v| !v.trim().is_empty())?;
    let libraries = get("jellyfin_libraries")
        .unwrap_or_default()
        .split('\t')
        .filter_map(|item| {
            let mut f = item.splitn(4, '|');
            let (on, kind, id, name) = (f.next()?, f.next()?, f.next()?, f.next()?);
            (!id.is_empty()).then(|| nulamp_library::jellyfin::Library {
                id: id.to_string(),
                name: name.to_string(),
                collection_type: kind.to_string(),
                enabled: on.trim() == "1",
            })
        })
        .collect();
    Some(nulamp_library::jellyfin::Account {
        server_url: server_url.trim().to_string(),
        server_id: get("jellyfin_server_id").unwrap_or_default(),
        server_name: get("jellyfin_server_name").unwrap_or_default(),
        user_name: get("jellyfin_user").unwrap_or_default(),
        user_id: user_id.trim().to_string(),
        token: token.trim().to_string(),
        device_id: get("jellyfin_device_id").unwrap_or_default(),
        libraries,
    })
}

/// Parses the settings file over the defaults.
pub fn from_ini(text: &str) -> (UiPrefs, PlayerState) {
    let ini = Ini::parse(text);
    let mut p = UiPrefs::default();
    let mut st = PlayerState::default();
    let get = |k: &str| ini.get("nulamp", k).map(str::to_string);
    let num = |k: &str| get(k).and_then(|v| v.trim().parse::<i64>().ok());
    let flag = |k: &str, d: bool| get(k).map(|v| v.trim() == "1").unwrap_or(d);
    macro_rules! set_i {
        ($field:expr, $key:expr, $t:ty) => {
            if let Some(n) = num($key) {
                $field = n as $t;
            }
        };
    }
    set_i!(p.main_pos.0, "main_x", i32);
    set_i!(p.main_pos.1, "main_y", i32);
    set_i!(p.eq_pos.0, "eq_x", i32);
    set_i!(p.eq_pos.1, "eq_y", i32);
    set_i!(p.pl_pos.0, "pl_x", i32);
    set_i!(p.pl_pos.1, "pl_y", i32);
    set_i!(p.pl_size.0, "pl_width", i32);
    set_i!(p.pl_size.1, "pl_height", i32);
    set_i!(p.pl_font_px, "pl_font_px", u32);
    set_i!(p.pl_bitmap_scale, "pl_bitmap_scale", i32);
    set_i!(p.snap_distance, "snap_distance", i32);
    p.pl_shade = flag("pl_shade", p.pl_shade);
    p.pl_bitmap_font = flag("pl_bitmap_font", p.pl_bitmap_font);
    p.pl_show_numbers = flag("pl_show_numbers", p.pl_show_numbers);
    p.pl_zero_pad = flag("pl_zero_pad", p.pl_zero_pad);
    p.pl_wheel_double = flag("pl_wheel_double", p.pl_wheel_double);
    p.pl_old_school_buttons = flag("pl_old_school_buttons", p.pl_old_school_buttons);
    if let Some(n) = num("pl_page_lines") {
        p.pl_page_lines = n.clamp(1, 16) as usize;
    }
    if let Some(r) = get("recent_urls") {
        p.recent_urls = r
            .split('\t')
            .filter(|u| !u.is_empty())
            .map(str::to_string)
            .collect();
    }
    p.eq_visible = flag("eq_visible", p.eq_visible);
    p.pl_visible = flag("pl_visible", p.pl_visible);
    p.double_size = flag("double_size", p.double_size);
    p.eq_double = flag("eq_double", p.eq_double);
    p.shade = flag("shade", p.shade);
    p.eq_shade = flag("eq_shade", p.eq_shade);
    p.always_on_top = flag("always_on_top", p.always_on_top);
    p.easymove = flag("easymove", p.easymove);
    p.snap = flag("snap", p.snap);
    p.keep_on_screen = flag("keep_on_screen", p.keep_on_screen);
    p.dim_inactive = flag("dim_inactive", p.dim_inactive);
    p.clutterbar_always = flag("clutterbar_always", p.clutterbar_always);
    p.bitmap_title = flag("bitmap_title", p.bitmap_title);
    p.show_number_in_title = flag("show_number_in_title", p.show_number_in_title);
    p.autoscroll = flag("autoscroll", p.autoscroll);
    p.time_remaining = flag("time_remaining", p.time_remaining);
    p.user_font = get("user_font").filter(|f| !f.is_empty());
    p.skin_path = get("skin").filter(|f| !f.is_empty());
    if let Some(d) = get("skins_dir").filter(|d| !d.is_empty()) {
        p.skins_dir = PathBuf::from(d);
    }
    p.last_dir = get("last_dir").filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Some(bands) = get("eq_bands") {
        for (i, v) in bands.split(',').take(10).enumerate() {
            if let Ok(x) = v.trim().parse::<u8>() {
                p.eq.bands[i] = x.min(63);
            }
        }
    }
    if let Some(n) = num("eq_preamp") {
        p.eq.preamp = n.clamp(0, 63) as u8;
    }
    p.eq.enabled = flag("eq_enabled", p.eq.enabled);
    p.eq.auto_load = flag("eq_autoload", p.eq.auto_load);
    p.eq.iso = flag("eq_iso", p.eq.iso);
    p.eq.limiter = flag("eq_limiter", p.eq.limiter);
    let pick = |k: &str, n: usize| num(k).map(|v| (v.max(0) as usize).min(n - 1));
    if let Some(i) = pick("vis_mode", 3) {
        p.vis.mode = [VisMode::Off, VisMode::Spectrum, VisMode::Oscilloscope][i];
    }
    if let Some(i) = pick("vis_style", 3) {
        p.vis.style = [
            AnalyzerStyle::Normal,
            AnalyzerStyle::Fire,
            AnalyzerStyle::Line,
        ][i];
    }
    p.vis.thick_bands = flag("vis_thick", p.vis.thick_bands);
    p.vis.peaks = flag("vis_peaks", p.vis.peaks);
    if let Some(i) = pick("vis_falloff", 5) {
        p.vis.falloff = Speed::ALL[i];
    }
    if let Some(i) = pick("vis_peak_falloff", 5) {
        p.vis.peak_falloff = Speed::ALL[i];
    }
    if let Some(i) = pick("vis_scope", 3) {
        p.vis.scope = [ScopeStyle::Dots, ScopeStyle::Lines, ScopeStyle::Solid][i];
    }
    if let Some(i) = pick("vis_refresh", 4) {
        p.vis.refresh = [
            RefreshRate::Full,
            RefreshRate::Half,
            RefreshRate::Quarter,
            RefreshRate::Eighth,
        ][i];
    }
    p.replaygain.enabled = flag("rg_enabled", p.replaygain.enabled);
    if flag("rg_album", false) {
        p.replaygain.source = nulamp_eq::replaygain::Source::Album;
    }
    if let Some(i) = pick("rg_mode", 4) {
        use nulamp_eq::replaygain::Mode::*;
        p.replaygain.mode = [
            ApplyGain,
            ApplyGainPreventClipping,
            Normalize,
            PreventClipping,
        ][i];
    }
    p.replaygain.preferred_only = flag("rg_preferred_only", p.replaygain.preferred_only);
    let fnum = |k: &str| get(k).and_then(|v| v.trim().parse::<f32>().ok());
    if let Some(v) = fnum("rg_preamp_db") {
        p.replaygain.preamp_db = v.clamp(-14.0, 6.0);
    }
    if let Some(v) = fnum("rg_no_tag_db") {
        p.replaygain.no_tag_db = v.clamp(-14.0, 6.0);
    }
    p.fade_pause_stop = flag("fade_pause_stop", p.fade_pause_stop);
    p.fade_seek = flag("fade_seek", p.fade_seek);
    p.show_tray = flag("show_tray", p.show_tray);
    set_i!(p.lib_pos.0, "lib_x", i32);
    set_i!(p.lib_pos.1, "lib_y", i32);
    if let Some(n) = num("lib_width") {
        p.lib_size.0 = n.clamp(275, 10_000) as i32;
    }
    if let Some(n) = num("lib_height") {
        p.lib_size.1 = n.clamp(116, 10_000) as i32;
    }
    p.lib_visible = flag("lib_visible", p.lib_visible);
    set_i!(p.lib_tree_w, "lib_tree_width", i32);
    if let Some(c) = get("lib_columns") {
        p.lib_columns = c
            .split(',')
            .filter_map(|w| w.trim().parse::<i32>().ok())
            .map(|w| w.clamp(1, 2000))
            .collect();
    }
    if let Some(f) = get("library_folders") {
        // Each folder is `<0|1>|<path>` (include subfolders, path); folders are separated
        // by tabs.
        p.library.folders = f
            .split('\t')
            .filter_map(|item| {
                let (rec, path) = item.split_once('|')?;
                (!path.is_empty()).then(|| (path.to_string(), rec.trim() == "1"))
            })
            .collect();
    }
    p.library.rescan_on_start = flag("library_rescan_on_start", p.library.rescan_on_start);
    p.library.watch = flag("library_watch", p.library.watch);
    p.library.jellyfin = jellyfin_account(&get);
    set_i!(p.video_pos.0, "video_wx", i32);
    set_i!(p.video_pos.1, "video_wy", i32);
    if let Some(n) = num("video_width") {
        p.video_size.0 = n.clamp(275, 10_000) as i32;
    }
    if let Some(n) = num("video_height") {
        p.video_size.1 = n.clamp(116, 10_000) as i32;
    }
    p.video_visible = flag("video_open", p.video_visible);
    let v = &mut p.video;
    v.enabled = flag("video_enabled", v.enabled);
    v.open_on_start = flag("video_open_on_start", v.open_on_start);
    v.close_on_end = flag("video_close_on_end", v.close_on_end);
    v.stop_on_close = flag("video_stop_on_close", v.stop_on_close);
    v.resize_to_video = flag("video_resize", v.resize_to_video);
    v.fullscreen_on_start = flag("video_fullscreen_on_start", v.fullscreen_on_start);
    v.leave_fullscreen_on_stop = flag("video_leave_fullscreen", v.leave_fullscreen_on_stop);
    v.lock_aspect = flag("video_lock_aspect", v.lock_aspect);
    if let Some(a) = get("video_aspect").and_then(|t| nulamp_ui::video::parse_aspect(&t)) {
        v.aspect = a;
    }
    v.logo_when_idle = flag("video_logo", v.logo_when_idle);
    v.inhibit_screensaver = flag("video_no_screensaver", v.inhibit_screensaver);
    p.hotkeys.enabled = flag("hotkeys_enabled", p.hotkeys.enabled);
    if get("hotkey_0").is_some() {
        p.hotkeys.bindings = (0..)
            .map_while(|i| get(&format!("hotkey_{i}")))
            .filter_map(|v| {
                let (action, hotkey) = v.trim().split_once(' ')?;
                Some(nulamp_remote::hotkey::Binding {
                    hotkey: nulamp_remote::Hotkey::parse(hotkey)?,
                    action: nulamp_remote::Assignable::from_name(action)?,
                })
            })
            .collect();
    }
    if let Some(n) = num("buffer_ms") {
        p.buffer_ms = n.clamp(200, 20_000) as u32;
    }
    p.crossfade = flag("crossfade", p.crossfade);
    if let Some(n) = num("crossfade_ms") {
        p.crossfade_ms = n.clamp(0, nulamp_audio::engine::MAX_CROSSFADE_MS as i64) as u32;
    }
    p.crossfade_equal_power = flag("crossfade_equal_power", p.crossfade_equal_power);
    p.crossfade_manual = flag("crossfade_manual", p.crossfade_manual);
    p.gapless_albums = flag("gapless_albums", p.gapless_albums);
    if let Some(n) = num("volume") {
        st.volume = n.clamp(0, 255) as u8;
    }
    if let Some(n) = num("balance") {
        st.balance = n.clamp(-127, 127) as i8;
    }
    st.shuffle = flag("shuffle", st.shuffle);
    st.repeat = flag("repeat", st.repeat);
    st.manual_advance = flag("manual_advance", st.manual_advance);
    if let Some(n) = num("position") {
        st.position = n.max(0) as usize;
    }
    (p, st)
}

pub fn load(dir: &Path) -> (UiPrefs, PlayerState) {
    match std::fs::read(settings_path(dir)) {
        Ok(bytes) => from_ini(&String::from_utf8_lossy(&bytes)),
        Err(_) => (UiPrefs::default(), PlayerState::default()),
    }
}

/// Writes the settings atomically (temporary file, then rename).
pub fn save(dir: &Path, p: &UiPrefs, st: &PlayerState) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = settings_path(dir);
    let tmp = path.with_extension("ini.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new();
    file.write(true).create_new(true);
    // Only the user may read it: it can hold a server access token.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut file, 0o600);
    std::io::Write::write_all(&mut file.open(&tmp)?, to_ini(p, st).as_bytes())?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut p = UiPrefs {
            main_pos: (100, 200),
            double_size: true,
            skin_path: Some("/s/x.wsz".into()),
            ..UiPrefs::default()
        };
        p.eq.bands[3] = 5;
        p.vis.mode = VisMode::Oscilloscope;
        p.pl_size = (450, 290);
        p.hotkeys.enabled = true;
        p.hotkeys.bindings.truncate(3);
        p.hotkeys.bindings[2].hotkey = nulamp_remote::Hotkey::parse("Shift+F7").unwrap();
        p.crossfade = true;
        p.crossfade_ms = 5000;
        p.crossfade_equal_power = true;
        p.gapless_albums = false;
        let st = PlayerState {
            volume: 128,
            balance: -20,
            shuffle: true,
            position: 7,
            ..PlayerState::default()
        };
        let (p2, st2) = from_ini(&to_ini(&p, &st));
        assert_eq!(p2.main_pos, (100, 200));
        assert!(p2.double_size);
        assert_eq!(p2.skin_path.as_deref(), Some("/s/x.wsz"));
        assert_eq!(p2.eq.bands[3], 5);
        assert_eq!(p2.vis.mode, VisMode::Oscilloscope);
        assert_eq!(p2.pl_size, (450, 290));
        assert_eq!(p2.hotkeys, p.hotkeys);
        assert!(p2.crossfade && p2.crossfade_equal_power && !p2.crossfade_manual);
        assert_eq!(p2.crossfade_ms, 5000);
        assert!(!p2.gapless_albums);
        assert_eq!(st2, st);
    }

    #[test]
    fn defaults_when_missing() {
        let (p, st) = from_ini("");
        assert_eq!(p.main_pos, (26, 29));
        assert_eq!(st.volume, 200);
        assert_eq!(p.library, nulamp_ui::shell::LibraryPrefs::default());
        assert!(p.library.rescan_on_start && p.library.watch);
        assert!(!p.lib_visible);
    }

    #[test]
    fn media_library_round_trip() {
        let mut p = UiPrefs {
            lib_pos: (400, -20),
            lib_size: (600, 377),
            lib_visible: true,
            lib_tree_w: 150,
            lib_columns: vec![30, 120, 160, 0],
            ..UiPrefs::default()
        };
        p.library.folders = vec![("/music".into(), true), ("/mnt/share/a|b c".into(), false)];
        p.library.rescan_on_start = false;
        p.library.watch = false;
        let (p2, _) = from_ini(&to_ini(&p, &PlayerState::default()));
        assert_eq!(p2.lib_pos, (400, -20));
        assert_eq!(p2.lib_size, (600, 377));
        assert!(p2.lib_visible);
        assert_eq!(p2.lib_tree_w, 150);
        assert_eq!(p2.lib_columns, [30, 120, 160, 1]);
        assert_eq!(p2.library, p.library);
        // An empty folder list stays empty.
        p.library.jellyfin = Some(nulamp_library::jellyfin::Account {
            server_url: "https://jf.example:8920".into(),
            server_id: "srv".into(),
            server_name: "Home".into(),
            user_name: "me".into(),
            user_id: "u1".into(),
            token: "tok".into(),
            device_id: "dev".into(),
            libraries: vec![
                nulamp_library::jellyfin::Library {
                    id: "l1".into(),
                    name: "Music | Lossless\tA".into(),
                    collection_type: "music".into(),
                    enabled: true,
                },
                nulamp_library::jellyfin::Library {
                    id: "l2".into(),
                    name: "Films".into(),
                    collection_type: "movies".into(),
                    enabled: false,
                },
            ],
        });
        let (p2, _) = from_ini(&to_ini(&p, &PlayerState::default()));
        let a = p2.library.jellyfin.unwrap();
        assert_eq!(a.libraries[0].name, "Music   Lossless A");
        let mut expected = p.library.jellyfin.clone().unwrap();
        expected.libraries[0].name = a.libraries[0].name.clone();
        assert_eq!(a, expected);
        p.library.jellyfin = None;
        p.library.folders.clear();
        let (p3, _) = from_ini(&to_ini(&p, &PlayerState::default()));
        assert!(p3.library.folders.is_empty());
    }

    #[test]
    fn video_round_trip() {
        let (d, _) = from_ini("");
        assert_eq!(d.video, nulamp_ui::video::VideoPrefs::default());
        assert!(!d.video_visible);
        let mut p = UiPrefs {
            video_pos: (500, 40),
            video_size: (400, 319),
            video_visible: true,
            ..UiPrefs::default()
        };
        p.video.enabled = false;
        p.video.stop_on_close = true;
        p.video.lock_aspect = false;
        p.video.aspect = Some(16.0 / 9.0);
        p.video.inhibit_screensaver = false;
        let ini = to_ini(&p, &PlayerState::default());
        assert!(
            ini.contains("video_wx=500") && ini.contains("video_open=1"),
            "{ini}"
        );
        let (p2, _) = from_ini(&ini);
        assert_eq!((p2.video_pos, p2.video_size), ((500, 40), (400, 319)));
        assert!(p2.video_visible);
        assert_eq!(p2.video, p.video);
    }
}
