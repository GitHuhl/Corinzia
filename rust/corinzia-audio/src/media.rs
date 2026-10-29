//! Sound files for audio tracks, on the main thread: decoded (WAV, FLAC,
//! MP3, Ogg Vorbis), reduced to mono or stereo, resampled to the engine's
//! rate, with peaks for drawing the waveform; kept by path, so a snapshot
//! rebuild never decodes a file again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Frames per waveform peak.
pub const PEAK_FRAMES: usize = 256;

/// A decoded file at the engine's rate.
#[derive(Debug)]
pub struct AudioData {
    /// One channel (mono) or two.
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: f64,
    /// (min, max) of the channels together, per [`PEAK_FRAMES`] frames.
    pub peaks: Vec<(f32, f32)>,
}

impl AudioData {
    pub fn frames(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.sample_rate
    }
    /// From samples already at `sample_rate` (tests, generated sound).
    pub fn from_channels(channels: Vec<Vec<f32>>, sample_rate: f64) -> AudioData {
        let peaks = peaks(&channels);
        AudioData { channels, sample_rate, peaks }
    }
}

fn peaks(channels: &[Vec<f32>]) -> Vec<(f32, f32)> {
    let frames = channels.first().map(|c| c.len()).unwrap_or(0);
    (0..frames.div_ceil(PEAK_FRAMES))
        .map(|k| {
            let r = k * PEAK_FRAMES..((k + 1) * PEAK_FRAMES).min(frames);
            channels.iter().flat_map(|c| c[r.clone()].iter()).fold((0.0f32, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)))
        })
        .collect()
}

/// Decodes a file to (channels, its sample rate): the first two channels.
pub fn decode(path: &Path) -> Result<(Vec<Vec<f32>>, u32), String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let name = path.display().to_string();
    let file = std::fs::File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("{name}: not a sound file Corinzia reads (WAV, FLAC, MP3, Ogg Vorbis): {e}"))?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or(format!("{name}: no audio in it"))?;
    let (id, params) = (track.id, track.codec_params.clone());
    let rate = params.sample_rate.ok_or(format!("{name}: unknown sample rate"))?;
    let mut decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(|e| format!("{name}: {e}"))?;
    let mut out: Vec<Vec<f32>> = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("{name}: {e}")),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(Error::DecodeError(_)) => continue, // a damaged packet: skipped
            Err(e) => return Err(format!("{name}: {e}")),
        };
        let spec = *decoded.spec();
        let n = spec.channels.count().max(1);
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * n {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        let keep = n.min(2);
        if out.len() < keep {
            out.resize(keep, Vec::new());
        }
        for frame in b.samples().chunks(n) {
            for (c, ch) in out.iter_mut().enumerate().take(keep) {
                ch.push(frame[c]);
            }
        }
    }
    if out.is_empty() || out[0].is_empty() {
        return Err(format!("{name}: no audio in it"));
    }
    Ok((out, rate))
}

/// Resamples every channel from `from` Hz to `to` Hz.
pub fn resample(channels: Vec<Vec<f32>>, from: f64, to: f64) -> Result<Vec<Vec<f32>>, String> {
    use rubato::{FftFixedIn, Resampler};
    if (from - to).abs() < 0.5 || channels.is_empty() {
        return Ok(channels);
    }
    let frames = channels[0].len();
    let want = (frames as f64 * to / from).round() as usize;
    let mut r = FftFixedIn::<f32>::new(from.round() as usize, to.round() as usize, 1024, 2, channels.len()).map_err(|e| e.to_string())?;
    let delay = r.output_delay();
    let mut out: Vec<Vec<f32>> = vec![Vec::with_capacity(want + delay + 2048); channels.len()];
    let mut at = 0;
    let mut push = |o: Vec<Vec<f32>>, out: &mut Vec<Vec<f32>>| {
        for (d, s) in out.iter_mut().zip(o) {
            d.extend_from_slice(&s);
        }
    };
    while frames - at >= r.input_frames_next() {
        let n = r.input_frames_next();
        let chunk: Vec<&[f32]> = channels.iter().map(|c| &c[at..at + n]).collect();
        push(r.process(&chunk, None).map_err(|e| e.to_string())?, &mut out);
        at += n;
    }
    let rest: Vec<&[f32]> = channels.iter().map(|c| &c[at..]).collect();
    push(r.process_partial(Some(&rest), None).map_err(|e| e.to_string())?, &mut out);
    // Flush the filter until the whole file is out.
    while out[0].len() < want + delay {
        push(r.process_partial::<&[f32]>(None, None).map_err(|e| e.to_string())?, &mut out);
    }
    for c in out.iter_mut() {
        c.drain(..delay.min(c.len()));
        c.truncate(want);
    }
    Ok(out)
}

