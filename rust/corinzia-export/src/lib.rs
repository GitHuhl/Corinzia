//! Corinzia export: Standard MIDI Files and MusicXML.
//!
//! The C++ application serializes one movement as an export document (JSON,
//! see `doc.rs`); the C interface is in the corinzia-ffi crate. Nothing here
//! touches the live project: the document is a snapshot, so exports can run
//! off the UI thread.

pub mod doc;
pub use corinzia_score::frac;
pub mod midi;
pub mod musicxml;

pub fn parse(json: &[u8]) -> Result<doc::Document, String> {
    let d: doc::Document = serde_json::from_slice(json).map_err(|e| format!("export document: {e}"))?;
    if d.version != 1 {
        return Err(format!("export document version {} not supported", d.version));
    }
    Ok(d)
}

pub fn midi_bytes(json: &[u8]) -> Result<Vec<u8>, String> {
    midi::write(&parse(json)?)
}

pub fn musicxml_string(json: &[u8]) -> Result<String, String> {
    musicxml::write(&parse(json)?)
}
