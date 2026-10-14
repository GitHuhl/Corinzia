//! Live output through a real (private, headless) PipeWire daemon:
//!   CORINZIA_PW=1 CORINZIA_PW_SESSION=1 tests/run_with_pipewire.sh cargo test -p corinzia-audio --test pipewire
//! (needs `wireplumber` on PATH for the session).
//! Skipped unless CORINZIA_PW is set.

#![cfg(feature = "pipewire")]

use corinzia_audio::engine::{self, Command};
use corinzia_audio::pool::PluginPool;
use corinzia_audio::{backend, snapshot};
use corinzia_model::Project;
use std::process::Command as Proc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

fn pw(args: &[&str]) -> bool {
    Proc::new(args[0]).args(&args[1..]).status().map(|s| s.success()).unwrap_or(false)
}

#[test]
fn plays_through_pipewire() {
    if std::env::var("CORINZIA_PW").is_err() {
        eprintln!("CORINZIA_PW not set; skipping");
        return;
    }
    // A sink to play into; the session manager (WirePlumber) links the
    // stream to it, as on a desktop.
    assert!(pw(&["pw-cli", "create-node", "adapter", "{ factory.name=support.null-audio-sink node.name=test-sink media.class=Audio/Sink object.linger=1 audio.position=[FL FR] }"]));
    let (mut ui, eng) = engine::new(48000.0, 1024);
    let out = backend::start(eng, 48000, 1024).expect("PipeWire stream");
    let p = Project::default();
    let mut pool = PluginPool::new(48000.0, 1024);
    ui.swap(snapshot::build(&p, 48000.0, 1, &[], |t| pool.plugin_for(&p, t)));
    ui.send(Command::Play).ok();
    let deadline = Instant::now() + Duration::from_secs(5);
    while ui.shared.blocks.load(Ordering::Relaxed) < 20 {
        if Instant::now() > deadline {
            let links = Proc::new("pw-link").arg("-l").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
            panic!("PipeWire never called us: {} blocks; links:\n{links}", ui.shared.blocks.load(Ordering::Relaxed));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(ui.shared.generation.load(Ordering::Acquire), 1, "the plan reached the realtime thread");
    drop(out); // stops the loop and joins the thread
    ui.collect();
}
