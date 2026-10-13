//! Menu contents (02 §12.7, 03 §5.3, 04 §6.3–6.4, 06 §6.2, 11 §3).

use crate::input::{Action, MenuKind};
use crate::keys::KeyAction;
use crate::library::LibCmd;
use crate::menu::{Item, MenuCmd};
use crate::playlist::PlMenuChoice;
use crate::wayland_limits;
use nulamp_core::Command;
use nulamp_library::Capabilities;
use nulamp_render::playlist_window::PopupMenu;

/// State shown by check marks and lists in menus.
#[derive(Debug, Clone, Default)]
pub struct MenuState {
    pub main_visible: bool,
    pub eq_visible: bool,
    pub pl_visible: bool,
    /// The media library exists (and the menu offers its window).
    pub has_library: bool,
    pub lib_visible: bool,
    pub video_visible: bool,
    pub always_on_top: bool,
    pub double_size: bool,
    pub easymove: bool,
    pub repeat: bool,
    pub shuffle: bool,
    pub manual_advance: bool,
    pub stop_after_current: bool,
    pub time_remaining: bool,
    pub autoscroll: bool,
    pub playing: bool,
    /// 0 off, 1 spectrum, 2 oscilloscope.
    pub vis_mode: u8,
    /// 0 normal, 1 fire, 2 line.
    pub analyzer_style: u8,
    pub thick_bands: bool,
    pub peaks: bool,
    /// 0 dots, 1 lines, 2 solid.
    pub scope_style: u8,
    /// 0..=3 (full, half, quarter, eighth).
    pub refresh: u8,
    /// 0..=4.
    pub falloff: u8,
    pub peak_falloff: u8,
    /// (display name, path or None for the built-in skin).
    pub skins: Vec<(String, Option<String>)>,
    pub current_skin: Option<String>,
    pub bookmarks: Vec<(String, String)>,
    /// Audio CD drives: (whole-disc location, label).
    pub cd_drives: Vec<(String, String)>,
    pub eq_enabled: bool,
    pub eq_auto: bool,
    pub eq_presets: Vec<String>,
    pub app_name: String,
    /// Running as a native Wayland window: items Wayland cannot support are greyed out.
    pub wayland_native: bool,
}

fn act(a: Action) -> MenuCmd {
    MenuCmd::Act(a)
}

fn player(c: Command) -> MenuCmd {
    MenuCmd::Act(Action::Player(c))
}

fn radio(label: &str, on: bool, cmd: MenuCmd) -> Item {
    Item::check(label, "", on, cmd)
}

pub fn skins_menu(s: &MenuState) -> Vec<Item> {
    let mut v = vec![
        Item::key(
            "Skin Browser...",
            "Alt+S",
            MenuCmd::Key(KeyAction::SkinBrowser),
        ),
        Item::Sep,
    ];
    for (name, path) in &s.skins {
        let label = if name.is_empty() { "<Base Skin>" } else { name };
        v.push(radio(
            label,
            *path == s.current_skin,
            MenuCmd::LoadSkin(path.clone()),
        ));
    }
    v
}

pub fn options_menu(s: &MenuState) -> Vec<Item> {
    vec![
        Item::key(
            "Preferences...",
            "Ctrl+P",
            MenuCmd::Key(KeyAction::Preferences),
        ),
        Item::sub("Skins", skins_menu(s)),
        Item::cmd("Plugins...", MenuCmd::Plugins),
        Item::Sep,
        Item::check(
            "Time elapsed",
            "Ctrl+T",
            !s.time_remaining,
            MenuCmd::SetTimeRemaining(false),
        ),
        Item::check(
            "Time remaining",
            "Ctrl+T",
            s.time_remaining,
            MenuCmd::SetTimeRemaining(true),
        ),
        Item::Sep,
        Item::check(
            "Always On Top",
            "Ctrl+A",
            s.always_on_top,
            act(Action::ToggleAlwaysOnTop),
        )
        .disabled_if(s.wayland_native, wayland_limits::ALWAYS_ON_TOP),
        Item::check(
            "Double Size",
            "Ctrl+D",
            s.double_size,
            act(Action::ToggleDoubleSize),
        ),
        Item::check(
            "EasyMove",
            "Ctrl+E",
            s.easymove,
            MenuCmd::Key(KeyAction::ToggleEasyMove),
        ),
        Item::Sep,
        Item::check("Repeat", "R", s.repeat, act(Action::ToggleRepeat)),
        Item::check("Shuffle", "S", s.shuffle, act(Action::ToggleShuffle)),
    ]
}