/// Decoded files by path, for one engine rate.
pub struct Media {
    pub sample_rate: f64,
    /// Where relative paths start (the project file's folder).
    pub base: Option<PathBuf>,
    files: HashMap<PathBuf, Result<Arc<AudioData>, String>>,
}

impl Media {
    pub fn new(sample_rate: f64) -> Media {
        Media { sample_rate, base: None, files: HashMap::new() }
    }

    pub fn resolve(&self, file: &str) -> PathBuf {
        let p = PathBuf::from(file);
        match (&self.base, p.is_relative()) {
            (Some(b), true) => b.join(p),
            _ => p,
        }
    }

    /// The file decoded at the engine's rate (decoded once; a failure is
    /// remembered too, until [`Media::forget`]).
    pub fn get(&mut self, file: &str) -> Result<Arc<AudioData>, String> {
        let path = self.resolve(file);
        let rate = self.sample_rate;
        self.files
            .entry(path.clone())
            .or_insert_with(|| {
                let (channels, from) = decode(&path)?;
                Ok(Arc::new(AudioData::from_channels(resample(channels, from as f64, rate)?, rate)))
            })
            .clone()
    }

    /// Already decoded (no work): for drawing.
    pub fn loaded(&self, file: &str) -> Option<Arc<AudioData>> {
        self.files.get(&self.resolve(file)).and_then(|r| r.as_ref().ok().cloned())
    }

    /// Puts sound in under a name (tests, generated audio).
    pub fn insert(&mut self, file: &str, data: AudioData) {
        let path = self.resolve(file);
        self.files.insert(path, Ok(Arc::new(data)));
    }

    /// Drops files no clip uses, and failures (to try them again).
    pub fn retain(&mut self, used: &[String]) {
        let keep: Vec<PathBuf> = used.iter().map(|f| self.resolve(f)).collect();
        self.files.retain(|p, r| r.is_ok() && keep.contains(p));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, rate: u32, samples: &[i16]) {
        let mut b = Vec::new();
        let data = (samples.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&1u16.to_le_bytes()); // mono
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn a_wav_file_is_decoded_and_resampled() {
        let dir = std::env::temp_dir().join(format!("corinzia-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // One second of 441 Hz at 44.1 kHz.
        let tone: Vec<i16> = (0..44100).map(|i| ((i as f64 * 441.0 / 44100.0 * std::f64::consts::TAU).sin() * 16000.0) as i16).collect();
        write_wav(&dir.join("tone.wav"), 44100, &tone);
        let mut m = Media::new(48000.0);
        m.base = Some(dir.clone());
        let a = m.get("tone.wav").unwrap();
        assert_eq!(a.channels.len(), 1);
        assert_eq!(a.frames(), 48000);
        // Still 441 Hz: a period is 48000 / 441 frames; compare a stretch
        // away from the ends with the ideal sine.
        let err = (10000..11000).map(|i| (a.channels[0][i] as f64 - (i as f64 * 441.0 / 48000.0 * std::f64::consts::TAU).sin() * 16000.0 / 32768.0).abs()).fold(0.0, f64::max);
        assert!(err < 0.01, "{err}");
        assert!(a.peaks.iter().any(|p| p.1 > 0.45));
        assert!(Arc::ptr_eq(&a, &m.get("tone.wav").unwrap()), "decoded once");
        assert!(m.get("missing.wav").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
