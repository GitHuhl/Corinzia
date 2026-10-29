//! Corinzia's audio engine in Rust.
//!
//! * [`host`]: plugin hosting (CLAP bundles, VST3 bundles through [`vst3`],
//!   and the built-in Corinzia Sine);
//! * [`gui`]: plugin editor windows and the run loop their GUIs need;
//! * [`pool`]: plugin instances per track, on the main thread;
//! * [`media`]: sound files for audio tracks, decoded and resampled;
//! * [`snapshot`]: the immutable, sample-positioned render plan built from
//!   the project through the performance compiler;
//! * [`engine`]: the realtime core and its UI handle (wait-free queues);
//! * [`workers`]: the threads instruments render on in parallel;
//! * [`backend`]: live output: PipeWire on Linux (feature `pipewire`),
//!   WASAPI on Windows (feature `wasapi`).

pub mod engine;
pub mod gui;
pub mod host;
pub mod media;
pub mod pool;
pub mod scan;
pub mod snapshot;
pub mod vst3;
pub mod workers;
#[cfg(all(feature = "pipewire", target_os = "linux"))]
pub mod backend;
#[cfg(all(feature = "wasapi", windows))]
#[path = "backend_wasapi.rs"]
pub mod backend;
