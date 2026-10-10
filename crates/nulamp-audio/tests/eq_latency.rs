//! Offline tests: an EQ change processes the buffered audio again, so it is heard at once and
//! without a click, across gapless joins, crossfades, pause and seek (decision EQ12).

mod common;

use std::path::{Path, PathBuf};

use common::*;
use nulamp_audio::chain::Stage;
use nulamp_audio::{FadeCurve, OutputPrefs};
use nulamp_core::Command;
use nulamp_eq::EqSettings;

const RATE: u32 = 44_100;
const CH: usize = 2;

/// The longest the change may take to be heard, in frames: 40 ms of margin plus up to 512
/// frames to the next saved state, with some slack.
const MAX_LATENCY: usize = RATE as usize * 80 / 1000;

/// After the change is heard, the EQ glide and filter history settle within this.
const SETTLE: usize = RATE as usize * 300 / 1000;

/// Bypassed: on but flat.
fn flat() -> EqSettings {
    EqSettings {
        enabled: true,
        limiter: false,
        ..EqSettings::default()
    }
}

/// Every band and the preamp at −12 dB.
fn cut() -> EqSettings {
    EqSettings {
        bands: [63; 10],
        preamp: 63,
        ..flat()
    }
}

fn sine_file(dir: &Path, name: &str, freq: f32, secs: f32) -> PathBuf {
    let path = dir.join(name);
    write_wav(
        &path,
        RATE,
        CH as u16,
        &sine(RATE, freq, 0.5, (secs * RATE as f32) as usize),
    );
    path
}

fn rig(prefs: &impl Fn(&mut OutputPrefs)) -> Rig {
    let mut rig = Rig::new(None, CH);
    rig.set_prefs(prefs);
    rig
}

/// Everything heard when `files` play with `eq` from the start.
fn reference(files: &[&Path], eq: &EqSettings, prefs: &impl Fn(&mut OutputPrefs)) -> Vec<f32> {
    let mut rig = rig(prefs);
    rig.player.set_eq(eq);
    rig.add(files);
    rig.player.command(Command::Play);
    rig.run_to_stop(60.0);
    rig.pulled
}

/// Plays `files` with `from`, changes to `to` after `secs` and plays to the end. Returns the
/// rig and the frames heard at the change.
fn change_at(
    files: &[&Path],
    from: &EqSettings,
    to: &EqSettings,
    secs: f32,
    prefs: &impl Fn(&mut OutputPrefs),
) -> (Rig, usize) {
    let mut rig = rig(prefs);
    rig.player.set_eq(from);
    rig.add(files);
    rig.player.command(Command::Play);
    rig.play_for(secs);
    let t = rig.pulled_frames();
    rig.player.set_eq(to);
    let_engine_take_it();
    rig.run_to_stop(60.0);
    (rig, t)
}

/// Gives the engine time to take a change before the rig, which plays faster than real time
/// whenever the buffer is full, renders on.
fn let_engine_take_it() {
    std::thread::sleep(std::time::Duration::from_millis(20));
}

/// First frame at or after `from` where `a` and `b` differ by more than `tol`.
fn first_difference(a: &[f32], b: &[f32], from: usize, tol: f32) -> Option<usize> {
    (from..a.len().min(b.len()) / CH)
        .find(|&f| (0..CH).any(|c| (a[f * CH + c] - b[f * CH + c]).abs() > tol))
}

fn largest_step(samples: &[f32], frames: std::ops::Range<usize>) -> f32 {
    let w = &samples[frames.start * CH..frames.end * CH];
    (0..CH).map(|c| max_step(w, CH, c)).fold(0.0, f32::max)
}

/// The change at frame `t` is heard within [`MAX_LATENCY`], without a click, the audio before
/// it is untouched and the audio after it matches playing with the new settings throughout.
/// Returns the first changed frame.
fn assert_change(got: &[f32], t: usize, old: &[f32], new: &[f32]) -> usize {
    assert_eq!(got.len(), old.len(), "frames were lost or added");
    let d = first_difference(got, old, 0, 1e-4).expect("the change was never heard");
    assert!(d >= t, "changed at {d}, before the change at {t}");
    assert!(
        d - t <= MAX_LATENCY,
        "heard {} ms after the change",
        (d - t) * 1000 / RATE as usize
    );
    let late = first_difference(got, new, d + SETTLE, 1e-3);
    assert_eq!(late, None, "differs from the new settings after settling");
    // No click: no step beyond what either setting produces on its own.
    let around = d.saturating_sub(RATE as usize / 10)..(d + SETTLE).min(got.len() / CH);
    let natural = largest_step(old, around.clone()).max(largest_step(new, around.clone()));
    let step = largest_step(got, around);
    assert!(step < natural * 1.5, "step {step} vs natural {natural}");
    d
}

