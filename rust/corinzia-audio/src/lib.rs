//! Corinzia's audio engine in Rust.
//!
//! * [`host`]: CLAP hosting (bundles on disk and the built-in Corinzia Sine);
//! * [`pool`]: plugin instances per track, on the main thread;
//! * [`snapshot`]: the immutable, sample-positioned render plan built from
//!   the project through the performance compiler;
//! * [`engine`]: the realtime core and its UI handle (wait-free queues);
//! * [`backend`]: live output: PipeWire on Linux (feature `pipewire`),
//!   WASAPI on Windows (feature `wasapi`).

pub mod engine;
pub mod host;
pub mod pool;
pub mod snapshot;
#[cfg(all(feature = "pipewire", target_os = "linux"))]
pub mod backend;
#[cfg(all(feature = "wasapi", windows))]
#[path = "backend_wasapi.rs"]
pub mod backend;
