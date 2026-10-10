//! Offline gapless conformance tests (07 §6.3, 12 §6), and checks that crossfade leaves
//! streams, pending reads and resizing DSP stages working.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command as Process;

use common::*;
use nulamp_audio::FadeCurve;
use nulamp_audio::chain::Stage;
use nulamp_core::Command;
use nulamp_decoders::{DecodeError, Decoder, DecoderFactory, ReadStatus, Registry, StreamInfo};

const RATE: u32 = 44_100;

/// One stereo sine written whole and in two parts split at an odd frame.
fn split_sine(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = test_dir(name);
    let all = sine(RATE, 441.0, 0.5, 3 * RATE as usize + 77);
    let cut = 70_001 * 2;
    let (whole, a, b) = (dir.join("whole.wav"), dir.join("a.wav"), dir.join("b.wav"));
    write_wav(&whole, RATE, 2, &all);
    write_wav(&a, RATE, 2, &all[..cut]);
    write_wav(&b, RATE, 2, &all[cut..]);
    (whole, a, b)
}

fn play(rig: &mut Rig, files: &[&Path]) {
    rig.add(files);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
}

fn assert_identical(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len(), "length differs");
    let first = got.iter().zip(want).position(|(a, b)| a != b);
    assert_eq!(first, None, "first difference at sample {first:?}");
}

#[test]
fn two_wav_parts_play_sample_identical() {
    let (whole, a, b) = split_sine("gapless");
    let mut rig = Rig::new(None, 2);
    play(&mut rig, &[&a, &b]);
    assert_identical(&rig.pulled, &decode_all(&whole));
}

#[test]
fn same_album_stays_gapless_with_crossfade_on() {
    let (whole, a, b) = split_sine("gaplessxf");
    let mut rig = Rig::new(None, 2);
    rig.set_prefs(|p| {
        p.crossfade = true;
        p.crossfade_ms = 1000;
        p.crossfade_curve = FadeCurve::EqualPower;
    });
    rig.player.album_of = Box::new(|_| Some("one album".into()));
    play(&mut rig, &[&a, &b]);
    assert_identical(&rig.pulled, &decode_all(&whole));
}

#[test]
fn seek_after_the_next_track_was_queued_stays_in_the_heard_track() {
    let dir = test_dir("seekqueued");
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    write_wav(&a, RATE, 2, &dc(4 * RATE as usize, &[0.5, 0.0]));
    write_wav(&b, RATE, 2, &dc(4 * RATE as usize, &[0.0, 0.5]));
    let mut rig = Rig::new(None, 2);
    let ids = rig.add(&[&a, &b]);
    rig.player.command(Command::Play);
    // The second track is decoding by now; the first one is still heard.
    rig.play_for(3.6);
    rig.player.command(Command::SeekTo(1000));
    rig.play_for(0.6);
    let st = rig.player.status();
    assert_eq!(st.position, 0, "playlist position");
    assert!((1000..1700).contains(&st.position_ms), "{}", st.position_ms);
    rig.run_to_stop(30.0);
    let started = |id| {
        rig.events
            .iter()
            .filter(|(_, e)| *e == nulamp_audio::PlayerEvent::TrackStarted(id))
            .count()
    };
    assert_eq!((started(ids[0]), started(ids[1])), (1, 1));
    // 3.6 s, then (opened again, without a seek fade) 3 s from the new position; then the
    // whole second track.
    let left = channel(&rig.pulled, 2, 0)
        .iter()
        .filter(|s| **s > 0.0)
        .count();
    let left_s = left as f32 / RATE as f32;
    assert!((left_s - 6.6).abs() < 0.1, "first track heard {left_s} s");
    let right = channel(&rig.pulled, 2, 1)
        .iter()
        .filter(|s| **s > 0.0)
        .count();
    assert_eq!(right, 4 * RATE as usize);
}

/// Encodes a WAV file with ffmpeg; `None` when ffmpeg is not installed or lacks the encoder
/// (Homebrew's ffmpeg, for example, has no libvorbis).
fn ffmpeg(input: &Path, output: &Path, args: &[&str]) -> Option<()> {
    let status = Process::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(input)
        .args(args)
        .arg(output)
        .status()
        .ok()?;
    if !status.success() {
        eprintln!("ffmpeg could not encode {}", output.display());
        return None;
    }
    Some(())
}

#[test]
fn flac_album_parts_play_sample_identical() {
    let (whole, a, b) = split_sine("gaplessflac");
    let (fa, fb) = (a.with_extension("flac"), b.with_extension("flac"));
    if ffmpeg(&a, &fa, &[]).is_none() || ffmpeg(&b, &fb, &[]).is_none() {
        eprintln!("ffmpeg or its encoder not available: skipped");
        return;
    }
    let mut rig = Rig::new(None, 2);
    play(&mut rig, &[&fa, &fb]);
    assert_identical(&rig.pulled, &decode_all(&whole));
}

