//! The equalizer as a processing stage (07 §4 stage 3).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nulamp_eq::{EqSettings, Equalizer};

use crate::chain::Stage;

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

    pub fn set(&self, s: &EqSettings) {
        *self.settings.lock().expect("eq lock") = *s;
        self.version.fetch_add(1, Ordering::Release);
    }
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
}