fn no_prefs(_: &mut OutputPrefs) {}

#[test]
fn change_is_heard_at_once() {
    let dir = test_dir("eqlag");
    let a = sine_file(&dir, "a.wav", 220.0, 6.0);
    let old = reference(&[&a], &flat(), &no_prefs);
    let new = reference(&[&a], &cut(), &no_prefs);
    for (from, to, old, new) in [(flat(), cut(), &old, &new), (cut(), flat(), &new, &old)] {
        let (rig, t) = change_at(&[&a], &from, &to, 2.0, &no_prefs);
        assert_change(&rig.pulled, t, old, new);
    }
}

#[test]
fn same_sound_reprocessed_is_bit_identical() {
    // Auto-load changes nothing audible: the audio is cut and processed again, and must come
    // out sample for sample the same (the kept audio and stage states line up exactly).
    let dir = test_dir("eqsame");
    let a = sine_file(&dir, "a.wav", 220.0, 3.0);
    let b = sine_file(&dir, "b.wav", 330.0, 3.0);
    let xf = |p: &mut OutputPrefs| {
        p.crossfade = true;
        p.crossfade_ms = 1500;
        p.crossfade_curve = FadeCurve::EqualPower;
    };
    let want = reference(&[&a, &b], &cut(), &xf);
    let mut rig = rig(&xf);
    rig.player.set_eq(&cut());
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    let mut toggled = cut();
    let mut revisions = Vec::new();
    // Changes before, during and after the crossfade (it starts at 1.5 s).
    for _ in 0..8 {
        rig.play_for(0.4);
        revisions.push(rig.player.audio().vis.revision());
        toggled.auto_load = !toggled.auto_load;
        rig.player.set_eq(&toggled);
    }
    rig.run_to_stop(60.0);
    assert_eq!(rig.pulled.len(), want.len());
    assert_eq!(first_difference(&rig.pulled, &want, 0, 0.0), None);
    // The vis tap was rewound: the audio was processed again. Changes that come in faster
    // than the engine handles them (this rig plays faster than real time) are coalesced.
    let rewinds = revisions.last().unwrap() - revisions.first().unwrap();
    assert!(rewinds >= 4, "{revisions:?}");
}

#[test]
fn vis_blocks_match_what_is_heard() {
    let dir = test_dir("eqvis");
    let a = sine_file(&dir, "a.wav", 220.0, 4.0);
    let mut rig = rig(&no_prefs);
    rig.add(&[&a]);
    rig.player.command(Command::Play);
    rig.play_for(1.0);
    rig.player.set_eq(&cut());
    rig.play_for(0.5);
    let blocks = rig.player.audio().vis.since(u64::MAX);
    assert!(blocks.len() > 100);
    // Contiguous: replaced blocks are gone, none overlaps.
    for w in blocks.windows(2) {
        assert_eq!(w[1].frame, w[0].frame + (w[0].samples.len() / CH) as u64);
    }
    // At volume 255 the heard audio is the tapped audio, including after the change.
    let heard = rig.pulled_frames() as u64;
    let mut checked = 0;
    for b in blocks.iter().filter(|b| b.frame + 576 <= heard) {
        let at = b.frame as usize * CH;
        assert_eq!(
            b.samples,
            rig.pulled[at..at + b.samples.len()],
            "{}",
            b.frame
        );
        checked += 1;
    }
    assert!(checked > 30, "{checked}");
}

#[test]
fn change_across_a_gapless_join() {
    // One sine in two files; the join (3 s) is inside the buffer at the change (2 s).
    let dir = test_dir("eqgapless");
    let all = sine(RATE, 220.0, 0.5, 6 * RATE as usize);
    let cut_at = (3 * RATE as usize + 17) * CH;
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    write_wav(&a, RATE, 2, &all[..cut_at]);
    write_wav(&b, RATE, 2, &all[cut_at..]);
    let old = reference(&[&a, &b], &flat(), &no_prefs);
    let new = reference(&[&a, &b], &cut(), &no_prefs);
    let (rig, t) = change_at(&[&a, &b], &flat(), &cut(), 2.0, &no_prefs);
    assert_change(&rig.pulled, t, &old, &new);
    // The join stays gapless: a continuous sine all along.
    let whole = decode_all(&a)
        .into_iter()
        .chain(decode_all(&b))
        .collect::<Vec<_>>();
    assert_eq!(&rig.pulled[..t * CH], &whole[..t * CH]);
}