pub fn playback_menu(s: &MenuState) -> Vec<Item> {
    vec![
        Item::key("Previous", "Z", player(Command::Previous)),
        Item::key("Play", "X", player(Command::Play)),
        Item::key("Pause", "C", player(Command::Pause)),
        Item::key("Stop", "V", player(Command::Stop)),
        Item::key("Next", "B", player(Command::Next)),
        Item::Sep,
        Item::key(
            "Stop with fade-out",
            "Shift+V",
            player(Command::StopWithFade),
        ),
        Item::check(
            "Stop after current",
            "Ctrl+V",
            s.stop_after_current,
            player(Command::ToggleStopAfterCurrent),
        ),
        Item::key(
            "Back 5 seconds",
            "Left",
            player(Command::SeekRelative(-5000)),
        ),
        Item::key(
            "Forward 5 seconds",
            "Right",
            player(Command::SeekRelative(5000)),
        ),
        Item::key("Start of list", "Ctrl+Z", player(Command::StartOfList)),
        Item::key("End of list", "Ctrl+B", player(Command::EndOfList)),
        Item::key("10 tracks back", "Num 1", player(Command::JumpEntries(-10))),
        Item::key(
            "10 tracks forward",
            "Num 3",
            player(Command::JumpEntries(10)),
        ),
        Item::Sep,
        Item::key(
            "Jump to time",
            "Ctrl+J",
            MenuCmd::Key(KeyAction::JumpToTime),
        ),
        Item::key("Jump to file", "J", MenuCmd::Key(KeyAction::JumpToFile)),
    ]
}

pub fn vis_menu(s: &MenuState) -> Vec<Item> {
    let names = ["Slowest", "Slow", "Moderate", "Fast", "Fastest"];
    vec![
        Item::sub(
            "Visualization mode",
            vec![
                radio("Spectrum analyzer", s.vis_mode == 1, MenuCmd::SetVisMode(1)),
                radio("Oscilloscope", s.vis_mode == 2, MenuCmd::SetVisMode(2)),
                radio("Off", s.vis_mode == 0, MenuCmd::SetVisMode(0)),
            ],
        ),
        Item::sub(
            "Analyzer options",
            vec![
                radio(
                    "Normal style",
                    s.analyzer_style == 0,
                    MenuCmd::SetAnalyzerStyle(0),
                ),
                radio(
                    "Fire style",
                    s.analyzer_style == 1,
                    MenuCmd::SetAnalyzerStyle(1),
                ),
                radio(
                    "Line style",
                    s.analyzer_style == 2,
                    MenuCmd::SetAnalyzerStyle(2),
                ),
                Item::Sep,
                radio("Thin bands", !s.thick_bands, MenuCmd::SetBands(false)),
                radio("Thick bands", s.thick_bands, MenuCmd::SetBands(true)),
                Item::Sep,
                Item::check("Show peaks", "", s.peaks, MenuCmd::TogglePeaks),
            ],
        ),
        Item::sub(
            "Oscilloscope options",
            vec![
                radio("Dot scope", s.scope_style == 0, MenuCmd::SetScopeStyle(0)),
                radio("Line scope", s.scope_style == 1, MenuCmd::SetScopeStyle(1)),
                radio("Solid scope", s.scope_style == 2, MenuCmd::SetScopeStyle(2)),
            ],
        ),
        Item::sub(
            "Refresh rate",
            [
                "Full (70 fps)",
                "Half (35 fps)",
                "Quarter (18 fps)",
                "Eighth (9 fps)",
            ]
            .iter()
            .enumerate()
            .map(|(i, n)| radio(n, s.refresh == i as u8, MenuCmd::SetRefresh(i as u8)))
            .collect(),
        ),
        Item::sub(
            "Analyzer falloff",
            names
                .iter()
                .enumerate()
                .map(|(i, n)| radio(n, s.falloff == i as u8, MenuCmd::SetFalloff(i as u8)))
                .collect(),
        ),
        Item::sub(
            "Peak falloff",
            names
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    radio(
                        n,
                        s.peak_falloff == i as u8,
                        MenuCmd::SetPeakFalloff(i as u8),
                    )
                })
                .collect(),
        ),
        Item::Sep,
        Item::key(
            "Start/stop plugin",
            "Ctrl+Shift+K",
            act(Action::ToggleVisPlugin),
        ),
        Item::key("Configure plugin...", "Alt+K", MenuCmd::None),
        Item::key(
            "Select plugin...",
            "Ctrl+K",
            MenuCmd::Key(KeyAction::Preferences),
        ),
    ]
}

