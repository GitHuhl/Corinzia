//! Corinzia export: Standard MIDI Files and MusicXML.
//!
//! Both take an export document (`doc.rs`), a snapshot of one movement, so
//! exports can run off the UI thread.

pub mod doc;
pub use corinzia_score::frac;
pub mod midi;
pub mod musicxml;
