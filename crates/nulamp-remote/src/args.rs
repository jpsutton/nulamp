//! Command line (11 §4.2).
//!
//! Switches are case-insensitive and come before file arguments. Each switch has a `/NAME` form
//! and a `--name` form (`--vol=50`, `--skin <path>`). The first argument that is not a known
//! switch starts the file list; `--` ends the switches explicitly. An unknown `--name` is
//! reported in [`Invocation::unknown`] and skipped, so it does not start the file list.

use crate::{DEFAULT_CLASS, Flag, NumberedCommand, Request};

/// Window state requested at start-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartMode {
    #[default]
    Normal,
    /// `/STARTMIN`
    Minimized,
    /// `/STARTMAX`
    Maximized,
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Invocation {
    /// Files, folders and URLs, in order (skin archives excluded).
    pub files: Vec<String>,
    /// Skin archives (`.wsz`) among the file arguments.
    pub skins: Vec<String>,
    /// `/ADD`: append the files, do not clear or play.
    pub enqueue: bool,
    /// `/NEW`: start a new instance.
    pub new_instance: bool,
    /// `/CONSOLE`: keep a console window with the log output (Windows).
    pub console: bool,
    /// The last transport-type switch. Ignored when files are given.
    pub command: Option<Request>,
    /// `/BOOKMARK <file>`.
    pub bookmark: Option<String>,
    pub start: StartMode,
    /// `/CONFIG=<file>`
    pub config: Option<String>,
    /// `/INIDIR=<dir>`
    pub ini_dir: Option<String>,
    /// `/M3UDIR=<dir>`
    pub m3u_dir: Option<String>,
    /// `/CLASS=<name>`
    pub class: Option<String>,
    /// `/SAFE`
    pub safe: bool,
    /// `/DELM3U`
    pub delete_m3u: bool,
    /// `/?`, `--help`, `-h`
    pub help: bool,
    /// `--skin <path>`
    pub skin: Option<String>,
    /// `--import-winamp <dir>`: import an original settings folder (11 §1.4).
    pub import_winamp: Option<String>,
    /// Switches that were not understood (unknown `--name`, bad values).
    pub unknown: Vec<String>,
}

impl Invocation {
    /// The single-instance class.
    pub fn class(&self) -> &str {
        self.class
            .as_deref()
            .filter(|c| !c.is_empty())
            .unwrap_or(DEFAULT_CLASS)
    }

    /// Whether the transport command is used (it is ignored when files are given).
    fn has_file_arguments(&self) -> bool {
        !self.files.is_empty() || !self.skins.is_empty()
    }

    /// The command that is used: `None` when files are given.
    pub fn effective_command(&self) -> Option<&Request> {
        if self.has_file_arguments() {
            None
        } else {
            self.command.as_ref()
        }
    }

    /// Requests to send to a running instance. A launch with nothing to do raises it.
    pub fn requests(&self) -> Vec<Request> {
        let mut out = Vec::new();
        if let Some(b) = &self.bookmark {
            out.push(Request::Bookmark(b.clone()));
        }
        if !self.files.is_empty() {
            out.push(Request::Open {
                files: self.files.clone(),
                enqueue: self.enqueue,
            });
        }
        out.extend(
            self.skins
                .iter()
                .chain(&self.skin)
                .map(|s| Request::LoadSkin(s.clone())),
        );
        if let Some(c) = self.effective_command() {
            out.push(c.clone());
        }
        if out.is_empty() {
            out.push(Request::Raise);
        }
        out
    }
}

/// Command-line help (`/?`).
pub const HELP: &str = "\
Usage: nulamp [switches] [files, folders, URLs]

Switches (case-insensitive, before files; /NAME or --name):
  /ADD                 append the files; do not clear or play
  /NEW                 start a new instance
  /CONSOLE             Windows: open a console window with the log output
  /PLAY /PAUSE /STOP /PREV /NEXT /PLAYPAUSE
  /STOPFADE /STOPAFTER stop with fade-out; toggle stop after current
  /FWD /REV            seek +5 s / -5 s
  /VOLUP /VOLDOWN /VOL=<0..100>
  /PANLEFT /PANRIGHT /PAN=<-127..127>
  /SHUFFLE[=0|1] /RANDOM[=0|1] /REPEAT[=0|1]
  /CLEAR               clear the playlist
  /JUMPTO              open Jump to file
  /CD[0-3]             play audio CD in drive n
  /BOOKMARK <file>     add a bookmark, then exit
  /QUIT /EXIT /CLOSE   close the running instance
  /KILL                close it; force-quit after 3 s
  /STARTMIN /STARTMAX  start minimized / restored
  /CONFIG=<file> /INIDIR=<dir> /M3UDIR=<dir>
  /CLASS=<name>        instance identity
  /SAFE                safe mode (no third-party plugins)
  /DELM3U              delete the saved default playlist, then exit
  /COMMAND=<id>        send a numbered command
  --skin <path>        use a skin
  --import-winamp <dir> import settings, presets, bookmarks and playlist
  /?                   show this help