fn bookmark_items(s: &MenuState) -> Vec<Item> {
    if s.bookmarks.is_empty() {
        return vec![Item::disabled("(no bookmarks)")];
    }
    s.bookmarks
        .iter()
        .map(|(loc, title)| Item::cmd(title, MenuCmd::Bookmark(loc.clone())))
        .collect()
}

pub fn main_menu(s: &MenuState) -> Vec<Item> {
    let mut bookmarks = vec![
        Item::key(
            "Edit bookmarks...",
            "Ctrl+Alt+I",
            MenuCmd::Key(KeyAction::EditBookmarks),
        ),
        Item::key(
            "Add current as bookmark",
            "Ctrl+Alt+B",
            MenuCmd::Key(KeyAction::BookmarkCurrent),
        ),
        Item::Sep,
    ];
    bookmarks.extend(bookmark_items(s));
    let mut play = vec![
        Item::key("File...", "L", act(Action::OpenFiles)),
        Item::key("URL...", "Ctrl+L", act(Action::OpenUrl)),
        Item::key("Folder...", "Shift+L", act(Action::OpenFolder)),
        Item::Sep,
    ];
    play.extend(
        s.cd_drives.iter().map(|(loc, label)| {
            Item::cmd(&format!("Audio CD {label}"), MenuCmd::PlayCd(loc.clone()))
        }),
    );
    play.push(Item::sub("Bookmark", bookmark_items(s)));
    let library = s.has_library.then(|| {
        Item::check(
            "Media Library",
            "Alt+L",
            s.lib_visible,
            act(Action::ToggleLibraryWindow),
        )
    });
    let mut items = vec![
        Item::key(
            &format!("About {}...", s.app_name),
            "Ctrl+F1",
            act(Action::About),
        ),
        Item::Sep,
        Item::sub("Play", play),
        Item::key("View file info...", "Alt+3", act(Action::FileInfoCurrent)),
        Item::sub("Bookmarks", bookmarks),
        Item::Sep,
        Item::check(
            "Main Window",
            "Alt+W",
            s.main_visible,
            MenuCmd::Key(KeyAction::ToggleMainWindow),
        ),
        Item::check(
            "Playlist Editor",
            "Alt+E",
            s.pl_visible,
            act(Action::TogglePlaylistWindow),
        ),
        Item::check(
            "Equalizer",
            "Alt+G",
            s.eq_visible,
            act(Action::ToggleEqWindow),
        ),
        Item::check(
            "Video",
            "Alt+V",
            s.video_visible,
            MenuCmd::Key(KeyAction::ToggleVideoWindow),
        ),
        Item::Sep,
        Item::sub("Options", options_menu(s)),
        Item::sub("Playback", playback_menu(s)),
        Item::sub("Visualization", vis_menu(s)),
        Item::sub("Skins", skins_menu(s)),
        Item::Sep,
        Item::key("Help", "F1", MenuCmd::Key(KeyAction::Help)),
        Item::Sep,
        Item::key("Exit", "Alt+F4", act(Action::Exit)),
    ];
    if let Some(lib) = library {
        // After "Video", with the other window toggles.
        let at = items
            .iter()
            .position(|i| matches!(i, Item::Cmd { label, .. } if label == "Video"))
            .map_or(items.len(), |p| p + 1);
        items.insert(at, lib);
    }
    items
}

