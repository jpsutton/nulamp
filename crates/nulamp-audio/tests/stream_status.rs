//! The player reports what a network stream learns after it opened: its length, whether it
//! seeks, and how much of it is downloaded.

mod common;

use common::Rig;
use nulamp_core::{Command, NewEntry};
use nulamp_decoders::{DecodeError, Decoder, DecoderFactory, ReadStatus, Registry, StreamInfo};

const RATE: u32 = 44_100;
const LENGTH_MS: u64 = 4000;

/// A stream that is unknown at open and becomes a seekable file of known length once it
/// delivers audio, as a network stream does when its headers arrive.
struct LateStream {
    info: StreamInfo,
    left: usize,
}

impl Decoder for LateStream {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<ReadStatus, DecodeError> {
        self.info.seekable = true;
        self.info.length_ms = Some(LENGTH_MS);
        if self.left == 0 {
            return Ok(ReadStatus::End);
        }
        let n = self.left.min(1024);
        self.left -= n;
        out.extend(std::iter::repeat_n(0.25, n * 2));
        Ok(ReadStatus::Frames(n))
    }

    fn seek(&mut self, ms: u64) -> Result<u64, DecodeError> {
        let total = (LENGTH_MS * RATE as u64 / 1000) as usize;
        self.left = total - (ms * RATE as u64 / 1000) as usize;
        Ok(ms)
    }

    fn kbps(&self) -> Option<u32> {
        None
    }

    fn buffered_ms(&self) -> Option<(u64, u64)> {
        self.info.seekable.then_some((0, 2500))
    }
}

struct LateFactory;

impl DecoderFactory for LateFactory {
    fn name(&self) -> &str {
        "late"
    }

    fn extensions(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn claims(&self, location: &str) -> bool {
        location.starts_with("http://late/")
    }

    fn open(&self, _: &str) -> Result<Box<dyn Decoder>, DecodeError> {
        Ok(Box::new(LateStream {
            info: StreamInfo {
                sample_rate: RATE,
                channels: 2,
                ..StreamInfo::default()
            },
            left: (LENGTH_MS * RATE as u64 / 1000) as usize,
        }))
    }
}

#[test]
fn status_follows_the_stream_after_it_opened() {
    let mut registry = Registry::empty();
    registry.add(Box::new(LateFactory));
    let mut rig = Rig::with_registry(registry, Some(RATE), 2);
    rig.player
        .engine
        .append(vec![NewEntry::new("http://late/a.mp3")]);
    rig.player.command(Command::Play);
    rig.play_for(0.5);
    let st = rig.player.status();
    assert!(st.seekable);
    assert_eq!(st.length_ms, Some(LENGTH_MS));
    assert_eq!(st.buffered_ms, Some((0, 2500)));

    rig.player.command(Command::SeekTo(3000));
    // The position holds while the output buffer refills after the seek.
    rig.run_until(5.0, |r| r.player.status().position_ms >= 3000);
    let st = rig.player.status();
    assert!((3000..3600).contains(&st.position_ms), "{}", st.position_ms);
    rig.run_to_stop(10.0);
    assert_eq!(rig.player.status().buffered_ms, None);
}
