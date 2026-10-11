//! The equalizer as a processing stage (07 §4 stage 3).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nulamp_eq::{EqSettings, Equalizer};

use crate::chain::{Stage, StageState};

/// Settings shared between the user interface and the audio thread.
#[derive(Debug, Default)]
pub struct SharedEq {
    settings: Mutex<EqSettings>,
    version: AtomicU64,
}

impl SharedEq {
    pub fn new(settings: EqSettings) -> Arc<Self> {
        Arc::new(Self {
            settings: Mutex::new(settings),
            version: AtomicU64::new(1),
        })
    }

    /// Returns false when the settings were already `s`.
    pub fn set(&self, s: &EqSettings) -> bool {
        let mut cur = self.settings.lock().expect("eq lock");
        if *cur == *s {
            return false;
        }
        *cur = *s;
        self.version.fetch_add(1, Ordering::Release);
        true
    }
}

/// What [`EqStage`] saves: the whole processor (no heap data) and the settings it has seen.
struct Saved {
    eq: Equalizer,
    seen: u64,
}

pub struct EqStage {
    shared: Arc<SharedEq>,
    eq: Equalizer,
    seen: u64,
}

impl EqStage {
    pub fn new(shared: Arc<SharedEq>) -> Self {
        Self {
            shared,
            eq: Equalizer::new(44_100, 2),
            seen: 0,
        }
    }

    fn sync(&mut self) {
        let v = self.shared.version.load(Ordering::Acquire);
        if v != self.seen
            && let Ok(s) = self.shared.settings.try_lock()
        {
            self.eq.set_settings(&s);
            self.seen = v;
        }
    }
}

impl Stage for EqStage {
    fn configure(&mut self, sample_rate: u32, channels: usize) {
        self.eq
            .set_format(sample_rate, channels.clamp(1, nulamp_eq::MAX_CHANNELS));
        self.seen = 0;
    }

    fn process(&mut self, samples: &mut [f32]) {
        self.sync();
        if self.eq.channels() > 0 {
            self.eq.process(samples);
        }
    }

    fn reset(&mut self) {
        self.eq.reset();
    }

    fn save(&self) -> Option<StageState> {
        Some(Box::new(Saved {
            eq: self.eq.clone(),
            seen: self.seen,
        }))
    }

    /// Back to a saved state; settings changed since then glide in on the next block (EQ2).
    fn restore(&mut self, state: StageState) {
        if let Ok(s) = state.downcast::<Saved>() {
            let Saved { eq, seen } = *s;
            self.eq = eq;
            self.seen = seen;
        }
    }
}
