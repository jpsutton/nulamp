//! Scripted effects (08 §5, optional): user scripts in a small expression language ([`lang`])
//! run per sample, with the variables `left`, `right`, `srate`, `nch`, `slider1`…`slider4`
//! and three sections: `@init` (on load and format change), `@slider` (after init and when a
//! slider moves) and `@sample` (per frame).
//!
//! A preset file (`.nsp`, UTF-8):
//!
//! ```text
//! #name Echo
//! #slider1 0.3 Delay (s)
//! #slider2 0.5 Feedback
//! @init
//! pos = 0;
//! @slider
//! len = floor(slider1 * srate);
//! @sample
//! l = buf[pos * 2]; r = buf[pos * 2 + 1];
//! buf[pos * 2] = left + l * slider2; buf[pos * 2 + 1] = right + r * slider2;
//! left += l; right += r;
//! pos = (pos + 1) % max(len, 1);
//! ```

pub mod lang;

use std::path::Path;

use lang::{Machine, ParseError, Program, Symbols};

/// A slider of a preset.
#[derive(Debug, Clone, PartialEq)]
pub struct Slider {
    pub value: f64,
    pub label: String,
}

/// A parsed and compiled preset.
#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    /// The preset file; `None` for a built-in preset.
    pub path: Option<std::path::PathBuf>,
    /// The built-in preset's file name.
    pub builtin_file: Option<&'static str>,
    pub sliders: [Option<Slider>; 4],
    init: Program,
    slider: Program,
    sample: Program,
    syms: Symbols,
    slots: Slots,
}

#[derive(Debug, Clone, Copy, Default)]
struct Slots {
    left: usize,
    right: usize,
    srate: usize,
    nch: usize,
    sliders: [usize; 4],
}

impl Script {
    /// Parses a preset. `fallback_name` is used when there is no `#name` line.
    pub fn parse(text: &str, fallback_name: &str) -> Result<Self, ParseError> {
        let mut syms = Symbols::default();
        let slots = Slots {
            left: syms.slot("left"),
            right: syms.slot("right"),
            srate: syms.slot("srate"),
            nch: syms.slot("nch"),
            sliders: [
                syms.slot("slider1"),
                syms.slot("slider2"),
                syms.slot("slider3"),
                syms.slot("slider4"),
            ],
        };
        let mut name = fallback_name.to_string();
        let mut sliders: [Option<Slider>; 4] = Default::default();
        let mut sections: [(String, usize); 3] = Default::default();
        let mut current: Option<usize> = None;
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix('#') {
                let (key, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let key = key.to_ascii_lowercase();
                if key == "name" {
                    name = value.trim().to_string();
                } else if let Some(n) = key
                    .strip_prefix("slider")
                    .and_then(|n| n.parse::<usize>().ok())
                    .filter(|n| (1..=4).contains(n))
                {
                    let (v, label) = value
                        .trim()
                        .split_once(char::is_whitespace)
                        .unwrap_or((value.trim(), ""));
                    sliders[n - 1] = Some(Slider {
                        value: v.parse().unwrap_or(0.0),
                        label: label.trim().to_string(),
                    });
                }
                continue;
            }
            if let Some(sec) = trimmed.strip_prefix('@') {
                current = match sec.trim().to_ascii_lowercase().as_str() {
                    "init" => Some(0),
                    "slider" => Some(1),
                    "sample" => Some(2),
                    other => {
                        return Err(ParseError {
                            line: i + 1,
                            message: format!("unknown section @{other}"),
                        });
                    }
                };
                if let Some(c) = current {
                    sections[c].1 = i + 2;
                }
                continue;
            }
            if let Some(c) = current {
                sections[c].0.push_str(line);
            }
            if let Some(c) = current {
                sections[c].0.push('\n');
            } else if !trimmed.is_empty() && !trimmed.starts_with("//") {
                return Err(ParseError {
                    line: i + 1,
                    message: "code outside a section (@init, @slider, @sample)".into(),
                });
            }
        }
        let init = lang::compile(&sections[0].0, &mut syms, sections[0].1)?;
        let slider = lang::compile(&sections[1].0, &mut syms, sections[1].1)?;
        let sample = lang::compile(&sections[2].0, &mut syms, sections[2].1)?;
        Ok(Self {
            name,
            path: None,
            builtin_file: None,
            sliders,
            init,
            slider,
            sample,
            syms,
            slots,
        })
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut s = Self::parse(&text, &stem).map_err(|e| format!("{}: {e}", path.display()))?;
        s.path = Some(path.to_path_buf());
        Ok(s)
    }

    /// A processing stage running this script.
    pub fn stage(&self) -> ScriptStage {
        ScriptStage {
            machine: Machine::new(self.syms.len()),
            script: self.clone(),
            format: None,
        }
    }
}

/// Built-in presets: (file name, text).
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "vocal-removal.nsp",
        include_str!("../presets/vocal-removal.nsp"),
    ),
    ("mono.nsp", include_str!("../presets/mono.nsp")),
    (
        "swap-channels.nsp",
        include_str!("../presets/swap-channels.nsp"),
    ),
    (
        "stereo-width.nsp",
        include_str!("../presets/stereo-width.nsp"),
    ),
    ("echo.nsp", include_str!("../presets/echo.nsp")),
    ("gain.nsp", include_str!("../presets/gain.nsp")),
];

