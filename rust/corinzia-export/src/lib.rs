//! Corinzia export: Standard MIDI Files and MusicXML.
//!
//! The C++ application serializes one movement as an export document (JSON,
//! see `doc.rs`) and calls the C functions below. Nothing here touches the
//! live project: the document is a snapshot, so exports can run off the UI
//! thread.

pub mod doc;
pub mod frac;
pub mod midi;
pub mod musicxml;

use std::ffi::{c_char, CStr};
use std::panic::catch_unwind;

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

fn report(err: *mut c_char, err_len: usize, msg: &str) {
    if err.is_null() || err_len == 0 {
        return;
    }
    let bytes = msg.as_bytes();
    let n = bytes.len().min(err_len - 1);
    // SAFETY: the caller provides a writable buffer of `err_len` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), err as *mut u8, n);
        *err.add(n) = 0;
    }
}

fn run(
    json: *const u8,
    len: usize,
    path: *const c_char,
    err: *mut c_char,
    err_len: usize,
    f: fn(&[u8]) -> Result<Vec<u8>, String>,
) -> i32 {
    if json.is_null() || path.is_null() {
        report(err, err_len, "null argument");
        return 1;
    }
    // SAFETY: the caller passes `len` readable bytes and a NUL-terminated path.
    let (input, path) = unsafe { (std::slice::from_raw_parts(json, len), CStr::from_ptr(path)) };
    let result = catch_unwind(|| f(input)).unwrap_or_else(|_| Err("internal error (panic) in export".into()));
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            report(err, err_len, &e);
            return 1;
        }
    };
    let path = match path.to_str() {
        Ok(p) => p,
        Err(_) => {
            report(err, err_len, "path is not UTF-8");
            return 1;
        }
    };
    if let Err(e) = std::fs::write(path, bytes) {
        report(err, err_len, &format!("cannot write {path}: {e}"));
        return 1;
    }
    0
}

/// Writes a Standard MIDI File. Returns 0 on success, else non-zero with a
/// message in `err` (NUL-terminated, truncated to `err_len`).
#[no_mangle]
pub extern "C" fn cz_export_midi(json: *const u8, len: usize, path: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    run(json, len, path, err, err_len, midi_bytes)
}

/// Writes a MusicXML 4.0 (partwise, uncompressed) file. Same conventions.
#[no_mangle]
pub extern "C" fn cz_export_musicxml(json: *const u8, len: usize, path: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    run(json, len, path, err, err_len, |j| musicxml_string(j).map(String::into_bytes))
}
