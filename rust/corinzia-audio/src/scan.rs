//! Plugin scanning that a bad plugin cannot crash: each bundle is opened in
//! a child process (`corinzia --scan-plugin <bundle>`, which prints its
//! plugins as JSON), with a time limit. Results are cached by the bundle's
//! path, size and modification time, so a later start only opens bundles
//! that are new or changed; a bundle that crashed stays marked until it
//! changes.

use crate::host::{find_bundles, Library, PluginInfo};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What the child process prints for one bundle.
pub fn bundle_json(path: &Path) -> Result<String, String> {
    let lib = Library::load(path)?;
    serde_json::to_string(&lib.plugins()).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    size: u64,
    modified: u64, // seconds since the epoch
    /// The bundle's plugins, or why it could not be scanned.
    result: Result<Vec<PluginInfo>, String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    entries: Vec<Entry>,
}

fn stamp(path: &Path) -> (u64, u64) {
    // A .vst3 bundle is a directory: its module inside changes, not the folder.
    let meta = |p: &Path| std::fs::metadata(p).ok();
    let mut size = 0;
    let mut modified = 0;
    let mut visit = |p: &Path| {
        if let Some(m) = meta(p) {
            size += m.len();
            modified = modified.max(m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()));
        }
    };
    if path.is_dir() {
        fn walk(d: &Path, depth: usize, f: &mut dyn FnMut(&Path)) {
            f(d);
            let Ok(rd) = std::fs::read_dir(d) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() && depth < 6 {
                    walk(&p, depth + 1, f);
                } else {
                    f(&p);
                }
            }
        }
        walk(path, 0, &mut visit);
    } else {
        visit(path);
    }
    (size, modified)
}

/// Runs `exe --scan-plugin <bundle>` and reads its answer; a crash, an
/// error exit or more than `limit` counts against the bundle.
pub fn run_child(exe: &Path, bundle: &Path, limit: Duration) -> Result<Vec<PluginInfo>, String> {
    use std::io::Read;
    let mut child = std::process::Command::new(exe)
        .arg("--scan-plugin")
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: could not start the scanner ({e})", bundle.display()))?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{}: gave no answer in {} s (skipped)", bundle.display(), limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("{}: {e}", bundle.display())),
        }
    };
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    if !status.success() {
        let why = err.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string());
        return Err(match (status.code(), why) {
            (Some(_), Some(w)) => w,
            (None, _) => format!("{}: crashed while being scanned (skipped)", bundle.display()),
            (Some(c), None) => format!("{}: scanner exited with {c}", bundle.display()),
        });
    }
    serde_json::from_str(out.trim()).map_err(|e| format!("{}: unreadable scan ({e})", bundle.display()))
}

/// The plugins under `paths`, the built-in instrument first. `probe` scans
/// one bundle (normally [`run_child`]); `cache` is the cache file, if any.
pub fn scan_with(paths: &[PathBuf], cache: Option<&Path>, mut probe: impl FnMut(&Path) -> Result<Vec<PluginInfo>, String>) -> (Vec<PluginInfo>, Vec<String>) {
    let old: Cache = cache.and_then(|c| std::fs::read_to_string(c).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut infos = Library::builtin().plugins();
    let mut errors = Vec::new();
    let mut fresh = Cache::default();
    for b in find_bundles(paths) {
        let (size, modified) = stamp(&b);
        let entry = match old.entries.iter().find(|e| e.path == b && e.size == size && e.modified == modified) {
            Some(e) => e.clone(),
            None => Entry { path: b.clone(), size, modified, result: probe(&b) },
        };
        match &entry.result {
            Ok(list) => infos.extend(list.iter().cloned()),
            Err(e) => errors.push(e.clone()),
        }
        fresh.entries.push(entry);
    }
    if let Some(c) = cache {
        if let Some(d) = c.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string(&fresh) {
            let _ = std::fs::write(c, t);
        }
    }
    (infos, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_bundles_are_not_probed_again_until_they_change() {
        let dir = std::env::temp_dir().join(format!("corinzia-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        let (good, bad) = (dir.join("plugins/good.clap"), dir.join("plugins/bad.clap"));
        std::fs::write(&good, b"x").unwrap();
        std::fs::write(&bad, b"y").unwrap();
        let cache = dir.join("cache.json");
        let mut probed = Vec::new();
        let fake = |p: &Path, probed: &mut Vec<PathBuf>| {
            probed.push(p.to_path_buf());
            if p.ends_with("bad.clap") {
                Err("bad.clap: crashed while being scanned (skipped)".to_string())
            } else {
                let mut info = Library::builtin().plugins().remove(0);
                info.name = "Good".into();
                info.bundle = p.to_path_buf();
                Ok(vec![info])
            }
        };
        let paths = vec![dir.join("plugins")];
        let (infos, errors) = scan_with(&paths, Some(&cache), |p| fake(p, &mut probed));
        let builtin = Library::builtin().plugins().len();
        assert_eq!((infos.len(), errors.len(), probed.len()), (builtin + 1, 1, 2));
        assert_eq!(infos[builtin].name, "Good");
        probed.clear();
        let (infos, errors) = scan_with(&paths, Some(&cache), |p| fake(p, &mut probed));
        assert_eq!((infos.len(), errors.len(), probed.len()), (builtin + 1, 1, 0), "all from the cache, the crash remembered");
        std::fs::write(&bad, b"changed").unwrap();
        let _ = scan_with(&paths, Some(&cache), |p| fake(p, &mut probed));
        assert_eq!(probed, std::slice::from_ref(&bad), "only the changed bundle");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_child_that_crashes_or_hangs_is_reported() {
        // `sh` stands in for the scanner: it ignores the arguments' meaning.
        let sh = Path::new("/bin/sh");
        if !sh.exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("corinzia-child-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // "sh --scan-plugin script" fails: sh has no such option.
        let e = run_child(sh, &dir.join("x.clap"), Duration::from_secs(5)).unwrap_err();
        assert!(!e.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
