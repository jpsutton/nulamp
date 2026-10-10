//! Options that cannot work when nulamp runs as a native Wayland window, and the tooltips that
//! explain why they are greyed out.

pub const ALWAYS_ON_TOP: &str = "Not available as a native Wayland window: Wayland does not let apps keep their windows above others.";
pub const KEEP_ON_SCREEN: &str = "Not available as a native Wayland window: Wayland does not tell apps where their windows are on screen.";
pub const GLOBAL_HOTKEYS: &str =
    "Not available as a native Wayland window: Wayland does not allow global key grabs.";
