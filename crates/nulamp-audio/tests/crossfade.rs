//! Offline audio conformance tests for crossfade (07 §6.4, 12 §6).

mod common;

use common::*;
use nulamp_audio::{FadeCurve, PlayerEvent};
use nulamp_core::Command;

const RATE: u32 = 44_100;

/// Two tracks: A is 0.5 on the left only, B is 0.5 on the right only, so the overlap is where
/// both channels sound.
fn left_right(name: &str, a_secs: f32, b_secs: f32) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = test_dir(name);
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    write_wav(
        &a,
        RATE,
        2,
        &dc((a_secs * RATE as f32) as usize, &[0.5, 0.0]),
    );
    write_wav(
        &b,
        RATE,
        2,
        &dc((b_secs * RATE as f32) as usize, &[0.0, 0.5]),
    );
    (a, b)
}

fn overlap_frames(samples: &[f32]) -> usize {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|f| f[0] > 0.0 && f[1] > 0.0)
        .count()
}

fn first_overlap(samples: &[f32]) -> usize {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .position(|f| f[0] > 0.0 && f[1] > 0.0)
        .expect("overlap")
}

fn crossfade_rig(ms: u32, curve: FadeCurve) -> Rig {
    let mut rig = Rig::new(None, 2);
    rig.set_prefs(|p| {
        p.crossfade = true;
        p.crossfade_ms = ms;
        p.crossfade_curve = curve;
    });
    rig
}

#[test]
fn crossfade_5s_overlaps_5s() {
    let (a, b) = left_right("xf5", 8.0, 8.0);
    let mut rig = crossfade_rig(5000, FadeCurve::Linear);
    let ids = rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    let overlap = overlap_frames(&rig.pulled) as f32 * 1000.0 / RATE as f32;
    assert!((overlap - 5000.0).abs() <= 10.0, "overlap {overlap} ms");
    // Nothing was lost or added: 8 s + 8 s − 5 s.
    let total = rig.pulled_frames() as f32 * 1000.0 / RATE as f32;
    assert!((total - 11_000.0).abs() <= 10.0, "total {total} ms");
    // Linear: the two gains always sum to one, without clicks.
    for f in rig.pulled.as_chunks::<2>().0 {
        assert!((f[0] + f[1] - 0.5).abs() < 1e-4, "{f:?}");
    }
    for c in 0..2 {
        assert!(max_step(&rig.pulled, 2, c) <= 0.01);
    }
    // The second track counts as started (time display, events) when it starts being heard:
    // its position 0 is the first frame of the overlap.
    let start = first_overlap(&rig.pulled);
    let (at, _) = rig
        .events
        .iter()
        .find(|(_, e)| *e == PlayerEvent::TrackStarted(ids[1]))
        .expect("second track started");
    assert!(
        *at + BLOCK >= start,
        "started at {at}, overlap from {start}"
    );
    let h = rig
        .heard
        .iter()
        .find(|h| h.token == Some(ids[1].0))
        .expect("second track heard");
    let zero = h.pulled as i64 - (h.position_ms * RATE as u64 / 1000) as i64;
    assert!(
        (zero - start as i64).abs() <= RATE as i64 / 100,
        "position 0 at frame {zero}, overlap from {start}"
    );
}

#[test]
fn crossfade_does_not_clip_with_the_limiter() {
    let dir = test_dir("xfclip");
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    let n = 7 * RATE as usize;
    write_wav(&a, RATE, 2, &sine(RATE, 440.0, 0.9, n));
    write_wav(&b, RATE, 2, &sine(RATE, 660.0, 0.9, n));
    // Linear: the mix never exceeds the louder track.
    let mut rig = crossfade_rig(5000, FadeCurve::Linear);
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    assert!(peak(&rig.pulled) <= 0.9 + 1e-3, "{}", peak(&rig.pulled));
    assert!(overlap_secs_by_length(&rig, 14.0, 5.0));
    // Equal power sums up to +3 dB: limited to the limiter ceiling.
    let mut rig = crossfade_rig(5000, FadeCurve::EqualPower);
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    let p = peak(&rig.pulled);
    assert!(p <= nulamp_eq::LIMITER_THRESHOLD + 1e-4, "{p}");
    assert!(overlap_secs_by_length(&rig, 14.0, 5.0));
    // With the EQ boosting +12 dB and its limiter on, the mixed output stays below the
    // ceiling as well.
    let mut rig = crossfade_rig(5000, FadeCurve::Linear);
    let eq = nulamp_eq::EqSettings {
        enabled: true,
        bands: [0; 10],
        ..Default::default()
    };
    rig.player.set_eq(&eq);
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    let p = peak(&rig.pulled);
    assert!(p <= nulamp_eq::LIMITER_THRESHOLD + 1e-4, "{p}");
}

/// The output is `total − overlap` seconds long (± 10 ms).
fn overlap_secs_by_length(rig: &Rig, total: f32, overlap: f32) -> bool {
    let got = rig.pulled_frames() as f32 / RATE as f32;
    (got - (total - overlap)).abs() <= 0.01
}