";

/// What a switch name means.
enum Switch {
    /// Sets a field; takes no value.
    Plain(fn(&mut Invocation)),
    /// A transport-type command without a value.
    Command(Request),
    /// Takes a value: `NAME=value` (or the next argument for `--name` and `/BOOKMARK`).
    Value(fn(&mut Invocation, &str) -> bool),
    /// A command with an optional `=0|1` value.
    Flag(fn(Flag) -> Request),
}

fn lookup(name: &str) -> Option<Switch> {
    use Switch::*;
    Some(match name {
        "add" => Plain(|i| i.enqueue = true),
        "new" => Plain(|i| i.new_instance = true),
        "console" => Plain(|i| i.console = true),
        "startmin" => Plain(|i| i.start = StartMode::Minimized),
        "startmax" => Plain(|i| i.start = StartMode::Maximized),
        "safe" => Plain(|i| i.safe = true),
        "delm3u" => Plain(|i| i.delete_m3u = true),
        "?" | "help" => Plain(|i| i.help = true),
        "play" => Command(Request::Play),
        "pause" => Command(Request::Pause),
        "stop" => Command(Request::Stop),
        "prev" => Command(Request::Previous),
        "next" => Command(Request::Next),
        "playpause" => Command(Request::PlayPause),
        "stopfade" => Command(Request::StopWithFade),
        "stopafter" => Command(Request::StopAfterCurrent),
        "fwd" => Command(Request::SeekRelative(5000)),
        "rev" => Command(Request::SeekRelative(-5000)),
        "volup" => Command(Request::VolumeBy(4)),
        "voldown" => Command(Request::VolumeBy(-4)),
        "panleft" => Command(Request::BalanceBy(-12)),
        "panright" => Command(Request::BalanceBy(12)),
        "clear" => Command(Request::Clear),
        "jumpto" => Command(Request::JumpToFile),
        "cd" | "cd0" => Command(Request::PlayCd(0)),
        "cd1" => Command(Request::PlayCd(1)),
        "cd2" => Command(Request::PlayCd(2)),
        "cd3" => Command(Request::PlayCd(3)),
        "quit" | "exit" | "close" => Command(Request::Quit),
        "kill" => Command(Request::Kill),
        "shuffle" | "random" => Flag(Request::SetShuffle),
        "repeat" => Flag(Request::SetRepeat),
        "vol" => Value(|i, v| {
            let Ok(p) = v.trim().parse::<i64>() else {
                return false;
            };
            let p = p.clamp(0, 100);
            i.command = Some(Request::SetVolume(((p * 255 + 50) / 100) as u8));
            true
        }),
        "pan" => Value(|i, v| {
            let Ok(p) = v.trim().parse::<i64>() else {
                return false;
            };
            i.command = Some(Request::SetBalance(p.clamp(-127, 127) as i8));
            true
        }),
        "command" => Value(|i, v| {
            let Some(c) = v.trim().parse().ok().and_then(NumberedCommand::from_id) else {
                return false;
            };
            i.command = Some(Request::Command(c));
            true
        }),
        "config" => Value(|i, v| set(&mut i.config, v)),
        "inidir" => Value(|i, v| set(&mut i.ini_dir, v)),
        "m3udir" => Value(|i, v| set(&mut i.m3u_dir, v)),
        "class" => Value(|i, v| set(&mut i.class, v)),
        "bookmark" => Value(|i, v| set(&mut i.bookmark, v)),
        "skin" => Value(|i, v| set(&mut i.skin, v)),
        "import-winamp" => Value(|i, v| set(&mut i.import_winamp, v)),
        _ => return None,
    })
}

fn set(field: &mut Option<String>, v: &str) -> bool {
    if v.is_empty() {
        return false;
    }
    *field = Some(v.to_string());
    true
}

/// Whether a file argument is a skin archive.
fn is_skin_archive(arg: &str) -> bool {
    let name = arg.rsplit(['/', '\\']).next().unwrap_or(arg);
    name.len() > 4 && name.to_ascii_lowercase().ends_with(".wsz")
}

