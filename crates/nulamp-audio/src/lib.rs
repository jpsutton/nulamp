//! Audio processing chain and output stage (07 §4–§6).

pub mod chain;
pub mod engine;
pub mod eq_stage;
pub mod mixer;
pub mod output;
pub mod player;
pub mod resample;

pub use engine::{AudioEngine, Control, EngineStatus, Event, OutputPrefs};
pub use player::{Player, PlayerEvent};
