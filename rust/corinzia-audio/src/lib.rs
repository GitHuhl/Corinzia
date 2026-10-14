//! Corinzia's audio engine in Rust.
//!
//! * [`host`]: CLAP hosting (bundles on disk and the built-in Corinzia Sine);
//! * [`pool`]: plugin instances per track, on the main thread;
//! * [`snapshot`]: the immutable, sample-positioned render plan built from
//!   the project through the performance compiler;
//! * [`engine`]: the realtime core and its UI handle (wait-free queues);
//! * [`backend`]: PipeWire output (feature `pipewire`) and offline rendering.

pub mod engine;
pub mod host;
pub mod pool;
pub mod snapshot;
#[cfg(feature = "pipewire")]
pub mod backend;
