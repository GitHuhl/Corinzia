//! Audio export: a movement rendered offline to a WAV file, by its own
//! engine and plugin instances (the live ones go on playing), a slice at a
//! time so the caller can stay responsive and show progress.

use crate::engine::{self, Engine, EngineHandle};
use crate::pool::PluginPool;
use crate::snapshot;
use corinzia_model::Project;
use std::path::{Path, PathBuf};

/// How long the sound may ring on after the music (reverbs, releases).
pub const TAIL_SECONDS: f64 = 3.0;

pub struct Bounce {
    _handle: EngineHandle,
    engine: Engine,
    /// Kept alive while the engine plays its plugins.
    _pool: PluginPool,
    left: Vec<f32>,
    right: Vec<f32>,
    total: usize,
    pub sample_rate: f64,
    pub path: PathBuf,
    /// Plugins that could not be loaded (played by the built-in sound
    /// instead) and clips whose files could not be read.
    pub warnings: Vec<String>,
}

/// What the file holds, for the status line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub seconds: f64,
    /// The highest sample, in dBFS (before clipping to the file).
    pub peak_db: f64,
    /// Samples over full scale (clipped in the file).
    pub clipped: usize,
}

impl Bounce {
    /// Prepares the render of `movement` (an index), plugins loaded from
    /// the project's states; `folder`: where clips' relative paths start.
    pub fn new(project: &Project, movement: usize, sample_rate: f64, block: u32, folder: Option<&Path>, path: PathBuf) -> Result<Bounce, String> {
        if project.movements.get(movement).is_none() {
            return Err("No movement to export".into());
        }
        let mut pool = PluginPool::new(sample_rate, block);
        pool.media.base = folder.map(Into::into);
        let snap = snapshot::build_from_pool(project, sample_rate, 1, &[movement], &mut pool);
        let warnings = pool.errors.iter().chain(&snap.clip_errors).cloned().collect();
        let total = snap.end().max(0) as usize + (TAIL_SECONDS * sample_rate) as usize;
        let (mut handle, engine) = engine::new(sample_rate, block);
        handle.swap(snap);
        handle.play();
        Ok(Bounce { _handle: handle, engine, _pool: pool, left: Vec::with_capacity(total), right: Vec::with_capacity(total), total, sample_rate, path, warnings })
    }

    /// Renders up to `seconds` more; true when all is rendered.
    pub fn step(&mut self, seconds: f64) -> bool {
        let n = ((seconds * self.sample_rate) as usize).min(self.total - self.left.len());
        let (l, r) = self.engine.render_offline(n);
        self.left.extend_from_slice(&l);
        self.right.extend_from_slice(&r);
        self.left.len() >= self.total
    }

    /// How far along, 0..1.
    pub fn progress(&self) -> f32 {
        self.left.len() as f32 / self.total.max(1) as f32
    }

    /// Writes the file (24-bit PCM, stereo).
    pub fn finish(self) -> Result<Summary, String> {
        let peak = self.left.iter().chain(&self.right).fold(0.0f32, |m, v| m.max(v.abs()));
        let clipped = self.left.iter().chain(&self.right).filter(|v| v.abs() > 1.0).count();
        let bytes = wav_24(&self.left, &self.right, self.sample_rate as u32);
        std::fs::write(&self.path, bytes).map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(Summary { seconds: self.left.len() as f64 / self.sample_rate, peak_db: 20.0 * (peak.max(1e-9) as f64).log10(), clipped })
    }
}

/// A stereo WAV file of 24-bit samples (clipped to full scale).
pub fn wav_24(left: &[f32], right: &[f32], rate: u32) -> Vec<u8> {
    let frames = left.len().min(right.len());
    let data = (frames * 6) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 6).to_le_bytes());
    b.extend_from_slice(&6u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    let sample = |v: f32| ((v.clamp(-1.0, 1.0) as f64 * 8_388_607.0).round() as i32).to_le_bytes();
    for i in 0..frames {
        b.extend_from_slice(&sample(left[i])[..3]);
        b.extend_from_slice(&sample(right[i])[..3]);
    }
    b
}
