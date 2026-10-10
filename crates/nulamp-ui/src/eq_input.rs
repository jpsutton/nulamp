//! Equalizer window mouse interaction (03 §1.3, §1.5, §1.6).

use crate::geom::hit;
use crate::input::{Action, MenuKind, Mods};
use crate::marquee::{balance_message, volume_message};
use nulamp_core::Command;
use nulamp_eq::{BandMode, Slider};
use nulamp_render::eq_window::{EqButton, SLIDER_X};

/// What the EQ controller asks for, in addition to the shared actions.
#[derive(Debug, Clone, PartialEq)]
pub enum EqAction {
    Do(Action),
    /// Set slider `index` (0 = preamp, 1..=10 bands) to a value.
    SetSlider(usize, u8),
    /// Set all ten bands (quick-set labels); the preamp is unchanged.
    SetAllBands(u8),
    ToggleEnabled,
    ToggleAutoLoad,
    PresetsMenu {
        x: i32,
        y: i32,
    },
    /// Context menu on the ON or AUTO button.
    ToggleMenu {
        auto: bool,
        x: i32,
        y: i32,
    },
    CloseWindow,
    ToggleShade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Button(EqButton),
    Label(u8),
    Slider {
        index: usize,
        offset: i32,
    },
    /// A band slider was released by moving sideways; the next move captures another one.
    Painting,
    ShadeVolume,
    ShadeBalance,
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EqContext {
    pub shade: bool,
    pub easymove: bool,
    /// Preamp then bands 1–10.
    pub values: [u8; 11],
    pub iso: bool,
}

#[derive(Debug, Clone, Default)]
pub struct EqInput {
    drag: Option<Drag>,
    pub pressed: Option<EqButton>,
    pub dragging: Option<usize>,
    balance: i8,
}

fn slider_at(x: i32, y: i32) -> Option<usize> {
    if !(39..=98).contains(&y) {
        return None;
    }
    SLIDER_X.iter().position(|&sx| x >= sx && x <= sx + 12)
}

fn slider_label(index: usize) -> Slider {
    if index == 0 {
        Slider::Preamp
    } else {
        Slider::Band(index - 1)
    }
}

/// Thumb top offset p used by the drag math.
fn thumb_p(value: u8) -> i32 {
    51 - ((63 - value as i32) * 52 / 64)
}

/// New slider value for a pointer y and grab offset, with the centre detent.
pub fn slider_value(y: i32, offset: i32, mods: Mods) -> u8 {
    let v = (((y - offset - 39) * 64) / 52).clamp(0, 63);
    if !mods.shift && (30..=32).contains(&v) {
        31
    } else {
        v as u8
    }
}

impl EqInput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn slider_msg(&self, index: usize, value: u8, ctx: &EqContext) -> EqAction {
        EqAction::Do(Action::DragMessage(nulamp_eq::value::marquee_text(
            slider_label(index),
            value,
            BandMode::from_iso(ctx.iso),
        )))
    }

    fn grab_slider(&mut self, index: usize, y: i32, mods: Mods, ctx: &EqContext) -> Vec<EqAction> {
        let p = thumb_p(ctx.values[index]);
        let rel = y - 39;
        let offset = if rel >= p - 1 && rel <= p + 10 {
            rel - p
        } else {
            5
        };
        self.drag = Some(Drag::Slider { index, offset });
        self.dragging = Some(index);
        let v = slider_value(y, offset, mods);
        vec![
            EqAction::SetSlider(index, v),
            self.slider_msg(index, v, ctx),
        ]
    }

    pub fn mouse_down(&mut self, x: i32, y: i32, mods: Mods, ctx: &EqContext) -> Vec<EqAction> {
        if self.drag.is_some() {
            return vec![];
        }
        if ctx.shade {
            if hit(x, y, 253, 273, 3, 12) {
                let b = if x < 264 {
                    EqButton::Shade
                } else {
                    EqButton::Close
                };
                self.drag = Some(Drag::Button(b));
                self.pressed = Some(b);
                return vec![];
            }
            if hit(x, y, 61, 162, 3, 11) {
                self.drag = Some(Drag::ShadeVolume);
                return self.shade_volume(x);
            }
            if hit(x, y, 163, 206, 3, 11) {
                self.drag = Some(Drag::ShadeBalance);
                return self.shade_balance(x, mods);
            }
            self.drag = Some(Drag::Window);
            return vec![EqAction::Do(Action::BeginWindowDrag { x, y })];
        }
        if let Some(i) = slider_at(x, y) {
            return self.grab_slider(i, y, mods, ctx);
        }
        let button = match () {
            _ if hit(x, y, 253, 273, 3, 12) => Some(if x < 264 {
                EqButton::Shade
            } else {
                EqButton::Close
            }),
            _ if hit(x, y, 14, 86, 18, 30) => Some(if x >= 39 {
                EqButton::Auto
            } else {
                EqButton::On
            }),
            _ if hit(x, y, 217, 261, 18, 30) => Some(EqButton::Presets),
            _ => None,
        };
        if let Some(b) = button {
            self.drag = Some(Drag::Button(b));
            self.pressed = Some(b);
            return vec![];
        }
        for (label, (y1, y2)) in [(0u8, (33, 42)), (31, (65, 74)), (63, (92, 101))] {
            if hit(x, y, 42, 67, y1, y2) {
                self.drag = Some(Drag::Label(label));
                return vec![];
            }
        }
        if y < 14 || ctx.easymove {
            self.drag = Some(Drag::Window);
            return vec![EqAction::Do(Action::BeginWindowDrag { x, y })];
        }
        vec![]
    }