/// Parses the arguments after the program name.
pub fn parse_args(args: impl IntoIterator<Item = String>) -> Invocation {
    let mut inv = Invocation::default();
    let mut args = args.into_iter();
    let mut switches = true;
    while let Some(arg) = args.next() {
        if switches {
            if arg == "--" {
                switches = false;
                continue;
            }
            if parse_switch(&mut inv, &arg, &mut args) {
                continue;
            }
            switches = false;
        }
        if is_skin_archive(&arg) {
            inv.skins.push(arg);
        } else {
            inv.files.push(arg);
        }
    }
    inv
}

/// Applies one switch. Returns false when `arg` is not a switch and starts the file list.
fn parse_switch(inv: &mut Invocation, arg: &str, rest: &mut impl Iterator<Item = String>) -> bool {
    let (body, long) = if let Some(b) = arg.strip_prefix("--") {
        (b, true)
    } else if let Some(b) = arg.strip_prefix('/') {
        (b, false)
    } else if arg == "-h" {
        ("help", true)
    } else {
        return false;
    };
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    };
    let Some(switch) = lookup(&name.to_ascii_lowercase()) else {
        if long && !body.is_empty() {
            inv.unknown.push(arg.to_string());
            return true;
        }
        return false;
    };
    let ok = match (switch, value) {
        (Switch::Plain(f), None) => {
            f(inv);
            true
        }
        (Switch::Command(r), None) => {
            inv.command = Some(r);
            true
        }
        (Switch::Flag(f), None) => {
            inv.command = Some(f(Flag::Toggle));
            true
        }
        (Switch::Flag(f), Some(v)) => match v.trim() {
            "0" => {
                inv.command = Some(f(Flag::Off));
                true
            }
            "1" => {
                inv.command = Some(f(Flag::On));
                true
            }
            _ => false,
        },
        (Switch::Value(f), Some(v)) => f(inv, v),
        (Switch::Value(f), None) => {
            // `/BOOKMARK <file>` and `--name <value>` take the next argument.
            let takes_next = long || name.eq_ignore_ascii_case("bookmark");
            match if takes_next { rest.next() } else { None } {
                Some(v) => f(inv, &v),
                None => false,
            }
        }
        (Switch::Plain(_) | Switch::Command(_), Some(_)) => false,
    };
    if !ok {
        inv.unknown.push(arg.to_string());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Invocation {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    fn command(args: &[&str]) -> Option<Request> {
        parse(args).command
    }

    #[test]
    fn empty() {
        let i = parse(&[]);
        assert_eq!(i, Invocation::default());
        assert_eq!(i.requests(), vec![Request::Raise]);
        assert_eq!(i.class(), DEFAULT_CLASS);
    }

    #[test]
    fn files_and_add() {
        let i = parse(&["/ADD", "a.mp3", "b c.flac"]);
        assert!(i.enqueue);
        assert_eq!(i.files, vec!["a.mp3", "b c.flac"]);
        assert_eq!(
            i.requests(),
            vec![Request::Open {
                files: vec!["a.mp3".into(), "b c.flac".into()],
                enqueue: true
            }]
        );
        let i = parse(&["a.mp3"]);
        assert!(!i.enqueue);
        assert_eq!(
            i.requests(),
            vec![Request::Open {
                files: vec!["a.mp3".into()],
                enqueue: false
            }]
        );
    }

    #[test]
    fn plain_switches() {
        assert!(parse(&["/NEW"]).new_instance);
        assert!(parse(&["/new"]).new_instance);
        assert_eq!(parse(&["/STARTMIN"]).start, StartMode::Minimized);
        assert_eq!(parse(&["/StartMax"]).start, StartMode::Maximized);
        assert!(parse(&["/SAFE"]).safe);
        assert_eq!(
            parse(&["--import-winamp", "/w"]).import_winamp.as_deref(),
            Some("/w")
        );
        assert!(parse(&["/DELM3U"]).delete_m3u);
        assert!(parse(&["/?"]).help);
        assert!(parse(&["--help"]).help);
        assert!(parse(&["-h"]).help);
    }

    #[test]
    fn transport_switches() {
        let cases: &[(&str, Request)] = &[
            ("/PLAY", Request::Play),
            ("/PAUSE", Request::Pause),
            ("/STOP", Request::Stop),
            ("/PREV", Request::Previous),
            ("/NEXT", Request::Next),
            ("/PLAYPAUSE", Request::PlayPause),
            ("/STOPFADE", Request::StopWithFade),
            ("/STOPAFTER", Request::StopAfterCurrent),
            ("/FWD", Request::SeekRelative(5000)),
            ("/REV", Request::SeekRelative(-5000)),
            ("/VOLUP", Request::VolumeBy(4)),
            ("/VOLDOWN", Request::VolumeBy(-4)),
            ("/PANLEFT", Request::BalanceBy(-12)),
            ("/PANRIGHT", Request::BalanceBy(12)),
            ("/CLEAR", Request::Clear),
            ("/JUMPTO", Request::JumpToFile),
            ("/CD", Request::PlayCd(0)),
            ("/CD0", Request::PlayCd(0)),
            ("/CD1", Request::PlayCd(1)),
            ("/CD2", Request::PlayCd(2)),
            ("/CD3", Request::PlayCd(3)),
            ("/QUIT", Request::Quit),
            ("/EXIT", Request::Quit),
            ("/CLOSE", Request::Quit),
            ("/KILL", Request::Kill),
        ];
        for (arg, req) in cases {
            assert_eq!(command(&[arg]).as_ref(), Some(req), "{arg}");
            assert_eq!(command(&[&arg.to_lowercase()]).as_ref(), Some(req), "{arg}");
            let long = format!("--{}", arg[1..].to_lowercase());
            assert_eq!(command(&[&long]).as_ref(), Some(req), "{long}");
            assert_eq!(parse(&[arg]).requests(), vec![req.clone()]);
        }
        assert_eq!(command(&["/CD4"]), None);
    }

    #[test]
    fn volume_and_pan_values() {
        assert_eq!(command(&["/VOL=0"]), Some(Request::SetVolume(0)));
        assert_eq!(command(&["/VOL=50"]), Some(Request::SetVolume(128)));
        assert_eq!(command(&["/VOL=100"]), Some(Request::SetVolume(255)));
        assert_eq!(command(&["/vol=250"]), Some(Request::SetVolume(255)));
        assert_eq!(command(&["/VOL=-3"]), Some(Request::SetVolume(0)));
        assert_eq!(command(&["--vol=10"]), Some(Request::SetVolume(26)));
        assert_eq!(command(&["--vol", "10"]), Some(Request::SetVolume(26)));
        assert_eq!(command(&["/PAN=-127"]), Some(Request::SetBalance(-127)));
        assert_eq!(command(&["/PAN=40"]), Some(Request::SetBalance(40)));
        assert_eq!(command(&["/pan=-500"]), Some(Request::SetBalance(-127)));
        let i = parse(&["/VOL=abc", "x.mp3"]);
        assert_eq!(i.command, None);
        assert_eq!(i.unknown, vec!["/VOL=abc"]);
        assert_eq!(i.files, vec!["x.mp3"]);
        assert_eq!(parse(&["/VOL"]).unknown, vec!["/VOL"]);
    }

    #[test]
    fn shuffle_random_repeat() {
        for name in ["SHUFFLE", "RANDOM"] {
            assert_eq!(
                command(&[&format!("/{name}")]),
                Some(Request::SetShuffle(Flag::Toggle))
            );
            assert_eq!(
                command(&[&format!("/{name}=0")]),
                Some(Request::SetShuffle(Flag::Off))
            );
            assert_eq!(
                command(&[&format!("/{name}=1")]),
                Some(Request::SetShuffle(Flag::On))
            );
        }
        assert_eq!(
            command(&["/REPEAT"]),
            Some(Request::SetRepeat(Flag::Toggle))
        );
        assert_eq!(command(&["/repeat=0"]), Some(Request::SetRepeat(Flag::Off)));
        assert_eq!(command(&["--repeat=1"]), Some(Request::SetRepeat(Flag::On)));
        assert_eq!(parse(&["/REPEAT=2"]).unknown, vec!["/REPEAT=2"]);
    }

    #[test]
    fn numbered_command() {
        assert_eq!(
            command(&["/COMMAND=40045"]),
            Some(Request::Command(NumberedCommand::Play))
        );
        assert_eq!(
            command(&["--command=40194"]),
            Some(Request::Command(NumberedCommand::JumpToFile))
        );
        let i = parse(&["/COMMAND=12"]);
        assert_eq!(i.command, None);
        assert_eq!(i.unknown, vec!["/COMMAND=12"]);
    }

    #[test]
    fn paths_and_identity() {
        let i = parse(&[
            "/CONFIG=C:\\n.ini",
            "/INIDIR=/home/u/cfg",
            "/M3UDIR=/home/u/pl",
            "/CLASS=work",
        ]);
        assert_eq!(i.config.as_deref(), Some("C:\\n.ini"));
        assert_eq!(i.ini_dir.as_deref(), Some("/home/u/cfg"));
        assert_eq!(i.m3u_dir.as_deref(), Some("/home/u/pl"));
        assert_eq!(i.class(), "work");
        assert_eq!(parse(&["--class", "x"]).class(), "x");
        assert_eq!(parse(&["/CLASS="]).class(), DEFAULT_CLASS);
    }

    #[test]
    fn bookmark_takes_next_argument() {
        let i = parse(&["/BOOKMARK", "/music/a.mp3", "b.mp3"]);
        assert_eq!(i.bookmark.as_deref(), Some("/music/a.mp3"));
        assert_eq!(i.files, vec!["b.mp3"]);
        assert_eq!(i.requests()[0], Request::Bookmark("/music/a.mp3".into()));
        let i = parse(&["/BOOKMARK"]);
        assert_eq!(i.bookmark, None);
        assert_eq!(i.requests(), vec![Request::Raise]);
        assert_eq!(parse(&["/BOOKMARK"]).unknown, vec!["/BOOKMARK"]);
        let i = parse(&["/bookmark", "x.mp3"]);
        assert_eq!(i.requests(), vec![Request::Bookmark("x.mp3".into())]);
    }

    #[test]
    fn skin_option() {
        assert_eq!(
            parse(&["--skin", "/s/a.wsz"]).skin.as_deref(),
            Some("/s/a.wsz")
        );
        assert_eq!(parse(&["--SKIN=/s/b"]).skin.as_deref(), Some("/s/b"));
        assert_eq!(parse(&["/skin=/s/b"]).skin.as_deref(), Some("/s/b"));
        let i = parse(&["--skin", "/s/b", "/PLAY"]);
        assert_eq!(
            i.requests(),
            vec![Request::LoadSkin("/s/b".into()), Request::Play]
        );
    }

    #[test]
    fn last_transport_wins() {
        assert_eq!(command(&["/PLAY", "/STOP", "/NEXT"]), Some(Request::Next));
        assert_eq!(command(&["/VOL=10", "/PAUSE"]), Some(Request::Pause));
    }

    #[test]
    fn files_override_transport() {
        let i = parse(&["/STOP", "a.mp3"]);
        assert_eq!(i.command, Some(Request::Stop));
        assert_eq!(i.effective_command(), None);
        assert_eq!(
            i.requests(),
            vec![Request::Open {
                files: vec!["a.mp3".into()],
                enqueue: false
            }]
        );
    }

    #[test]
    fn switches_only_before_files() {
        let i = parse(&["a.mp3", "/PLAY", "--add"]);
        assert_eq!(i.files, vec!["a.mp3", "/PLAY", "--add"]);
        assert_eq!(i.command, None);
        assert!(!i.enqueue);
        let i = parse(&["--", "/PLAY"]);
        assert_eq!(i.files, vec!["/PLAY"]);
    }

    #[test]
    fn unix_paths_are_files() {
        let i = parse(&["/PLAY", "/home/u/a.mp3", "/NEXT"]);
        assert_eq!(i.command, Some(Request::Play));
        assert_eq!(i.files, vec!["/home/u/a.mp3", "/NEXT"]);
        let i = parse(&["/music"]);
        assert_eq!(i.files, vec!["/music"]);
        assert!(i.unknown.is_empty());
    }

    #[test]
    fn unknown_long_switch_is_skipped() {
        let i = parse(&["--frobnicate", "--play", "a.mp3"]);
        assert_eq!(i.unknown, vec!["--frobnicate"]);
        assert_eq!(i.command, Some(Request::Play));
        assert_eq!(i.files, vec!["a.mp3"]);
        assert_eq!(parse(&["/PLAY=1"]).unknown, vec!["/PLAY=1"]);
    }

    #[test]
    fn skins_reported_separately() {
        let i = parse(&["a.mp3", "/s/Cool.WSZ", "b.ogg", "C:\\s\\x.wsz", ".wsz"]);
        assert_eq!(i.files, vec!["a.mp3", "b.ogg", ".wsz"]);
        assert_eq!(i.skins, vec!["/s/Cool.WSZ", "C:\\s\\x.wsz"]);
        let i = parse(&["/PLAY", "x.wsz"]);
        assert_eq!(i.requests(), vec![Request::LoadSkin("x.wsz".into())]);
    }

    #[test]
    fn help_text_lists_switches() {
        for s in ["/ADD", "/KILL", "/COMMAND=", "/CLASS=", "--skin"] {
            assert!(HELP.contains(s), "{s}");
        }
    }
}
