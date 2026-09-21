//! Game-facing names for the shared SDK telemetry emitter.
//!
//! Normal builds keep only the unconditional debug console; profiling writes
//! are enabled by forwarding this game's `emulator-telemetry` feature.
#![allow(unused_imports)]

pub use psx_telemetry::emit::{
    console, counter, cycles, debug_log, frame_begin, stage_begin, stage_end, task_begin, task_end,
};
pub use psx_telemetry::stage;
