//! Song title marquee (02 §8): displayed string, scrolling and transient messages.
//!
//! Time is passed in as milliseconds so the logic is deterministic in tests.

/// Gap between repetitions of a scrolling title.
pub const SEPARATOR: &str = "  ***  ";
/// Strings wider than this scroll.
pub const SCROLL_THRESHOLD: i32 = 146;
/// Auto-scroll step.
pub const STEP_PX: i32 = 5;
pub const STEP_MS: u64 = 200;
const PAUSE_MS: u64 = 2000;
const MESSAGE_MS: u64 = 1000;

/// Formats `M:SS` with unpadded, unlimited minutes.
pub fn format_length(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// The displayed title string (02 §8.2).
///
/// `position` is 0-based; `length_s` is `None` for unknown lengths.
pub fn title_string(
    position: usize,
    title: &str,
    length_s: Option<u64>,
    show_number: bool,
) -> String {
    match length_s {
        Some(len) if show_number => format!("{}. {} ({})", position + 1, title, format_length(len)),
        Some(len) => format!("{} ({})", title, format_length(len)),
        None => format!("{}. {}", position + 1, title),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageEnd {
    At(u64),
    OnRelease,
}

#[derive(Debug, Clone, Default)]
pub struct Marquee {
    title: String,
    message: Option<(String, MessageEnd)>,
    offset: i32,
    paused_until: u64,
    drag_last_x: Option<i32>,
    /// Width of the scroll period (title + separator), or 0 when the title fits.
    period: i32,
}

impl Marquee {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the title. A different title resets scrolling and pauses it for 2 s.
    pub fn set_title(&mut self, title: &str, now: u64, measure: impl Fn(&str) -> i32) {
        if title == self.title {
            return;
        }
        self.title = title.to_string();
        self.offset = 0;
        self.paused_until = now + PAUSE_MS;
        self.update_period(measure);
    }

    /// Recomputes the scroll period (after a font or skin change).
    pub fn update_period(&mut self, measure: impl Fn(&str) -> i32) {
        let w = measure(&self.title);
        self.period = if w > SCROLL_THRESHOLD {
            w + measure(SEPARATOR)
        } else {
            0
        };
        if self.period == 0 {
            self.offset = 0;
        } else {
            self.offset = self.offset.rem_euclid(self.period);
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// Shows a message for about 1 s.
    pub fn show_message(&mut self, text: impl Into<String>, now: u64) {
        self.message = Some((text.into(), MessageEnd::At(now + MESSAGE_MS)));
    }

    /// Shows a drag message (volume, balance, seek, EQ) until the mouse is released.
    pub fn show_drag_message(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), MessageEnd::OnRelease));
    }

    /// Ends drag messages; they then stay for 1 s.
    pub fn release(&mut self, now: u64) {
        if let Some((_, end @ MessageEnd::OnRelease)) = &mut self.message {
            *end = MessageEnd::At(now + MESSAGE_MS);
        }
    }

    /// Whether a transient message is showing.
    pub fn has_message(&self) -> bool {
        self.message.is_some()
    }

    /// Advances timers. `autoscroll` is false when the option is off, the window is hidden,
    /// minimized or shaded. Returns true when the display changed.
    pub fn tick(&mut self, now: u64, autoscroll: bool) -> bool {
        let mut changed = false;
        if let Some((_, MessageEnd::At(t))) = &self.message
            && now >= *t {
                self.message = None;
                changed = true;
            }
        if autoscroll
            && self.message.is_none()
            && self.drag_last_x.is_none()
            && self.period > 0
            && now >= self.paused_until
        {
            self.offset = (self.offset + STEP_PX) % self.period;
            changed = true;
        }
        changed
    }

    /// Turning auto-scroll off resets the position.
    pub fn autoscroll_disabled(&mut self) {
        self.offset = 0;
    }

    pub fn drag_start(&mut self, x: i32) {
        self.drag_last_x = Some(x);
    }

    pub fn drag_move(&mut self, x: i32) {
        if let Some(last) = self.drag_last_x {
            if self.period > 0 {
                self.offset = (self.offset - (x - last)).rem_euclid(self.period);
            }
            self.drag_last_x = Some(x);
        }
    }

    pub fn drag_end(&mut self, now: u64) {
        if self.drag_last_x.take().is_some() {
            self.paused_until = now + PAUSE_MS;
        }
    }

    /// The text to draw (scroll buffer) and the scroll offset in pixels.
    pub fn visible(&self) -> (String, i32) {
        if let Some((m, _)) = &self.message {
            return (m.clone(), 0);
        }
        if self.period > 0 {
            (
                format!("{}{}{}", self.title, SEPARATOR, self.title),
                self.offset,
            )
        } else {
            (self.title.clone(), 0)
        }
    }
}

/// `Volume: N%`.
pub fn volume_message(volume: u8) -> String {
    format!("Volume: {}%", volume as u32 * 100 / 255)
}

/// `Balance: N% Left|Right` or `Balance: Center`.
pub fn balance_message(balance: i8) -> String {
    let b = balance as i32;
    let n = b.abs() * 100 / 127;
    match b {
        0 => "Balance: Center".into(),
        b if b < 0 => format!("Balance: {n}% Left"),
        _ => format!("Balance: {n}% Right"),
    }
}

/// `Seek to: M:SS/M:SS (P%)`.
pub fn seek_message(target_ms: u64, length_ms: u64) -> String {
    let pct = if length_ms > 0 {
        target_ms * 100 / length_ms
    } else {
        0
    };
    format!(
        "Seek to: {}/{} ({}%)",
        format_length(target_ms / 1000),
        format_length(length_ms / 1000),
        pct
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap_width(s: &str) -> i32 {
        s.chars().count() as i32 * 5
    }

    #[test]
    fn title_strings() {
        assert_eq!(title_string(2, "A - B", Some(225), true), "3. A - B (3:45)");
        assert_eq!(
            title_string(2, "A - B", Some(6000), false),
            "A - B (100:00)"
        );
        assert_eq!(title_string(0, "Radio", None, false), "1. Radio");
    }

    #[test]
    fn scrolls_after_pause() {
        let mut m = Marquee::new();
        let long = "x".repeat(40); // 200 px > 146
        m.set_title(&long, 0, bitmap_width);
        assert!(!m.tick(1000, true));
        assert_eq!(m.visible().1, 0);
        assert!(m.tick(2000, true));
        assert_eq!(m.visible().1, 5);
        let (buf, _) = m.visible();
        assert_eq!(buf, format!("{long}  ***  {long}"));
        // Wraps modulo 200 + 35.
        for i in 0..46 {
            m.tick(2200 + i * 200, true);
        }
        assert_eq!(m.visible().1, (5 + 46 * 5) % 235);
    }

    #[test]
    fn short_title_does_not_scroll() {
        let mut m = Marquee::new();
        m.set_title("short", 0, bitmap_width);
        assert!(!m.tick(5000, true));
        assert_eq!(m.visible(), ("short".into(), 0));
    }

    #[test]
    fn manual_drag_pauses() {
        let mut m = Marquee::new();
        m.set_title(&"y".repeat(40), 0, bitmap_width);
        m.drag_start(150);
        m.drag_move(140);
        assert_eq!(m.visible().1, 10);
        m.drag_move(150);
        assert_eq!(m.visible().1, 0);
        m.drag_move(155);
        assert_eq!(m.visible().1, 230);
        m.drag_end(10_000);
        assert!(!m.tick(11_000, true));
        assert!(m.tick(12_000, true));
    }

    #[test]
    fn messages() {
        let mut m = Marquee::new();
        m.set_title("t", 0, bitmap_width);
        m.show_message("Shuffle: On", 100);
        assert_eq!(m.visible().0, "Shuffle: On");
        m.tick(1099, true);
        assert!(m.has_message());
        m.tick(1100, true);
        assert_eq!(m.visible().0, "t");
        m.show_drag_message(volume_message(200));
        m.tick(99_999, true);
        assert_eq!(m.visible().0, "Volume: 78%");
        m.release(100_000);
        m.tick(101_000, true);
        assert!(!m.has_message());
    }

    #[test]
    fn message_texts() {
        assert_eq!(balance_message(0), "Balance: Center");
        assert_eq!(balance_message(-127), "Balance: 100% Left");
        assert_eq!(balance_message(64), "Balance: 50% Right");
        assert_eq!(seek_message(60_000, 120_000), "Seek to: 1:00/2:00 (50%)");
    }
}
