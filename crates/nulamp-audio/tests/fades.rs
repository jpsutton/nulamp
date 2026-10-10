//! Offline audio conformance tests (12 §6): volume, balance and the pause, stop and seek fades
//! of 07 §2.3, §6.2 and §6.6.

mod common;

use common::*;
use nulamp_audio::PlayerEvent;
use nulamp_core::Command;

const RATE: u32 = 44_100;
/// −40 dB: the largest allowed step between consecutive samples at a fade edge (12 §6).
const MAX_STEP: f32 = 0.01;

/// Expects a fade of `ms` ± 5 ms.
fn assert_len(start: usize, end: usize, ms: f32) {
    let got = (end - start) as f32 * 1000.0 / RATE as f32;
    assert!(
        (got - ms).abs() <= 5.0,
        "fade lasted {got} ms, expected {ms}"
    );
}

/// A rig playing a constant 0.5 on both channels.
fn dc_rig(name: &str, secs: f32) -> Rig {
    let dir = test_dir(name);
    let path = dir.join("dc.wav");
    write_wav(
        &path,
        RATE,
        2,
        &dc((secs * RATE as f32) as usize, &[0.5, 0.5]),
    );
    let mut rig = Rig::new(None, 2);
    rig.add(&[&path]);
    rig
}

#[test]
fn volume_200_is_minus_2_11_db() {
    let mut rig = dc_rig("volume", 2.0);
    rig.player.command(Command::SetVolume(200));
    rig.player.command(Command::Play);
    rig.play_for(0.5);
    let gain = rig.pulled[rig.pulled.len() - 2] / 0.5;
    assert!((gain - 200.0 / 255.0).abs() < 1e-6, "{gain}");
    assert!((20.0 * gain.log10() + 2.11).abs() < 0.01);
}

#[test]
fn balance_plus_64_halves_the_left_channel() {
    let mut rig = dc_rig("balance", 2.0);
    rig.player.command(Command::Play);
    rig.play_for(0.3);
    rig.player.command(Command::SetBalance(64));
    rig.play_for(0.3);
    let n = rig.pulled.len();
    let (l, r) = (rig.pulled[n - 2], rig.pulled[n - 1]);
    assert!((l - 0.25).abs() < 1e-6, "left {l}");
    assert!((r - 0.5).abs() < 1e-6, "right {r}");
}

#[test]
fn pause_fades_out_and_in_over_333_ms() {
    let mut rig = dc_rig("pause", 4.0);
    rig.player.command(Command::Play);
    rig.play_for(0.5);
    let from = rig.timeline_frames();
    rig.player.command(Command::Pause);
    rig.run_until(5.0, |r| r.timeline.last() == Some(&0.0));
    // Some silent blocks while paused.
    for _ in 0..8 {
        rig.step();
    }
    let (start, end) = find_fade(&rig.timeline, 2, 0, from, 0.5, 0.0);
    assert_len(start, end, 333.0);
    let resume = rig.timeline_frames();
    rig.player.command(Command::Pause);
    rig.play_for(0.6);
    let (start2, end2) = find_fade(&rig.timeline, 2, 0, resume, 0.0, 0.5);
    assert_len(start2, end2, 333.0);
    for c in 0..2 {
        assert!(max_step(&rig.timeline[from * 2..], 2, c) <= MAX_STEP);
    }
    // Unpausing resumes without a jump: the position is the audio heard (within 50 ms).
    let heard_ms = rig.pulled_frames() as f32 * 1000.0 / RATE as f32;
    let pos = rig.player.status().position_ms as f32;
    assert!(
        (pos - heard_ms).abs() <= 50.0,
        "position {pos}, heard {heard_ms}"
    );
}

#[test]
fn stop_fades_out_over_333_ms() {
    let mut rig = dc_rig("stop", 4.0);
    rig.player.command(Command::Play);
    rig.play_for(0.5);
    let from = rig.timeline_frames();
    rig.player.command(Command::Stop);
    for _ in 0..30 {
        rig.step();
    }
    let (start, end) = find_fade(&rig.timeline, 2, 0, from, 0.5, 0.0);
    assert_len(start, end, 333.0);
    assert!(max_step(&rig.timeline[from * 2..], 2, 0) <= MAX_STEP);
    assert!(
        rig.timeline[end * 2..].iter().all(|s| *s == 0.0),
        "audio after the stop fade"
    );
}

#[test]
fn seek_fades_out_and_in_over_333_ms() {
    let mut rig = dc_rig("seek", 6.0);
    rig.player.command(Command::Play);
    rig.play_for(0.5);
    let from = rig.timeline_frames();
    rig.player.command(Command::SeekTo(3000));
    rig.play_for(1.0);
    let (start, end) = find_fade(&rig.timeline, 2, 0, from, 0.5, 0.0);
    assert_len(start, end, 333.0);
    let (start2, end2) = find_fade(&rig.timeline, 2, 0, end, 0.0, 0.5);
    assert_len(start2, end2, 333.0);
    assert!(max_step(&rig.timeline[from * 2..], 2, 0) <= MAX_STEP);
    let pos = rig.player.status().position_ms;
    assert!((3300..3900).contains(&pos), "position {pos}");
}

#[test]
fn stop_with_fade_ramps_over_2048_ms() {
    let mut rig = dc_rig("stopfade", 8.0);
    rig.player.command(Command::Play);
    rig.play_for(0.5);
    let from = rig.timeline_frames();
    rig.player.command(Command::StopWithFade);
    rig.run_until(10.0, |r| {
        r.timeline_frames() > from + RATE as usize * 5 / 2
            && r.player.engine_status().token.is_none()
    });
    let (start, end) = find_fade(&rig.timeline, 2, 0, from, 0.5, 0.0);
    assert_len(start, end, 2048.0);
    // The buffer stays filled: the ramp is not cut short by an underrun.
    let v = channel(&rig.timeline, 2, 0);
    let mid = v[(start + end) / 2];
    assert!((mid - 0.25).abs() < 0.01, "{mid}");
    assert!(max_step(&rig.timeline[from * 2..], 2, 0) <= MAX_STEP);
    assert!(rig.saw(&PlayerEvent::Stopped));
}