/// The built-in presets, then every `.nsp` file in `dir` (sorted by name). A file in `dir`
/// with a built-in preset's file name replaces it. Presets that fail to compile are returned as
/// errors.
pub fn presets(dir: Option<&Path>) -> (Vec<Script>, Vec<String>) {
    let mut ok = Vec::new();
    let mut errors = Vec::new();
    for (file, text) in BUILTIN {
        if dir.is_some_and(|d| d.join(file).is_file()) {
            continue;
        }
        match Script::parse(text, file) {
            Ok(mut s) => {
                s.builtin_file = Some(file);
                ok.push(s)
            }
            Err(e) => errors.push(format!("{file}: {e}")),
        }
    }
    if let Some(dir) = dir
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        let mut files: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nsp")))
            .collect();
        files.sort();
        for f in files {
            match Script::load(&f) {
                Ok(s) => ok.push(s),
                Err(e) => errors.push(e),
            }
        }
    }
    (ok, errors)
}

/// A script as an audio stage (after the EQ). Channels beyond the second pass through; mono
/// input runs with `right = left` and keeps `left`.
pub struct ScriptStage {
    script: Script,
    machine: Machine,
    format: Option<(u32, usize)>,
}

impl ScriptStage {
    fn start(&mut self) {
        let (rate, ch) = self.format.unwrap_or((44_100, 2));
        let s = &self.script.slots;
        self.machine.clear();
        self.machine.vars[s.srate] = rate as f64;
        self.machine.vars[s.nch] = ch as f64;
        for (i, slider) in self.script.sliders.iter().enumerate() {
            self.machine.vars[s.sliders[i]] = slider.as_ref().map_or(0.0, |s| s.value);
        }
        self.machine.run(&self.script.init);
        self.machine.run(&self.script.slider);
    }

    /// Moves slider `n` (0–3) and runs the `@slider` section.
    pub fn set_slider(&mut self, n: usize, value: f64) {
        if let Some(&slot) = self.script.slots.sliders.get(n) {
            self.machine.vars[slot] = value;
            self.machine.run(&self.script.slider);
        }
    }
}

impl nulamp_audio::chain::Stage for ScriptStage {
    fn configure(&mut self, sample_rate: u32, channels: usize) {
        if self.format != Some((sample_rate, channels)) {
            self.format = Some((sample_rate, channels));
            self.start();
        }
    }

    fn process(&mut self, samples: &mut [f32]) {
        let ch = self.format.map_or(2, |f| f.1).max(1);
        let s = self.script.slots;
        for frame in samples.chunks_exact_mut(ch) {
            let l = frame[0] as f64;
            let r = if ch > 1 { frame[1] as f64 } else { l };
            self.machine.vars[s.left] = l;
            self.machine.vars[s.right] = r;
            self.machine.run(&self.script.sample);
            let out = |v: f64| if v.is_finite() { v as f32 } else { 0.0 };
            frame[0] = out(self.machine.vars[s.left]);
            if ch > 1 {
                frame[1] = out(self.machine.vars[s.right]);
            }
        }
    }

    fn reset(&mut self) {
        self.start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nulamp_audio::chain::Stage;

    fn builtin(name: &str) -> Script {
        presets(None)
            .0
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no preset {name}"))
    }

    #[test]
    fn builtins_compile() {
        let (ok, errors) = presets(None);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(ok.len(), BUILTIN.len());
    }

    #[test]
    fn vocal_removal() {
        let mut st = builtin("Vocal removal").stage();
        st.configure(44_100, 2);
        let mut s = vec![0.5, 0.5, 0.75, 0.25];
        st.process(&mut s);
        assert_eq!(s, vec![0.0, 0.0, 0.5, 0.5]);
    }

    #[test]
    fn mono_input_keeps_left() {
        let mut st = builtin("Swap channels").stage();
        st.configure(44_100, 1);
        let mut s = vec![0.25, -0.5];
        st.process(&mut s);
        assert_eq!(s, vec![0.25, -0.5]);
    }

    #[test]
    fn gain_slider() {
        let mut st = builtin("Gain").stage();
        st.configure(48_000, 2);
        st.set_slider(0, -6.0206);
        let mut s = vec![1.0, -1.0];
        st.process(&mut s);
        assert!(
            (s[0] - 0.5).abs() < 1e-4 && (s[1] + 0.5).abs() < 1e-4,
            "{s:?}"
        );
    }

    #[test]
    fn echo_repeats_after_the_delay() {
        let mut st = builtin("Echo").stage();
        st.configure(1000, 2);
        st.set_slider(0, 0.01);
        st.set_slider(1, 0.5);
        let mut s = vec![0.0; 2 * 30];
        s[0] = 1.0;
        st.process(&mut s);
        assert_eq!(s[0], 1.0);
        assert_eq!(s[2 * 10], 1.0);
        assert_eq!(s[2 * 20], 0.5);
        assert_eq!(s[2 * 5], 0.0);
    }

    #[test]
    fn user_file_replaces_builtin() {
        let dir = std::env::temp_dir().join(format!("nulamp-nsp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mono.nsp"), "#name My mono\n@sample\n").unwrap();
        std::fs::write(dir.join("bad.nsp"), "@sample\nleft = (\n").unwrap();
        let (ok, errors) = presets(Some(&dir));
        assert_eq!(errors.len(), 1);
        assert!(ok.iter().all(|s| s.name != "Mono"));
        let mine = ok.iter().find(|s| s.name == "My mono").unwrap();
        assert_eq!(mine.path.as_deref(), Some(dir.join("mono.nsp").as_path()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preset_errors() {
        assert!(Script::parse("x = 1", "a").is_err());
        assert!(Script::parse("@nope\n", "a").is_err());
        let e = Script::parse("#name A\n@sample\nleft = (1;\n", "a").unwrap_err();
        assert_eq!(e.line, 3);
        let s = Script::parse("#name A\n#slider2 0.25 Mix level\n@sample\n", "a").unwrap();
        assert_eq!(
            s.sliders[1],
            Some(Slider {
                value: 0.25,
                label: "Mix level".into()
            })
        );
    }
}
