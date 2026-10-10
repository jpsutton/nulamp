//! Native file dialogs and the actions that use them (10 §2.1).

use std::path::PathBuf;

use super::{Kind, PlayerHandle, Shell};
use crate::files;
use nulamp_core::Command;

impl<P: PlayerHandle> Shell<P> {
    fn media_dialog(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(dir) = &self.prefs.last_dir {
            d = d.set_directory(dir);
        }
        let mut all: Vec<String> = Vec::new();
        let groups = self.player.extensions();
        for (exts, _) in &groups {
            all.extend(exts.split(';').map(str::to_string));
        }
        all.extend(["m3u", "m3u8", "pls", "xspf", "b4s", "asx", "wpl"].map(String::from));
        let all_refs: Vec<&str> = all.iter().map(String::as_str).collect();
        d = d.add_filter("All supported types", &all_refs);
        for (exts, desc) in &groups {
            let v: Vec<&str> = exts.split(';').collect();
            d = d.add_filter(desc, &v);
        }
        d.add_filter("All files", &["*"])
    }

    fn remember_dir(&mut self, p: &std::path::Path) {
        let dir = if p.is_dir() {
            p.to_path_buf()
        } else {
            p.parent().map(PathBuf::from).unwrap_or_default()
        };
        self.prefs.last_dir = Some(dir);
    }

    /// Expands opened or dropped items into entries (10 §2.5).
    pub(super) fn expand(&self, items: &[String]) -> Vec<nulamp_core::NewEntry> {
        let mut out = Vec::new();
        for i in items {
            if let Some(entries) = self.player.expand_location(i) {
                out.extend(entries);
                continue;
            }
            out.extend(files::add_anything(i, true, false, &|ext| {
                self.player.is_supported_extension(ext)
            }));
        }
        out
    }

    /// Replaces the list with `items` and plays (open) or appends them (add). Playlist URLs
    /// are downloaded in the background and added when they arrive.
    pub(super) fn add_items(&mut self, items: Vec<String>, replace: bool) {
        let mut local = Vec::new();
        for item in items {
            let is_url = item.contains("://") && !item.starts_with("file://");
            match is_url && files::is_playlist_location(&item) {
                true => match self.player.resolve_playlist_url(&item) {
                    Some(rx) => self.url_expansions.push((rx, replace)),
                    None => local.push(item),
                },
                false => local.push(item),
            }
        }
        self.add_expanded(local, replace);
    }

    /// Adds playlist entries downloaded from a playlist URL.
    pub(super) fn poll_url_expansions(&mut self) {
        let mut done = Vec::new();
        self.url_expansions
            .retain(|(rx, replace)| match rx.try_recv() {
                Ok(items) => {
                    done.push((items, *replace));
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
            });
        for (items, replace) in done {
            self.add_expanded(items, replace);
        }
    }

    fn add_expanded(&mut self, items: Vec<String>, replace: bool) {
        let entries = self.expand(&items);
        if entries.is_empty() {
            return;
        }
        let e = self.player.engine_mut();
        if replace {
            e.clear();
        }
        e.append(entries);
        if replace {
            self.player.command(Command::StartOfList);
            self.player.command(Command::PlayIndex(
                self.player.engine().playlist.current_index(),
            ));
        }
        let n = self.player.engine().playlist.len();
        self.pl.clamp_scroll(n);
        self.poll_player();
        self.request_redraw_all();
    }

    pub(super) fn dialog_open_files(&mut self, replace: bool) {
        if let Some(paths) = self
            .media_dialog()
            .set_title(if replace {
                "Open file(s)"
            } else {
                "Add file(s)"
            })
            .pick_files()
        {
            if let Some(p) = paths.first() {
                self.remember_dir(p);
            }
            let items = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            self.add_items(items, replace);
        }
    }

    pub(super) fn dialog_open_folder(&mut self, replace: bool) {
        let mut d =
            rfd::FileDialog::new().set_title(if replace { "Open folder" } else { "Add folder" });
        if let Some(dir) = &self.prefs.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(dir) = d.pick_folder() {
            self.remember_dir(&dir);
            self.add_items(vec![dir.to_string_lossy().into_owned()], replace);
        }
    }

    pub(super) fn dialog_load_playlist(&mut self) {
        let mut d = rfd::FileDialog::new()
            .set_title("Open playlist")
            .add_filter(
                "Playlists",
                &[
                    "m3u", "m3u8", "pls", "xspf", "b4s", "asx", "wax", "wvx", "wpl", "zpl",
                ],
            );
        if let Some(dir) = &self.prefs.last_dir {
            d = d.set_directory(dir);
        }
        let Some(path) = d.pick_file() else { return };
        self.remember_dir(&path);
        match files::load_playlist(&path.to_string_lossy()) {
            Ok(entries) => {
                let e = self.player.engine_mut();
                e.clear();
                e.append(entries);
                self.pl.first = 0;
            }
            Err(err) => log::error!("cannot open playlist {}: {err}", path.display()),
        }
        self.poll_player();
        self.request_redraw(Kind::Playlist);
    }

    pub(super) fn dialog_save_playlist(&mut self) {
        let mut d = rfd::FileDialog::new()
            .set_title("Save playlist")
            .set_file_name("playlist.m3u8")
            .add_filter("M3U8 playlist (UTF-8)", &["m3u8"])
            .add_filter("M3U playlist", &["m3u"])
            .add_filter("PLS playlist", &["pls"])
            .add_filter("XSPF playlist", &["xspf"]);
        if let Some(dir) = &self.prefs.last_dir {
            d = d.set_directory(dir);
        }
        let Some(path) = d.save_file() else { return };
        self.remember_dir(&path);
        if let Err(err) = files::save_playlist(
            &self.player.engine().playlist,
            &path.to_string_lossy(),
            true,
        ) {
            log::error!("cannot save playlist {}: {err}", path.display());
        }
    }
}