#[test]
fn change_during_a_crossfade() {
    let dir = test_dir("eqxf");
    let a = sine_file(&dir, "a.wav", 220.0, 4.0);
    let b = sine_file(&dir, "b.wav", 330.0, 4.0);
    let xf = |p: &mut OutputPrefs| {
        p.crossfade = true;
        p.crossfade_ms = 2000;
    };
    let old = reference(&[&a, &b], &flat(), &xf);
    let new = reference(&[&a, &b], &cut(), &xf);
    // The overlap is 2–4 s of output: change in the middle of it.
    let (rig, t) = change_at(&[&a, &b], &flat(), &cut(), 2.8, &xf);
    assert_change(&rig.pulled, t, &old, &new);
    assert_eq!(rig.pulled_frames(), 6 * RATE as usize);
}

#[test]
fn change_while_paused() {
    let dir = test_dir("eqpause");
    let a = sine_file(&dir, "a.wav", 220.0, 5.0);
    let no_fade = |p: &mut OutputPrefs| p.fade_pause_stop = false;
    let old = reference(&[&a], &flat(), &no_fade);
    let new = reference(&[&a], &cut(), &no_fade);
    let mut rig = rig(&no_fade);
    rig.player.set_eq(&flat());
    rig.add(&[&a]);
    rig.player.command(Command::Play);
    rig.play_for(1.5);
    rig.player.command(Command::Pause);
    for _ in 0..20 {
        rig.step();
    }
    let t = rig.pulled_frames();
    rig.player.set_eq(&cut());
    let_engine_take_it();
    for _ in 0..20 {
        rig.step();
    }
    assert_eq!(rig.pulled_frames(), t, "paused");
    rig.player.command(Command::Pause);
    rig.run_to_stop(60.0);
    // Without a fade the block playing at the pause is silenced; the rest is the track.
    let mut got = rig.pulled.clone();
    let p = (t - BLOCK) * CH;
    assert!(got[p..t * CH].iter().all(|s| *s == 0.0));
    got[p..t * CH].copy_from_slice(&old[p..t * CH]);
    assert_change(&got, t, &old, &new);
}

#[test]
fn change_then_seek() {
    let dir = test_dir("eqseek");
    let a = sine_file(&dir, "a.wav", 220.0, 5.0);
    let no_fade = |p: &mut OutputPrefs| p.fade_seek = false;
    let old = reference(&[&a], &flat(), &no_fade);
    let new = reference(&[&a], &cut(), &no_fade);
    for steps in [0, 1, 3] {
        let mut rig = rig(&no_fade);
        rig.add(&[&a]);
        rig.player.command(Command::Play);
        rig.play_for(1.5);
        let t = rig.pulled_frames();
        rig.player.set_eq(&cut());
        for _ in 0..steps {
            rig.step();
        }
        rig.player.command(Command::SeekTo(500));
        rig.run_to_stop(60.0);
        assert_eq!(&rig.pulled[..t * CH], &old[..t * CH]);
        // From the seek on: the rest of the track from 0.5 s, with the new settings.
        let rest = new.len() - RATE as usize / 2 * CH;
        let n = rig.pulled.len();
        assert!(n >= rest + t * CH, "{n} < {rest} + {t}");
        let settle = SETTLE * CH;
        assert_eq!(
            first_difference(
                &rig.pulled[n - rest + settle..],
                &new[new.len() - rest + settle..],
                0,
                1e-3
            ),
            None,
            "{steps} steps"
        );
    }
}

/// An effect that cannot save its state.
struct Pass;

impl Stage for Pass {
    fn configure(&mut self, _: u32, _: usize) {}
    fn process(&mut self, _: &mut [f32]) {}
}

#[test]
fn dsp_stage_without_saved_state_keeps_the_buffer() {
    let dir = test_dir("eqdsp");
    let a = sine_file(&dir, "a.wav", 220.0, 5.0);
    let old = reference(&[&a], &flat(), &no_prefs);
    let mut rig = rig(&no_prefs);
    rig.player.set_dsp(Some(Box::new(Pass)));
    rig.add(&[&a]);
    rig.player.command(Command::Play);
    rig.play_for(1.0);
    let t = rig.pulled_frames();
    rig.player.set_eq(&cut());
    rig.run_to_stop(60.0);
    assert_eq!(rig.pulled.len(), old.len());
    // The change is heard only once the buffered audio has played.
    let d = first_difference(&rig.pulled, &old, 0, 1e-4).expect("changed");
    assert!(d >= t + RATE as usize, "{d} {t}");
}