/// Track-list menu of the media library. `playlists` are the stored playlists of the
/// listing's provider (id, name); `caps` its capabilities.
pub fn library_context(playlists: &[(String, String)], caps: Capabilities) -> Vec<Item> {
    use crate::library::Purpose::*;
    let sel = |p| MenuCmd::Lib(LibCmd::Selection(p));
    let mut items = vec![
        Item::key("Play", "Enter", sel(Play)),
        Item::key("Enqueue", "Shift+Enter", sel(Enqueue)),
        Item::key("Play next", "Q", sel(PlayNext)),
    ];
    if caps.contains(Capabilities::PLAYLISTS) {
        let mut sub = vec![
            Item::cmd("New playlist...", sel(AddToPlaylist(None))),
            Item::Sep,
        ];
        if playlists.is_empty() {
            sub.push(Item::disabled("(no playlists)"));
        }
        sub.extend(
            playlists
                .iter()
                .map(|(id, name)| Item::cmd(name, sel(AddToPlaylist(Some(id.clone()))))),
        );
        items.push(Item::sub("Add to playlist", sub));
    }
    items.push(Item::Sep);
    items.push(Item::key("View file info...", "Alt+3", sel(FileInfo)));
    if caps.contains(Capabilities::WRITE_RATING) {
        items.push(Item::sub(
            "Rate",
            (0..=5)
                .rev()
                .map(|r| Item::cmd(&rating_label(r), sel(Rate(r))))
                .collect(),
        ));
    }
    if !caps.contains(Capabilities::REMOTE) {
        items.push(Item::key("Show in folder", "Ctrl+F", sel(ShowInFolder)));
    }
    items.push(Item::Sep);
    items.push(Item::key(
        "Select all",
        "Ctrl+A",
        MenuCmd::Lib(LibCmd::SelectAll),
    ));
    if caps.contains(Capabilities::REMOVE) {
        items.push(Item::key("Remove from library", "Del", sel(Remove)));
    }
    items
}

/// Menu of the Play button's arrow: act on the selection, or on everything without one.
pub fn library_play_menu() -> Vec<Item> {
    use crate::library::Purpose::*;
    let auto = |p| MenuCmd::Lib(LibCmd::Auto(p));
    vec![
        Item::cmd("Play", auto(Play)),
        Item::cmd("Enqueue", auto(Enqueue)),
        Item::cmd("Play next", auto(PlayNext)),
    ]
}

/// Menu of a media-library tree node: act on everything the node lists.
pub fn library_tree_menu(caps: Capabilities) -> Vec<Item> {
    use crate::library::Purpose::*;
    let all = |p| MenuCmd::Lib(LibCmd::All(p));
    let mut items = vec![
        Item::cmd("Play all", all(Play)),
        Item::cmd("Enqueue all", all(Enqueue)),
    ];
    if caps.contains(Capabilities::SCAN) {
        items.push(Item::Sep);
        items.push(Item::cmd("Rescan library", MenuCmd::Lib(LibCmd::Rescan)));
    }
    items
}