#[test]
fn crossfade_mixes_different_formats() {
    let dir = test_dir("xfformats");
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    write_wav(&a, RATE, 2, &dc(4 * RATE as usize, &[0.5, 0.0]));
    // Half the rate, mono: resampled and mapped to both channels before mixing.
    write_wav(&b, 22_050, 1, &dc(4 * 22_050, &[0.5]));
    let mut rig = crossfade_rig(2000, FadeCurve::Linear);
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    assert_eq!(rig.rate(), RATE);
    let rising = rig
        .pulled
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|f| f[1] > 1e-3 && f[1] < 0.5 - 1e-3)
        .count() as f32
        * 1000.0
        / RATE as f32;
    assert!((rising - 2000.0).abs() <= 10.0, "overlap {rising} ms");
    let total = rig.pulled_frames() as f32 * 1000.0 / RATE as f32;
    assert!((total - 6000.0).abs() <= 10.0, "total {total} ms");
    assert!(peak(&rig.pulled) <= 0.51);
}

#[test]
fn same_album_joins_gaplessly() {
    let (a, b) = left_right("xfalbum", 3.0, 3.0);
    let mut rig = crossfade_rig(1000, FadeCurve::Linear);
    rig.player.album_of = Box::new(|_| Some("album".into()));
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(20.0);
    assert_eq!(overlap_frames(&rig.pulled), 0);
    assert_eq!(rig.pulled_frames(), 6 * RATE as usize);
    // With "gapless for albums" off, the same tracks crossfade.
    let mut rig = crossfade_rig(1000, FadeCurve::Linear);
    rig.set_prefs(|p| p.gapless_albums = false);
    rig.player.album_of = Box::new(|_| Some("album".into()));
    rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    rig.run_to_stop(20.0);
    let overlap = overlap_frames(&rig.pulled) as f32 * 1000.0 / RATE as f32;
    assert!((overlap - 1000.0).abs() <= 10.0, "overlap {overlap} ms");
}

#[test]
fn short_tracks_and_the_end_of_the_list() {
    // A track shorter than the crossfade is joined gaplessly and crossfades into the next one
    // over its whole length; the last track of the list plays to its end.
    let dir = test_dir("xfshort");
    let paths: Vec<_> = (0..3).map(|i| dir.join(format!("{i}.wav"))).collect();
    write_wav(&paths[0], RATE, 2, &dc(3 * RATE as usize, &[0.5, 0.0]));
    write_wav(&paths[1], RATE, 2, &dc(RATE as usize / 2, &[0.0, 0.5]));
    write_wav(&paths[2], RATE, 2, &dc(3 * RATE as usize, &[0.5, 0.0]));
    let mut rig = crossfade_rig(1000, FadeCurve::Linear);
    let refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
    rig.add(&refs);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    let total = rig.pulled_frames() as f32 / RATE as f32;
    // 3 + 0.5 + 3 − 0.5.
    assert!((total - 6.0).abs() <= 0.01, "total {total} s");
    // No overlap between the first two tracks.
    let first_right = channel(&rig.pulled, 2, 1)
        .iter()
        .position(|s| *s > 0.0)
        .expect("second track");
    assert_eq!(first_right, 3 * RATE as usize);
    assert!(peak(&rig.pulled) <= 0.5 + 1e-4);
    // The end is not faded.
    let n = rig.pulled.len();
    assert!((rig.pulled[n - 2] - 0.5).abs() < 1e-6);
}

#[test]
fn manual_change_crossfades_only_when_enabled() {
    let dir = test_dir("xfmanual");
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    // A rises slowly on the left, so a skip or a repeat shows as a jump.
    let n = 6 * RATE as usize;
    let ramp: Vec<f32> = (0..n)
        .flat_map(|i| [0.1 + 0.4 * i as f32 / n as f32, 0.0])
        .collect();
    write_wav(&a, RATE, 2, &ramp);
    write_wav(&b, RATE, 2, &dc(4 * RATE as usize, &[0.0, 0.5]));
    for manual in [false, true] {
        let mut rig = crossfade_rig(1000, FadeCurve::Linear);
        rig.set_prefs(|p| p.crossfade_manual = manual);
        rig.add(&[&a, &b]);
        rig.player.command(Command::Play);
        rig.play_for(1.5);
        rig.player.command(Command::Next);
        rig.run_to_stop(20.0);
        let overlap = overlap_frames(&rig.pulled) as f32 * 1000.0 / RATE as f32;
        if manual {
            assert!((overlap - 1000.0).abs() <= 10.0, "overlap {overlap} ms");
            // The old track continues from where it was heard (within the 1 ms resolution
            // of the position), then fades.
            let s = first_overlap(&rig.pulled);
            let (before, at) = (rig.pulled[(s - 1) * 2], rig.pulled[s * 2]);
            let ms = 0.4 / 6000.0;
            assert!(
                (at - before).abs() <= 2.0 * ms,
                "jump from {before} to {at}"
            );
            assert!(max_step(&rig.pulled, 2, 1) <= 0.01);
        } else {
            // A plain cut.
            assert_eq!(overlap, 0.0);
        }
    }
}

#[test]
fn seek_during_crossfade_and_streams_are_not_crossfaded() {
    let (a, b) = left_right("xfseek", 4.0, 6.0);
    let mut rig = crossfade_rig(2000, FadeCurve::Linear);
    let ids = rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    // Into the overlap (it starts at 2 s).
    rig.play_for(2.5);
    assert_eq!(rig.player.engine_status().token, Some(ids[1].0));
    rig.player.command(Command::SeekTo(3000));
    rig.run_to_stop(20.0);
    // After the seek only the second track plays, from 3 s.
    let n = rig.pulled.len();
    assert!(rig.pulled[n - 2] == 0.0 && (rig.pulled[n - 1] - 0.5).abs() < 1e-6);
}