    fn shade_volume(&mut self, x: i32) -> Vec<EqAction> {
        let t = (x - 61).clamp(0, 96);
        let v = (t * 255 / 96) as u8;
        vec![
            EqAction::Do(Action::Player(Command::SetVolume(v))),
            EqAction::Do(Action::DragMessage(volume_message(v))),
        ]
    }

    fn shade_balance(&mut self, x: i32, mods: Mods) -> Vec<EqAction> {
        let t = (x - 164).clamp(0, 42);
        let mut b = (t * 255 / 42 - 127).min(127);
        if !mods.shift && b.abs() < 9 {
            b = 0;
        }
        self.balance = b as i8;
        vec![
            EqAction::Do(Action::Player(Command::SetBalance(b as i8))),
            EqAction::Do(Action::DragMessage(balance_message(b as i8))),
        ]
    }

    pub fn mouse_move(&mut self, x: i32, y: i32, mods: Mods, ctx: &EqContext) -> Vec<EqAction> {
        match self.drag {
            Some(Drag::Slider { index, offset }) => {
                let sx = SLIDER_X[index];
                // Paint across bands: moving more than 3 px sideways out of a band slider
                // releases it (03 §1.3).
                let outside = x < sx - 3 || x > sx + 12 + 3;
                if index > 0 && outside && hit(x, y, 78, 258, 39, 98) {
                    self.drag = Some(Drag::Painting);
                    self.dragging = None;
                    return vec![];
                }
                let v = slider_value(y, offset, mods);
                vec![
                    EqAction::SetSlider(index, v),
                    self.slider_msg(index, v, ctx),
                ]
            }
            Some(Drag::Painting) => match slider_at(x, y).filter(|&i| i > 0) {
                Some(i) => {
                    self.drag = None;
                    self.grab_slider(i, y, mods, ctx)
                }
                None => vec![],
            },
            Some(Drag::Button(b)) => {
                let inside = match b {
                    EqButton::Shade => hit(x, y, 253, 263, 3, 12),
                    EqButton::Close => hit(x, y, 264, 273, 3, 12),
                    EqButton::On => hit(x, y, 14, 38, 18, 30),
                    EqButton::Auto => hit(x, y, 39, 86, 18, 30),
                    EqButton::Presets => hit(x, y, 217, 261, 18, 30),
                };
                self.pressed = inside.then_some(b);
                vec![]
            }
            Some(Drag::ShadeVolume) => self.shade_volume(x),
            Some(Drag::ShadeBalance) => self.shade_balance(x, mods),
            _ => vec![],
        }
    }

    pub fn mouse_up(&mut self, x: i32, y: i32, _mods: Mods) -> Vec<EqAction> {
        let drag = self.drag.take();
        let pressed = self.pressed.take();
        self.dragging = None;
        match drag {
            Some(Drag::Button(b)) if pressed == Some(b) => vec![match b {
                EqButton::Shade => EqAction::ToggleShade,
                EqButton::Close => EqAction::CloseWindow,
                EqButton::On => EqAction::ToggleEnabled,
                EqButton::Auto => EqAction::ToggleAutoLoad,
                EqButton::Presets => EqAction::PresetsMenu { x: 218, y: 19 },
            }],
            Some(Drag::Label(v)) => {
                let (y1, y2) = match v {
                    0 => (33, 42),
                    31 => (65, 74),
                    _ => (92, 101),
                };
                if hit(x, y, 42, 67, y1, y2) {
                    vec![EqAction::SetAllBands(v)]
                } else {
                    vec![]
                }
            }
            Some(Drag::Slider { .. } | Drag::Painting | Drag::ShadeVolume) => {
                vec![EqAction::Do(Action::ReleaseMessage)]
            }
            Some(Drag::ShadeBalance) => vec![EqAction::Do(Action::ReleaseMessage)],
            _ => vec![],
        }
    }

