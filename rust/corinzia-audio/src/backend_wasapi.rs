//! Live output on Windows through WASAPI (via cpal): the device's default
//! output, 32-bit float stereo, the realtime callback rendering the engine.
//! The stream lives on its own thread; dropping [`Output`] stops it.

use crate::engine::Engine;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::thread::JoinHandle;

pub struct Output {
    quit: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
    pub sample_rate: u32,
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.quit.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts the stream; `engine` moves to the audio callback. Fails when no
/// output device can play `sample_rate` stereo (the engine is dropped).
pub fn start(engine: Engine, sample_rate: u32, max_frames: u32) -> Result<Output, String> {
    let (quit_tx, quit_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new()
        .name("corinzia-wasapi".into())
        .spawn(move || match open(engine, sample_rate, max_frames) {
            Ok(stream) => {
                let _ = ready_tx.send(Ok(()));
                let _ = quit_rx.recv();
                drop(stream);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        })
        .map_err(|e| e.to_string())?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Output { quit: quit_tx, thread: Some(thread), sample_rate }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("the audio thread ended".into()),
    }
}

fn open(mut engine: Engine, rate: u32, max_frames: u32) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let channels = device.default_output_config().map(|c| c.channels()).unwrap_or(2).max(2);
    let config = cpal::StreamConfig { channels, sample_rate: cpal::SampleRate(rate), buffer_size: cpal::BufferSize::Default };
    let block = max_frames as usize;
    let (mut left, mut right) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut marked = false;
    let ch = channels as usize;
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if !marked {
                    crate::host::mark_audio_thread();
                    marked = true;
                }
                let frames = data.len() / ch;
                let mut at = 0;
                while at < frames {
                    let n = (frames - at).min(block);
                    engine.process(&mut left[..n], &mut right[..n]);
                    for i in 0..n {
                        let frame = &mut data[(at + i) * ch..(at + i + 1) * ch];
                        frame[0] = left[i];
                        frame[1] = right[i];
                        for extra in &mut frame[2..] {
                            *extra = 0.0;
                        }
                    }
                    at += n;
                }
            },
            |e| eprintln!("corinzia: audio output: {e}"),
            None,
        )
        .map_err(|e| format!("audio output at {rate} Hz: {e}"))?;
    stream.play().map_err(|e| format!("audio output: {e}"))?;
    Ok(stream)
}
