//! The numeric command table of `NulampHost.send_command` (11 §6.3 plus nulamp commands).

use nulamp_core::command::Command;

use crate::sys::*;

/// A command a plugin asked for, decoded from its number.
#[derive(Debug, Clone, PartialEq)]
pub enum HostCommand {
    /// A player command.
    Player(Command),
    /// A numbered command of 11 §6.3 that is not a [`Command`] (toggles, dialogs, windows).
    /// The application handles it like the matching menu item.
    Numbered(u32),
}

/// Numbered commands of 11 §6.3 that have no [`Command`] equivalent.
const NUMBERED: &[u32] = &[
    40001, 40012, 40019, 40022, 40023, 40029, 40036, 40037, 40038, 40040, 40041, 40043, 40063,
    40064, 40065, 40145, 40155, 40165, 40185, 40186, 40187, 40188, 40192, 40193, 40194, 40219,
    40242, 40258, 40266, 40291, 40321, 40328, 40395,
];

/// Decodes a command number and its argument. `None` for unknown numbers and invalid
/// arguments (a negative playlist position). Out-of-range values are clamped.
pub fn command_from_id(id: u32, arg: i64) -> Option<HostCommand> {
    let clamp_i32 = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let cmd = match id {
        NULAMP_CMD_PREVIOUS => Command::Previous,
        NULAMP_CMD_PLAY => Command::Play,
        NULAMP_CMD_PAUSE => Command::Pause,
        NULAMP_CMD_STOP => Command::Stop,
        NULAMP_CMD_NEXT => Command::Next,
        NULAMP_CMD_VOLUME_UP => Command::VolumeBy(4),
        NULAMP_CMD_VOLUME_DOWN => Command::VolumeBy(-4),
        NULAMP_CMD_FORWARD_5S | NULAMP_CMD_FORWARD_5S_ALT => Command::SeekRelative(5000),
        NULAMP_CMD_REWIND_5S | NULAMP_CMD_REWIND_5S_ALT => Command::SeekRelative(-5000),
        NULAMP_CMD_STOP_WITH_FADE => Command::StopWithFade,
        NULAMP_CMD_START_OF_LIST => Command::StartOfList,
        NULAMP_CMD_STOP_AFTER_CURRENT => Command::ToggleStopAfterCurrent,
        NULAMP_CMD_END_OF_LIST => Command::EndOfList,
        NULAMP_CMD_FORWARD_10_TRACKS => Command::JumpEntries(10),
        NULAMP_CMD_BACK_10_TRACKS => Command::JumpEntries(-10),
        NULAMP_CMD_SEEK_TO => Command::SeekTo(arg.max(0) as u64),
        NULAMP_CMD_SEEK_RELATIVE => Command::SeekRelative(arg),
        NULAMP_CMD_SET_VOLUME => Command::SetVolume(arg.clamp(0, 255) as u8),
        NULAMP_CMD_VOLUME_BY => Command::VolumeBy(clamp_i32(arg)),
        NULAMP_CMD_SET_BALANCE => Command::SetBalance(arg.clamp(-127, 127) as i8),
        NULAMP_CMD_SET_SHUFFLE => Command::SetShuffle(arg != 0),
        NULAMP_CMD_SET_REPEAT => Command::SetRepeat(arg != 0),
        NULAMP_CMD_SET_MANUAL_ADVANCE => Command::SetManualAdvance(arg != 0),
        NULAMP_CMD_PLAY_INDEX => Command::PlayIndex(usize::try_from(arg).ok()?),
        NULAMP_CMD_JUMP_ENTRIES => Command::JumpEntries(clamp_i32(arg)),
        id if NUMBERED.contains(&id) => return Some(HostCommand::Numbered(id)),
        _ => return None,
    };
    Some(HostCommand::Player(cmd))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: u32, arg: i64) -> Command {
        match command_from_id(id, arg) {
            Some(HostCommand::Player(c)) => c,
            other => panic!("{id}: {other:?}"),
        }
    }

    #[test]
    fn spec_numbers() {
        assert_eq!(player(40044, 0), Command::Previous);
        assert_eq!(player(40045, 7), Command::Play);
        assert_eq!(player(40046, 0), Command::Pause);
        assert_eq!(player(40047, 0), Command::Stop);
        assert_eq!(player(40048, 0), Command::Next);
        assert_eq!(player(40147, 0), Command::StopWithFade);
        assert_eq!(player(40148, 0), Command::SeekRelative(5000));
        assert_eq!(player(40061, 0), Command::SeekRelative(-5000));
        assert_eq!(player(40197, 0), Command::JumpEntries(-10));
        assert_eq!(
            command_from_id(40023, 0),
            Some(HostCommand::Numbered(40023))
        );
        assert_eq!(
            command_from_id(40012, 0),
            Some(HostCommand::Numbered(40012))
        );
        assert_eq!(command_from_id(40000, 0), None);
        assert_eq!(command_from_id(0, 0), None);
    }

    #[test]
    fn arguments_are_clamped() {
        assert_eq!(player(NULAMP_CMD_SET_VOLUME, 300), Command::SetVolume(255));
        assert_eq!(player(NULAMP_CMD_SET_VOLUME, -3), Command::SetVolume(0));
        assert_eq!(
            player(NULAMP_CMD_SET_BALANCE, -500),
            Command::SetBalance(-127)
        );
        assert_eq!(player(NULAMP_CMD_SEEK_TO, -1), Command::SeekTo(0));
        assert_eq!(
            player(NULAMP_CMD_SEEK_RELATIVE, -2500),
            Command::SeekRelative(-2500)
        );
        assert_eq!(player(NULAMP_CMD_SET_SHUFFLE, 2), Command::SetShuffle(true));
        assert_eq!(player(NULAMP_CMD_PLAY_INDEX, 4), Command::PlayIndex(4));
        assert_eq!(command_from_id(NULAMP_CMD_PLAY_INDEX, -1), None);
        assert_eq!(
            player(NULAMP_CMD_JUMP_ENTRIES, i64::MIN),
            Command::JumpEntries(i32::MIN)
        );
    }
}