/// Menus opened from main-window areas (02 §12.7).
pub fn for_kind(kind: MenuKind, s: &MenuState) -> Vec<Item> {
    match kind {
        MenuKind::Main => main_menu(s),
        MenuKind::Options => options_menu(s),
        MenuKind::Visualization => vis_menu(s),
        MenuKind::SongTitle => vec![
            Item::key("View file info...", "Alt+3", act(Action::FileInfoCurrent)),
            Item::key("Jump to file", "J", MenuCmd::Key(KeyAction::JumpToFile)),
            Item::key(
                "Jump to time",
                "Ctrl+J",
                MenuCmd::Key(KeyAction::JumpToTime),
            ),
            Item::check("Auto-scroll song title", "", s.autoscroll, MenuCmd::None),
            Item::sub(
                "Rating",
                (0..=5)
                    .rev()
                    .map(|r| Item::cmd(&rating_label(r), MenuCmd::Rate(r)))
                    .collect(),
            ),
        ],
        MenuKind::TimeMode => vec![
            radio(
                "Time elapsed",
                !s.time_remaining,
                MenuCmd::SetTimeRemaining(false),
            ),
            radio(
                "Time remaining",
                s.time_remaining,
                MenuCmd::SetTimeRemaining(true),
            ),
        ],
        MenuKind::Previous => vec![
            Item::key("Previous", "Z", player(Command::Previous)),
            Item::key("Start of list", "Ctrl+Z", player(Command::StartOfList)),
            Item::key(
                "Rewind 5 seconds",
                "Shift+Z",
                player(Command::SeekRelative(-5000)),
            ),
        ],
        MenuKind::Play => vec![
            Item::key("Play/restart", "X", player(Command::Play)),
            Item::key("Open URL...", "Ctrl+X", act(Action::OpenUrl)),
            Item::key("Open file...", "Shift+X", act(Action::OpenFiles)),
        ],
        MenuKind::Pause => vec![Item::key("Pause/unpause", "C", player(Command::Pause))],
        MenuKind::Stop => vec![
            Item::key("Stop", "V", player(Command::Stop)),
            Item::key(
                "Stop with fade-out",
                "Shift+V",
                player(Command::StopWithFade),
            ),
            Item::check(
                "Stop after current",
                "Ctrl+V",
                s.stop_after_current,
                player(Command::ToggleStopAfterCurrent),
            ),
        ],
        MenuKind::Next => vec![
            Item::key("Next", "B", player(Command::Next)),
            Item::key(
                "Fast-forward 5 seconds",
                "Shift+B",
                player(Command::SeekRelative(5000)),
            ),
            Item::key("End of list", "Ctrl+B", player(Command::EndOfList)),
        ],
        MenuKind::Eject => vec![
            Item::key("Open file...", "L", act(Action::OpenFiles)),
            Item::key("Open folder...", "Shift+L", act(Action::OpenFolder)),
            Item::key("Open URL...", "Ctrl+L", act(Action::OpenUrl)),
        ],
        MenuKind::Seek => vec![
            Item::key(
                "Jump to time",
                "Ctrl+J",
                MenuCmd::Key(KeyAction::JumpToTime),
            ),
            Item::key(
                "Rewind 5 seconds",
                "Left",
                player(Command::SeekRelative(-5000)),
            ),
            Item::key(
                "Forward 5 seconds",
                "Right",
                player(Command::SeekRelative(5000)),
            ),
        ],
        MenuKind::Shuffle => vec![Item::check(
            "Shuffle",
            "S",
            s.shuffle,
            act(Action::ToggleShuffle),
        )],
        MenuKind::Repeat => vec![
            Item::check("Repeat", "R", s.repeat, act(Action::ToggleRepeat)),
            Item::check(
                "Manual playlist advance",
                "Shift+R",
                s.manual_advance,
                MenuCmd::Key(KeyAction::ToggleManualAdvance),
            ),
        ],
        MenuKind::EqToggle => vec![Item::check(
            "Graphical equalizer",
            "Alt+G",
            s.eq_visible,
            act(Action::ToggleEqWindow),
        )],
        MenuKind::PlToggle => {
            vec![Item::check(
                "Playlist editor",
                "Alt+E",
                s.pl_visible,
                act(Action::TogglePlaylistWindow),
            )]
        }
    }
}

fn rating_label(r: u8) -> String {
    if r == 0 {
        "No rating".into()
    } else {
        "★".repeat(r as usize)
    }
}

/// Playlist standard menus (04 §6.3).
pub fn playlist_menu(menu: PopupMenu) -> Vec<Item> {
    use PlMenuChoice::*;
    match menu {
        PopupMenu::Add => vec![
            Item::key("Add file(s)...", "L", MenuCmd::Pl(AddFile)),
            Item::key("Add folder...", "Shift+L", MenuCmd::Pl(AddFolder)),
            Item::key("Add URL...", "Ctrl+L", MenuCmd::Pl(AddUrl)),
        ],
        PopupMenu::Rem => vec![
            Item::key("Remove selected", "Del", MenuCmd::Pl(RemoveSelected)),
            Item::key("Crop selected", "Ctrl+Del", MenuCmd::Pl(Crop)),
            Item::key("Clear playlist", "Ctrl+Shift+Del", MenuCmd::Pl(RemoveAll)),
            Item::sub("Remove", remove_misc()),
        ],
        PopupMenu::Sel => vec![
            Item::key("Select all", "Ctrl+A", MenuCmd::Pl(SelectAll)),
            Item::cmd("Select none", MenuCmd::Pl(SelectNone)),
            Item::key("Invert selection", "Ctrl+I", MenuCmd::Pl(InvertSelection)),
        ],
        PopupMenu::Misc => vec![
            Item::sub("File info", file_info_menu()),
            Item::sub("Sort", sort_menu()),
            Item::sub("Misc", misc_menu()),
        ],
        PopupMenu::List => vec![
            Item::key("Open playlist...", "Ctrl+O", MenuCmd::Pl(LoadList)),
            Item::key("Save playlist...", "Ctrl+S", MenuCmd::Pl(SaveList)),
            Item::key("New playlist (clear)", "Ctrl+N", MenuCmd::Pl(NewList)),
        ],
    }
}