    pub fn double_click(&self, x: i32, y: i32) -> Option<Vec<EqAction>> {
        (y <= 14 && x < 252).then(|| vec![EqAction::ToggleShade])
    }

    pub fn right_click(&self, x: i32, y: i32, ctx: &EqContext) -> EqAction {
        if !ctx.shade {
            if hit(x, y, 14, 38, 18, 30) {
                return EqAction::ToggleMenu { auto: false, x, y };
            }
            if hit(x, y, 39, 72, 18, 30) {
                return EqAction::ToggleMenu { auto: true, x, y };
            }
        }
        EqAction::Do(Action::Menu {
            kind: MenuKind::Main,
            x,
            y,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> EqContext {
        EqContext {
            shade: false,
            easymove: true,
            values: [31; 11],
            iso: false,
        }
    }

    #[test]
    fn slider_drag_math_every_row() {
        // From the top of the hit box down: values follow ((y − 5 − 39) × 64) / 52.
        for y in 39..=98 {
            let v = slider_value(y, 5, Mods::SHIFT) as i32;
            assert_eq!(v, (((y - 44) * 64) / 52).clamp(0, 63), "y={y}");
        }
        assert_eq!(slider_value(69, 5, Mods::SHIFT), 30);
        assert_eq!(slider_value(69, 5, Mods::NONE), 31);
    }

    #[test]
    fn grab_offsets() {
        let mut e = EqInput::new();
        // Value 31: p = 51 − (32 × 52 / 64) = 25; thumb rows 39+24..39+35.
        let a = e.mouse_down(80, 39 + 27, Mods::NONE, &ctx());
        assert_eq!(a[0], EqAction::SetSlider(1, 31));
        let a = e.mouse_move(80, 39, Mods::NONE, &ctx());
        assert_eq!(a[0], EqAction::SetSlider(1, 0));
        e.mouse_up(80, 39, Mods::NONE);
        // Off the thumb: offset 5 centres it.
        let mut e = EqInput::new();
        let a = e.mouse_down(22, 98, Mods::NONE, &ctx());
        assert_eq!(a[0], EqAction::SetSlider(0, 63));
    }

    #[test]
    fn paint_across() {
        let mut e = EqInput::new();
        e.mouse_down(80, 60, Mods::NONE, &ctx());
        assert!(e.mouse_move(95, 60, Mods::NONE, &ctx()).is_empty());
        let a = e.mouse_move(100, 70, Mods::NONE, &ctx());
        assert!(matches!(a[0], EqAction::SetSlider(2, _)));
    }

    #[test]
    fn buttons_and_labels() {
        let mut e = EqInput::new();
        let c = ctx();
        e.mouse_down(20, 22, Mods::NONE, &c);
        assert_eq!(e.pressed, Some(EqButton::On));
        assert_eq!(
            e.mouse_up(20, 22, Mods::NONE),
            vec![EqAction::ToggleEnabled]
        );
        e.mouse_down(50, 22, Mods::NONE, &c);
        assert_eq!(
            e.mouse_up(50, 22, Mods::NONE),
            vec![EqAction::ToggleAutoLoad]
        );
        e.mouse_down(230, 22, Mods::NONE, &c);
        assert_eq!(
            e.mouse_up(230, 22, Mods::NONE),
            vec![EqAction::PresetsMenu { x: 218, y: 19 }]
        );
        e.mouse_down(50, 70, Mods::NONE, &c);
        assert_eq!(
            e.mouse_up(50, 70, Mods::NONE),
            vec![EqAction::SetAllBands(31)]
        );
        e.mouse_down(268, 6, Mods::NONE, &c);
        assert_eq!(e.mouse_up(268, 6, Mods::NONE), vec![EqAction::CloseWindow]);
        assert_eq!(e.double_click(100, 5), Some(vec![EqAction::ToggleShade]));
    }

    #[test]
    fn shade_sliders() {
        let c = EqContext {
            shade: true,
            ..ctx()
        };
        let mut e = EqInput::new();
        let a = e.mouse_down(157, 6, Mods::NONE, &c);
        assert_eq!(a[0], EqAction::Do(Action::Player(Command::SetVolume(255))));
        e.mouse_up(157, 6, Mods::NONE);
        let a = e.mouse_down(185, 6, Mods::NONE, &c);
        assert_eq!(a[0], EqAction::Do(Action::Player(Command::SetBalance(0))));
    }
}
