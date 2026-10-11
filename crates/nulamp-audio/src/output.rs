//! Audio device output through cpal (07 §6.7).

use std::sync::Arc;
use std::sync::atomic::Ordering;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::mixer::{MixCmd, Mixer, Shared, to_int_dithered};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("no audio output device is available")]
    NoDevice,
    #[error("audio device error: {0}")]
    Device(String),
}

/// Longest stretch of buffered audio kept when the rest is discarded to be written again
/// (decision EQ12), in milliseconds.
pub const MAX_KEEP_MS: u32 = 300;

/// [`MAX_KEEP_MS`] in frames.
pub fn keep_frames(rate: u32) -> usize {
    (rate as u64 * MAX_KEEP_MS as u64 / 1000) as usize
}

/// An open device stream (or an offline sink, see [`Output::offline`]). Dropping it closes the
/// device.
pub struct Output {
    _stream: Option<cpal::Stream>,
    pub rate: u32,
    pub channels: usize,
    /// Interleaved samples for the callback.
    pub producer: Producer<f32>,
    pub commands: Producer<MixCmd>,
    pub shared: Arc<Shared>,
    /// Ring capacity in frames.
    pub capacity_frames: usize,
    /// Most frames a truncation can keep (see [`MixCmd::Truncate`]).
    pub keep_frames: usize,
    /// Set by the error callback when the device went away.
    pub failed: Arc<std::sync::atomic::AtomicBool>,
}