pub fn remove_misc() -> Vec<Item> {
    vec![
        Item::key("Remove missing files", "Alt+Del", MenuCmd::PlRemoveMissing),
        Item::cmd("Remove duplicate entries", MenuCmd::PlRemoveDuplicates),
        Item::cmd(
            "Physically remove selected file(s)",
            MenuCmd::PlPhysicallyRemove,
        ),
    ]
}

pub fn file_info_menu() -> Vec<Item> {
    vec![
        Item::key("File info...", "Alt+3", MenuCmd::PlFileInfo),
        Item::key("Playlist entry...", "Ctrl+E", MenuCmd::PlEditEntry),
    ]
}

pub fn sort_menu() -> Vec<Item> {
    vec![
        Item::key("Sort list by title", "Ctrl+Shift+1", MenuCmd::PlSortTitle),
        Item::key(
            "Sort list by file name",
            "Ctrl+Shift+2",
            MenuCmd::PlSortFileName,
        ),
        Item::key(
            "Sort list by path and file name",
            "Ctrl+Shift+3",
            MenuCmd::PlSortPath,
        ),
        Item::Sep,
        Item::key("Reverse list", "Ctrl+R", MenuCmd::PlReverse),
        Item::key("Randomize list", "Ctrl+Shift+R", MenuCmd::PlRandomize),
    ]
}

pub fn misc_menu() -> Vec<Item> {
    vec![
        Item::key(
            "Generate HTML playlist",
            "Ctrl+Alt+G",
            MenuCmd::PlGenerateHtml,
        ),
        Item::Sep,
        Item::key(
            "Rebuild titles on selection",
            "Ctrl+Alt+E",
            MenuCmd::PlRebuildTitles,
        ),
    ]
}

/// List context menu (04 §6.4).
pub fn list_context() -> Vec<Item> {
    vec![
        Item::key("Play item(s)", "Enter", MenuCmd::PlPlaySelected),
        Item::key("Enqueue item(s)", "Q", MenuCmd::PlEnqueueSelected),
        Item::Sep,
        Item::key(
            "Remove item(s)",
            "Del",
            MenuCmd::Pl(PlMenuChoice::RemoveSelected),
        ),
        Item::key("Crop files", "Ctrl+Del", MenuCmd::Pl(PlMenuChoice::Crop)),
        Item::Sep,
        Item::key("View file info...", "Alt+3", MenuCmd::PlFileInfo),
        Item::key("Playlist entry...", "Ctrl+E", MenuCmd::PlEditEntry),
        Item::Sep,
        Item::sub(
            "Rate items",
            (0..=5)
                .rev()
                .map(|r| Item::cmd(&rating_label(r), MenuCmd::PlRateSelected(r)))
                .collect(),
        ),
        Item::Sep,
        Item::key("Show item(s) in folder", "Ctrl+F", MenuCmd::PlShowInFolder),
    ]
}

/// EQ presets menu (03 §5.3).
pub fn presets_menu(s: &MenuState) -> Vec<Item> {
    let load: Vec<Item> = s
        .eq_presets
        .iter()
        .map(|n| Item::cmd(n, MenuCmd::EqLoadPreset(n.clone())))
        .collect();
    let delete: Vec<Item> = s
        .eq_presets
        .iter()
        .map(|n| Item::cmd(n, MenuCmd::EqDeletePreset(n.clone())))
        .collect();
    vec![
        Item::sub(
            "Load",
            vec![
                Item::sub(
                    "Preset",
                    if load.is_empty() {
                        vec![Item::disabled("(none)")]
                    } else {
                        load
                    },
                ),
                Item::cmd("Auto-load preset", MenuCmd::EqLoadAutoPreset),
                Item::cmd("Default", MenuCmd::EqLoadDefault),
                Item::Sep,
                Item::cmd("From EQF...", MenuCmd::EqImportEqf),
            ],
        ),
        Item::sub(
            "Save",
            vec![
                Item::cmd("Preset...", MenuCmd::EqSavePreset),
                Item::cmd("Auto-load preset", MenuCmd::EqSaveAutoPreset),
                Item::cmd("Default", MenuCmd::EqSaveDefault),
                Item::Sep,
                Item::cmd("To EQF...", MenuCmd::EqExportEqf),
            ],
        ),
        Item::sub(
            "Delete",
            if delete.is_empty() {
                vec![Item::disabled("(none)")]
            } else {
                delete
            },
        ),
    ]
}

