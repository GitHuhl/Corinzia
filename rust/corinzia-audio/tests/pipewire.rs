//! Live output through a real (private, headless) PipeWire daemon:
//!   CORINZIA_PW=1 CORINZIA_PW_SESSION=1 scripts/run_with_pipewire.sh cargo test -p corinzia-audio --test pipewire
//! (needs `wireplumber` on PATH for the session).
//! Skipped unless CORINZIA_PW is set.

#![cfg(all(feature = "pipewire", target_os = "linux"))]

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
    let out = backend::start(eng, 48000, 1024, "").expect("PipeWire stream");
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

    // Playing advances one cycle at a time: each callback renders what
    // PipeWire asked for (a quantum), not the whole buffer it maps (often
    // many quanta), which would bunch many blocks of plugin work into one
    // deadline and make PipeWire queue the rest. A silent minute.
    ui.swap(snapshot::build_tones_from_pool(&p, 48000.0, 2, 99, 60.0, 60.0, 0, &[], &mut pool));
    ui.seek(0);
    ui.play();
    std::thread::sleep(Duration::from_millis(300));
    let (t0, p0) = (Instant::now(), ui.position());
    let (mut last, mut biggest) = (p0, 0);
    while t0.elapsed() < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(3));
        let now = ui.position();
        biggest = biggest.max(now - last);
        last = now;
    }
    let (seconds, played) = (t0.elapsed().as_secs_f64(), (last - p0) as f64 / 48000.0);
    assert!(ui.is_playing(), "still playing");
    assert!((played / seconds - 1.0).abs() < 0.15, "{played:.2} s of audio in {seconds:.2} s");
    assert!(biggest <= 2 * 1024, "the playhead jumped {biggest} frames at once (a quantum is 1024)");
    drop(out); // stops the loop and joins the thread
    ui.collect();
}
