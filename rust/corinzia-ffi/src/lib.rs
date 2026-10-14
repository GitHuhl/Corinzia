//! The C interface (include/corinzia/export/corinzia_export.h). One static
//! library for all Rust code the C++ application uses: two Rust static
//! libraries in one binary would each carry the Rust runtime.
//!
//! Conventions: input is a JSON document; functions return 0 on success,
//! else non-zero with a NUL-terminated message in `err` (`err_len` bytes).
//! Strings returned through `out` are freed with `cz_free_string`.

use corinzia_score::score::Score;
use serde::Deserialize;
use std::ffi::{c_char, CStr, CString};
use std::panic::catch_unwind;

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

/// Runs `f` on the input bytes with panics turned into errors.
fn guarded<T>(json: *const u8, len: usize, err: *mut c_char, err_len: usize, f: impl Fn(&[u8]) -> Result<T, String> + std::panic::RefUnwindSafe) -> Option<T> {
    if json.is_null() {
        report(err, err_len, "null argument");
        return None;
    }
    // SAFETY: the caller passes `len` readable bytes.
    let input = unsafe { std::slice::from_raw_parts(json, len) };
    match catch_unwind(|| f(input)).unwrap_or_else(|_| Err("internal error (panic)".into())) {
        Ok(v) => Some(v),
        Err(e) => {
            report(err, err_len, &e);
            None
        }
    }
}

fn write_file(path: *const c_char, bytes: &[u8], err: *mut c_char, err_len: usize) -> i32 {
    if path.is_null() {
        report(err, err_len, "null path");
        return 1;
    }
    // SAFETY: NUL-terminated by contract.
    let Ok(path) = unsafe { CStr::from_ptr(path) }.to_str() else {
        report(err, err_len, "path is not UTF-8");
        return 1;
    };
    match std::fs::write(path, bytes) {
        Ok(()) => 0,
        Err(e) => {
            report(err, err_len, &format!("cannot write {path}: {e}"));
            1
        }
    }
}

/// Writes a Standard MIDI File from an export document.
#[no_mangle]
pub extern "C" fn cz_export_midi(json: *const u8, len: usize, path: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    match guarded(json, len, err, err_len, corinzia_export::midi_bytes) {
        Some(b) => write_file(path, &b, err, err_len),
        None => 1,
    }
}

/// Writes MusicXML 4.0 (partwise) from an export document.
#[no_mangle]
pub extern "C" fn cz_export_musicxml(json: *const u8, len: usize, path: *const c_char, err: *mut c_char, err_len: usize) -> i32 {
    match guarded(json, len, err, err_len, corinzia_export::musicxml_string) {
        Some(s) => write_file(path, s.as_bytes(), err, err_len),
        None => 1,
    }
}

#[derive(Deserialize)]
struct LayoutDoc {
    version: u32,
    score: Score,
    /// "line" (default): one system; "pages": A4 pages.
    #[serde(default)]
    mode: String,
}

/// Engraves the score of a document (`{"version":1,"score":{...},"mode":…}`)
/// with the native engraver: one system, or A4 pages (`"mode":"pages"`, SVGs
/// separated by form feeds). The result goes to `*out`.
#[no_mangle]
pub extern "C" fn cz_layout_svg(json: *const u8, len: usize, out: *mut *mut c_char, err: *mut c_char, err_len: usize) -> i32 {
    if out.is_null() {
        report(err, err_len, "null out");
        return 1;
    }
    let svg = guarded(json, len, err, err_len, |j| {
        let doc: LayoutDoc = serde_json::from_slice(j).map_err(|e| format!("layout document: {e}"))?;
        if doc.version != 1 {
            return Err(format!("layout document version {} not supported", doc.version));
        }
        let opt = corinzia_layout::Options::default();
        if doc.mode == "pages" {
            let pages = corinzia_layout::layout_pages(&doc.score, &opt, &corinzia_layout::PageSpec::default());
            Ok(pages.iter().map(corinzia_layout::svg::page_to_svg).collect::<Vec<_>>().join("\u{c}"))
        } else {
            Ok(corinzia_layout::svg::to_svg(&corinzia_layout::layout_line(&doc.score, &opt)))
        }
    });
    match svg.and_then(|s| CString::new(s).ok()) {
        Some(c) => {
            // SAFETY: `out` checked non-null above.
            unsafe { *out = c.into_raw() };
            0
        }
        None => 1,
    }
}

/// Frees a string returned by this library.
#[no_mangle]
pub extern "C" fn cz_free_string(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: created by CString::into_raw in this library.
        drop(unsafe { CString::from_raw(s) });
    }
}