pub fn eq_toggle_menu(auto: bool, s: &MenuState) -> Vec<Item> {
    if auto {
        vec![Item::check(
            "EQ auto-loading enabled",
            "A",
            s.eq_auto,
            MenuCmd::EqAutoLoad,
        )]
    } else {
        vec![Item::check(
            "EQ enabled",
            "N",
            s.eq_enabled,
            MenuCmd::EqEnabled,
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                Item::Cmd { label, .. } | Item::Sub { label, .. } => label.clone(),
                Item::Sep => "-".into(),
            })
            .collect()
    }

    fn play_menu(s: &MenuState) -> Vec<Item> {
        main_menu(s)
            .into_iter()
            .find_map(|i| match i {
                Item::Sub { label, items } if label == "Play" => Some(items),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn play_menu_lists_cd_drives() {
        let none = play_menu(&MenuState::default());
        assert_eq!(
            labels(&none),
            ["File...", "URL...", "Folder...", "-", "Bookmark"]
        );

        let s = MenuState {
            cd_drives: vec![
                ("cda://sr0".into(), "/dev/sr0".into()),
                ("cda://sr1".into(), "/dev/sr1".into()),
            ],
            ..MenuState::default()
        };
        let items = play_menu(&s);
        assert_eq!(
            labels(&items),
            [
                "File...",
                "URL...",
                "Folder...",
                "-",
                "Audio CD /dev/sr0",
                "Audio CD /dev/sr1",
                "Bookmark",
            ]
        );
        assert!(matches!(
            &items[5],
            Item::Cmd { cmd: MenuCmd::PlayCd(loc), .. } if loc == "cda://sr1"
        ));
    }

    #[test]
    fn always_on_top_under_wayland() {
        let on_top = |s: &MenuState| {
            options_menu(s)
                .into_iter()
                .find(|i| matches!(i, Item::Cmd { label, .. } if label == "Always On Top"))
                .unwrap()
        };
        let mut s = MenuState {
            always_on_top: true,
            ..MenuState::default()
        };
        assert_eq!(
            on_top(&s),
            Item::check(
                "Always On Top",
                "Ctrl+A",
                true,
                act(Action::ToggleAlwaysOnTop)
            )
        );
        s.wayland_native = true;
        assert_eq!(
            on_top(&s),
            Item::Cmd {
                label: "Always On Top".into(),
                shortcut: "Ctrl+A".into(),
                cmd: act(Action::ToggleAlwaysOnTop),
                check: Some(true),
                enabled: false,
                tip: Some(wayland_limits::ALWAYS_ON_TOP.into()),
            }
        );
    }

    #[test]
    fn library_items() {
        let mut s = MenuState::default();
        assert!(!labels(&main_menu(&s)).contains(&"Media Library".to_string()));
        s.has_library = true;
        s.lib_visible = true;
        let main = main_menu(&s);
        let l = labels(&main);
        let at = l.iter().position(|x| x == "Media Library").unwrap();
        assert_eq!(l[at - 1], "Video");
        let local = Capabilities::SCAN
            | Capabilities::WRITE_RATING
            | Capabilities::PLAYLISTS
            | Capabilities::REMOVE;
        let ctx = labels(&library_context(&[("1".into(), "Mix".into())], local));
        for want in [
            "Play",
            "Enqueue",
            "Play next",
            "Add to playlist",
            "Rate",
            "Show in folder",
            "Remove from library",
        ] {
            assert!(ctx.contains(&want.to_string()), "{want}");
        }
        let remote = labels(&library_context(&[], Capabilities::REMOTE));
        assert!(!remote.contains(&"Rate".to_string()));
        assert!(!remote.contains(&"Show in folder".to_string()));
        assert!(!remote.contains(&"Remove from library".to_string()));
        assert_eq!(
            labels(&library_tree_menu(local))
                .iter()
                .filter(|l| *l != "-")
                .count(),
            3
        );
    }
}