impl Output {
    /// Opens the default device, preferring `want_rate` and a float format.
    pub fn open(want_rate: u32, buffer_ms: u32, shared: Arc<Shared>) -> Result<Self, OutputError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(OutputError::NoDevice)?;
        let default = device
            .default_output_config()
            .map_err(|e| OutputError::Device(e.to_string()))?;
        let channels = default.channels();
        // Prefer the source rate when the device supports it (no resampling).
        let mut chosen = default;
        if let Ok(configs) = device.supported_output_configs() {
            let mut best: Option<cpal::SupportedStreamConfig> = None;
            for c in configs {
                if c.channels() != channels
                    || want_rate < c.min_sample_rate()
                    || want_rate > c.max_sample_rate()
                {
                    continue;
                }
                let cand = c.with_sample_rate(want_rate);
                let better = match &best {
                    None => true,
                    Some(b) => {
                        cand.sample_format() == SampleFormat::F32
                            && b.sample_format() != SampleFormat::F32
                    }
                };
                if better {
                    best = Some(cand);
                }
            }
            if let Some(b) = best {
                chosen = b;
            }
        }
        let format = chosen.sample_format();
        let config: StreamConfig = chosen.config();
        let rate = config.sample_rate;
        let ch = config.channels as usize;
        let capacity_frames = (rate as usize * buffer_ms.max(200) as usize / 1000).max(4096);
        let (producer, consumer) = RingBuffer::<f32>::new(capacity_frames * ch);
        let (commands, cmd_rx) = RingBuffer::<MixCmd>::new(64);
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(
                &device,
                &config,
                consumer,
                cmd_rx,
                shared.clone(),
                failed.clone(),
                0,
            ),
            SampleFormat::I16 => build::<i16>(
                &device,
                &config,
                consumer,
                cmd_rx,
                shared.clone(),
                failed.clone(),
                16,
            ),
            SampleFormat::I32 => build::<i32>(
                &device,
                &config,
                consumer,
                cmd_rx,
                shared.clone(),
                failed.clone(),
                32,
            ),
            SampleFormat::U16 => build::<u16>(
                &device,
                &config,
                consumer,
                cmd_rx,
                shared.clone(),
                failed.clone(),
                16,
            ),
            other => {
                return Err(OutputError::Device(format!(
                    "unsupported sample format {other}"
                )));
            }
        }?;
        stream
            .play()
            .map_err(|e| OutputError::Device(e.to_string()))?;
        log::info!("audio output: {rate} Hz, {ch} channels, {format}");
        Ok(Self {
            _stream: Some(stream),
            rate,
            channels: ch,
            producer,
            commands,
            shared,
            capacity_frames,
            keep_frames: keep_frames(rate),
            failed,
        })
    }

    /// An output with no device: the returned [`DeviceSide`] plays the part of the device
    /// callback and is driven by the caller (offline rendering, tests).
    pub fn offline(
        rate: u32,
        channels: usize,
        buffer_ms: u32,
        shared: Arc<Shared>,
    ) -> (Self, DeviceSide) {
        let ch = channels.max(1);
        let capacity_frames = (rate as usize * buffer_ms.max(200) as usize / 1000).max(4096);
        let (producer, consumer) = RingBuffer::<f32>::new(capacity_frames * ch);
        let (commands, cmd_rx) = RingBuffer::<MixCmd>::new(64);
        let device = DeviceSide::new(consumer, cmd_rx, shared.clone(), rate, ch);
        let out = Self {
            _stream: None,
            rate,
            channels: ch,
            producer,
            commands,
            shared,
            capacity_frames,
            keep_frames: keep_frames(rate),
            failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        (out, device)
    }

    pub fn send(&mut self, cmd: MixCmd) {
        if self.commands.push(cmd).is_err() {
            log::warn!("audio command queue full");
        }
    }

    /// Frames currently buffered and not yet heard.
    pub fn buffered_frames(&self) -> usize {
        (self.producer.buffer().capacity() - self.producer.slots()) / self.channels
    }

    /// Free space in frames.
    pub fn free_frames(&self) -> usize {
        self.producer.slots() / self.channels
    }

    /// Writes interleaved samples; returns how many were written.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let n = (self.producer.slots() / self.channels * self.channels).min(samples.len());
        if n == 0 {
            return 0;
        }
        if let Ok(chunk) = self.producer.write_chunk_uninit(n) {
            chunk.fill_from_iter(samples[..n].iter().copied());
        }
        n
    }

    /// Asks the callback to keep the buffered audio up to output frame `frame` and discard the
    /// rest (see [`MixCmd::Truncate`]); [`Self::truncated`] gives the answer.
    pub fn truncate(&mut self, frame: u64, generation: u64) {
        self.send(MixCmd::Truncate { frame, generation });
    }

    /// The answer to truncation `generation`: `None` while there is none, else whether it was
    /// carried out. It is refused when `frame` was already heard.
    pub fn truncated(&self, generation: u64) -> Option<bool> {
        let ack = self.shared.truncate_ack.load(Ordering::Acquire);
        (ack >> 1 == generation).then_some(ack & 1 == 1)
    }

    /// Discards buffered audio and waits (up to 300 ms) until the callback acknowledges.
    pub fn flush(&mut self, generation: u64) {
        self.send(MixCmd::Flush(generation));
        let t = std::time::Instant::now();
        while self.shared.flush_ack.load(Ordering::Acquire) < generation
            && t.elapsed().as_millis() < 300
        {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Ramps the fade gain and waits until it is reached (bounded).
    pub fn fade_and_wait(&mut self, to: f32, ms: u32) {
        self.send(MixCmd::Fade { to, ms });
        let t = std::time::Instant::now();
        let limit = ms as u128 + 150;
        while (self.shared.fade_gain() - to).abs() > 1e-4 && t.elapsed().as_millis() < limit {
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    consumer: Consumer<f32>,
    cmd_rx: Consumer<MixCmd>,
    shared: Arc<Shared>,
    failed: Arc<std::sync::atomic::AtomicBool>,
    int_bits: u32,
) -> Result<cpal::Stream, OutputError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let mut side = DeviceSide::new(
        consumer,
        cmd_rx,
        shared,
        config.sample_rate,
        config.channels as usize,
    );
    let mut float_buf: Vec<f32> = Vec::new();
    let mut rng = 0x9E37_79B9u32;
    let data = move |out: &mut [T], _: &cpal::OutputCallbackInfo| {
        float_buf.resize(out.len(), 0.0);
        side.render(&mut float_buf);
        for (o, s) in out.iter_mut().zip(float_buf.iter()) {
            *o = if int_bits == 0 {
                T::from_sample(*s)
            } else {
                let i = to_int_dithered(*s, int_bits.min(24), &mut rng) as f32
                    / (1i64 << (int_bits.min(24) - 1)) as f32;
                T::from_sample(i)
            };
        }
    };
    let err = move |e: cpal::Error| {
        log::warn!("audio stream error: {e}");
        if matches!(
            e.kind(),
            cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::DeviceChanged
        ) {
            failed.store(true, Ordering::Relaxed);
        }
    };
    device
        .build_output_stream::<T, _, _>(*config, data, err, None)
        .map_err(|e| OutputError::Device(e.to_string()))
}

/// The consuming end of an output: what a device callback runs. For a device it lives inside the
/// callback; for an offline output the caller drives it.
pub struct DeviceSide {
    mixer: Mixer,
    consumer: Consumer<f32>,
    cmd_rx: Consumer<MixCmd>,
    shared: Arc<Shared>,
    pub rate: u32,
    pub channels: usize,
    /// Audio kept by a truncation, played before the ring: samples `keep_pos..keep_end`.
    /// Allocated up front ([`MAX_KEEP_MS`]) so the callback never allocates.
    keep: Vec<f32>,
    keep_pos: usize,
    keep_end: usize,
}

impl DeviceSide {
    fn new(
        consumer: Consumer<f32>,
        cmd_rx: Consumer<MixCmd>,
        shared: Arc<Shared>,
        rate: u32,
        channels: usize,
    ) -> Self {
        Self {
            mixer: Mixer::new(shared.clone(), rate, channels),
            consumer,
            cmd_rx,
            shared,
            rate,
            channels,
            keep: vec![0.0; keep_frames(rate) * channels],
            keep_pos: 0,
            keep_end: 0,
        }
    }

    /// Applies pending commands, then fills `out` (interleaved, a whole number of frames) as
    /// the device callback would. Returns the number of frames taken from the buffer; frames
    /// after those are silence (underrun, prebuffering or pause).
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        while let Ok(cmd) = self.cmd_rx.pop() {
            if let MixCmd::Truncate { frame, generation } = cmd {
                self.truncate(frame, generation);
            } else if let Some(g) = self.mixer.command(cmd) {
                self.discard_ring();
                self.keep_pos = 0;
                self.keep_end = 0;
                self.shared.flush_ack.store(g, Ordering::Release);
            }
        }
        let before = self.shared.frames_played.load(Ordering::Relaxed);
        let available = self.keep_end - self.keep_pos + self.consumer.slots();
        let (consumer, keep, keep_pos, keep_end) = (
            &mut self.consumer,
            &self.keep,
            &mut self.keep_pos,
            self.keep_end,
        );
        self.mixer.fill(out, available, |dst| {
            let k = dst.len().min(keep_end - *keep_pos);
            dst[..k].copy_from_slice(&keep[*keep_pos..*keep_pos + k]);
            *keep_pos += k;
            let dst = &mut dst[k..];
            let n = dst.len().min(consumer.slots());
            if let Ok(chunk) = consumer.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                dst[..a.len()].copy_from_slice(a);
                dst[a.len()..a.len() + b.len()].copy_from_slice(b);
                chunk.commit_all();
            }
            k + n
        });
        (self.shared.frames_played.load(Ordering::Relaxed) - before) as usize
    }

    fn discard_ring(&mut self) {
        let n = self.consumer.slots();
        if let Ok(chunk) = self.consumer.read_chunk(n) {
            chunk.commit_all();
        }
    }

    /// Keeps the audio before output frame `frame` and discards the rest (decision EQ12).
    /// Refused when `frame` was already heard or the kept part does not fit.
    fn truncate(&mut self, frame: u64, generation: u64) {
        let ch = self.channels;
        let played = self.shared.frames_played.load(Ordering::Relaxed);
        let kept = self.keep_end - self.keep_pos;
        let ring = self.consumer.slots();
        let want = frame
            .checked_sub(played)
            .map(|f| f as usize * ch)
            .filter(|&w| w <= kept + ring && w <= self.keep.len());
        if let Some(want) = want {
            if want <= kept {
                self.keep_end = self.keep_pos + want;
            } else {
                self.keep.copy_within(self.keep_pos..self.keep_end, 0);
                self.keep_pos = 0;
                if let Ok(chunk) = self.consumer.read_chunk(want - kept) {
                    let (a, b) = chunk.as_slices();
                    self.keep[kept..kept + a.len()].copy_from_slice(a);
                    self.keep[kept + a.len()..want].copy_from_slice(b);
                    chunk.commit_all();
                }
                self.keep_end = want;
            }
            self.discard_ring();
        }
        let ack = generation << 1 | u64::from(want.is_some());
        self.shared.truncate_ack.store(ack, Ordering::Release);
    }

    /// Frames waiting in the buffer.
    pub fn buffered_frames(&self) -> usize {
        (self.keep_end - self.keep_pos + self.consumer.slots()) / self.channels
    }

    /// Buffer size in frames.
    pub fn capacity_frames(&self) -> usize {
        self.consumer.buffer().capacity() / self.channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes frames numbered `from..to` (both channels hold the number).
    fn write_frames(out: &mut Output, from: u64, to: u64) {
        let v: Vec<f32> = (from..to).flat_map(|f| [f as f32, f as f32]).collect();
        assert_eq!(out.write(&v), v.len());
    }

    fn render(dev: &mut DeviceSide, frames: usize) -> Vec<f32> {
        let mut buf = vec![0.0; frames * 2];
        let got = dev.render(&mut buf);
        buf.chunks(2).take(got).map(|f| f[0]).collect()
    }

    #[test]
    fn truncate_keeps_the_head_and_takes_new_audio() {
        let shared = Shared::new();
        shared.volume.store(255, Ordering::Relaxed);
        let (mut out, mut dev) = Output::offline(1000, 2, 1000, shared);
        write_frames(&mut out, 0, 4000);
        assert_eq!(
            render(&mut dev, 100),
            (0..100).map(|f| f as f32).collect::<Vec<_>>()
        );
        // Keep up to frame 250, then new audio numbered from 10 000.
        out.truncate(250, 1);
        assert_eq!(out.truncated(1), None);
        render(&mut dev, 0);
        assert_eq!(out.truncated(1), Some(true));
        assert_eq!(dev.buffered_frames(), 150);
        write_frames(&mut out, 10_000, 10_100);
        // A second truncation inside the kept part.
        out.truncate(200, 2);
        let got = render(&mut dev, 50);
        assert_eq!(out.truncated(2), Some(true));
        assert_eq!(got, (100..150).map(|f| f as f32).collect::<Vec<_>>());
        write_frames(&mut out, 20_000, 20_100);
        let got = render(&mut dev, 100);
        let want: Vec<f32> = (150..200).chain(20_000..20_050).map(|f| f as f32).collect();
        assert_eq!(got, want);
        // Already heard: refused, nothing discarded.
        out.truncate(100, 3);
        render(&mut dev, 0);
        assert_eq!(out.truncated(3), Some(false));
        assert_eq!(dev.buffered_frames(), 50);
    }
}
