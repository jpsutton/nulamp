//! Numbered commands (11 §6.3), used by `/COMMAND=<id>` and [`crate::Request::Command`].

use serde::{Deserialize, Serialize};

macro_rules! numbered {
    ($($(#[$doc:meta])* $name:ident = $id:literal,)*) => {
        /// A command identified by its classic number. Serialised as the number.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(into = "u32", try_from = "u32")]
        pub enum NumberedCommand {
            $($(#[$doc])* $name,)*
        }

        impl NumberedCommand {
            /// Every command, in table order.
            pub const ALL: &[NumberedCommand] = &[$(Self::$name,)*];

            /// The command's number.
            pub fn id(self) -> u32 {
                match self {
                    $(Self::$name => $id,)*
                }
            }

            /// The command with a number, if it is in the table.
            pub fn from_id(id: u32) -> Option<Self> {
                match id {
                    $($id => Some(Self::$name),)*
                    _ => None,
                }
            }
        }
    };
}

numbered! {
    Exit = 40001,
    Preferences = 40012,
    AlwaysOnTop = 40019,
    /// Toggle repeat.
    Repeat = 40022,
    /// Toggle shuffle.
    Shuffle = 40023,
    OpenFiles = 40029,
    ToggleEqualizer = 40036,
    TimeElapsed = 40037,
    TimeRemaining = 40038,
    TogglePlaylist = 40040,
    About = 40041,
    MainMenu = 40043,
    Previous = 40044,
    Play = 40045,
    Pause = 40046,
    Stop = 40047,
    Next = 40048,
    VolumeUp = 40058,
    VolumeDown = 40059,
    Forward5s = 40060,
    Rewind5s = 40061,
    NextWindow = 40063,
    MainWindowshade = 40064,
    WindowshadeFocused = 40065,
    /// Second number for "rewind 5 s".
    Rewind5sAlt = 40144,
    OpenFileDialog = 40145,
    StopWithFade = 40147,
    FastForward5s = 40148,
    StartOfList = 40154,
    OpenUrlDialog = 40155,
    /// Toggle stop after current.
    StopAfterCurrent = 40157,
    EndOfList = 40158,
    DoubleSize = 40165,
    OpenUrl = 40185,
    EasyMove = 40186,
    OpenFolder = 40187,
    FileInfo = 40188,
    VisualizationStartStop = 40192,
    JumpToTime = 40193,
    JumpToFile = 40194,
    TenTracksForward = 40195,
    TenTracksBack = 40197,
    SkinBrowser = 40219,
    ToggleTimeDisplay = 40242,
    ToggleMainWindow = 40258,
    PlaylistWindowshade = 40266,
    ReloadSkin = 40291,
    BookmarkCurrent = 40321,
    ToggleVideo = 40328,
    /// Toggle manual advance.
    ManualAdvance = 40395,
}

impl From<NumberedCommand> for u32 {
    fn from(c: NumberedCommand) -> u32 {
        c.id()
    }
}

impl TryFrom<u32> for NumberedCommand {
    type Error = String;

    fn try_from(id: u32) -> Result<Self, String> {
        Self::from_id(id).ok_or_else(|| format!("unknown command number {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_are_unique() {
        assert_eq!(NumberedCommand::ALL.len(), 50);
        let mut seen = std::collections::HashSet::new();
        for &c in NumberedCommand::ALL {
            assert!(seen.insert(c.id()), "duplicate id {}", c.id());
            assert_eq!(NumberedCommand::from_id(c.id()), Some(c));
        }
        assert_eq!(NumberedCommand::from_id(40000), None);
    }

    #[test]
    fn table_samples() {
        assert_eq!(NumberedCommand::Play.id(), 40045);
        assert_eq!(NumberedCommand::Rewind5sAlt.id(), 40144);
        assert_eq!(NumberedCommand::ManualAdvance.id(), 40395);
        assert_eq!(
            NumberedCommand::from_id(40197),
            Some(NumberedCommand::TenTracksBack)
        );
    }

    #[test]
    fn serialises_as_number() {
        let s = serde_json::to_string(&NumberedCommand::Stop).unwrap();
        assert_eq!(s, "40047");
        let c: NumberedCommand = serde_json::from_str("40194").unwrap();
        assert_eq!(c, NumberedCommand::JumpToFile);
        assert!(serde_json::from_str::<NumberedCommand>("1").is_err());
    }
}