/// Lossy album parts cannot match the WAV sample for sample, but must join with no gap and no
/// overlap: the output is the two decoded parts back to back, and the decoders trim encoder
/// delay and padding so the total length is the original's.
#[test]
fn lossy_album_parts_join_without_gap() {
    let (whole, a, b) = split_sine("gaplesslossy");
    let frames = decode_all(&whole).len() / 2;
    for (ext, args, rate) in [
        ("mp3", ["-c:a", "libmp3lame", "-b:a", "256k"], RATE),
        ("ogg", ["-c:a", "libvorbis", "-q:a", "8"], RATE),
        ("opus", ["-c:a", "libopus", "-b:a", "256k"], 48_000),
    ] {
        let (ea, eb) = (a.with_extension(ext), b.with_extension(ext));
        if ffmpeg(&a, &ea, &args).is_none() || ffmpeg(&b, &eb, &args).is_none() {
            eprintln!("{ext}: ffmpeg or its encoder not available: skipped");
            continue;
        }
        let mut rig = Rig::new(None, 2);
        play(&mut rig, &[&ea, &eb]);
        let want = frames as f64 * rate as f64 / RATE as f64;
        let got = rig.pulled_frames() as f64;
        assert!(
            (got - want).abs() <= 2.0,
            "{ext}: {got} frames, want {want}"
        );
        let mut parts = decode_all(&ea);
        parts.extend(decode_all(&eb));
        assert_identical(&rig.pulled, &parts);
    }
}

/// A decoder of a constant 0.25 that is "pending" every other read, with or without a length.
struct Flaky {
    info: StreamInfo,
    left: usize,
    tick: bool,
}

impl Decoder for Flaky {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        self.tick = !self.tick;
        if self.tick {
            return Ok(ReadStatus::Pending);
        }
        if self.left == 0 {
            return Ok(ReadStatus::End);
        }
        let n = self.left.min(1000);
        self.left -= n;
        out.extend(std::iter::repeat_n(0.25, n * 2));
        Ok(ReadStatus::Frames(n))
    }

    fn seek(&mut self, _: u64) -> Result<u64, DecodeError> {
        Err(DecodeError::Seek("not seekable".into()))
    }

    fn kbps(&self) -> Option<u32> {
        None
    }
}

struct FlakyFactory;

impl DecoderFactory for FlakyFactory {
    fn name(&self) -> &str {
        "flaky"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        vec![]
    }

    fn claims(&self, location: &str) -> bool {
        location.starts_with("flaky://")
    }

    fn open(&self, location: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        Ok(Box::new(Flaky {
            info: StreamInfo {
                sample_rate: RATE,
                channels: 2,
                // "flaky://live" is a live stream of unknown length.
                length_ms: (!location.ends_with("live")).then_some(2000),
                ..Default::default()
            },
            left: 2 * RATE as usize,
            tick: false,
        }))
    }
}

#[test]
fn pending_streams_play_whole_and_are_not_crossfaded() {
    let dir = test_dir("pending");
    let a = dir.join("a.wav");
    write_wav(&a, RATE, 2, &dc(2 * RATE as usize, &[0.5, 0.5]));
    let mut reg = Registry::with_defaults();
    reg.add(Box::new(FlakyFactory));
    let mut rig = Rig::with_registry(reg, None, 2);
    rig.set_prefs(|p| {
        p.crossfade = true;
        p.crossfade_ms = 1000;
    });
    rig.player
        .engine
        .append(vec![nulamp_core::NewEntry::new("flaky://live")]);
    rig.add(&[&a]);
    rig.player
        .engine
        .append(vec![nulamp_core::NewEntry::new("flaky://known-length")]);
    rig.player.command(Command::Play);
    rig.run_to_stop(30.0);
    // 2 s of stream, the file, then 2 s more of stream: all whole, none overlapping.
    let want: Vec<f32> = [0.25, 0.5, 0.25]
        .iter()
        .flat_map(|v| dc(2 * RATE as usize, &[*v, *v]))
        .collect();
    assert_identical(&rig.pulled, &want);
}

/// A time-stretch stand-in: every frame twice.
struct Double;

impl Stage for Double {
    fn configure(&mut self, _: u32, _: usize) {}
    fn process(&mut self, _: &mut [f32]) {
        unreachable!("the chain uses the resizing path");
    }
    fn process_resizing(&mut self, samples: &mut Vec<f32>) {
        *samples = samples
            .chunks(2)
            .flat_map(|f| [f, f])
            .flatten()
            .copied()
            .collect();
    }
}

#[test]
fn resizing_dsp_stage_with_crossfade() {
    let dir = test_dir("dsp");
    let (a, b) = (dir.join("a.wav"), dir.join("b.wav"));
    write_wav(&a, RATE, 2, &dc(3 * RATE as usize, &[0.5, 0.0]));
    write_wav(&b, RATE, 2, &dc(3 * RATE as usize, &[0.0, 0.5]));
    let mut rig = Rig::new(None, 2);
    rig.set_prefs(|p| {
        p.crossfade = true;
        p.crossfade_ms = 1000;
    });
    rig.player.set_dsp(Some(Box::new(Double)));
    play(&mut rig, &[&a, &b]);
    // (3 + 3 − 1) s, doubled by the stage after mixing.
    assert_eq!(rig.pulled_frames(), 10 * RATE as usize);
    let overlap = rig
        .pulled
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|f| f[0] > 0.0 && f[1] > 0.0)
        .count();
    assert_eq!(overlap, 2 * RATE as usize);
}
