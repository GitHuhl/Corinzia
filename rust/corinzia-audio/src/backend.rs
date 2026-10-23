//! Live output through PipeWire: a playback stream whose realtime process
//! callback renders the engine (32-bit float, stereo, interleaved). The
//! PipeWire main loop runs on its own thread; dropping [`Output`] stops it.

use crate::engine::Engine;
use pipewire as pw;
use pw::spa;
use spa::pod::Pod;
use std::thread::JoinHandle;

pub struct Output {
    quit: pw::channel::Sender<()>,
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

/// Output devices (PipeWire audio sinks): (node name, description).
pub fn devices() -> Vec<(String, String)> {
    use std::cell::RefCell;
    use std::rc::Rc;
    pw::init();
    let Ok(mainloop) = pw::main_loop::MainLoop::new(None) else { return vec![] };
    let Ok(context) = pw::context::Context::new(&mainloop) else { return vec![] };
    let Ok(core) = context.connect(None) else { return vec![] };
    let Ok(registry) = core.get_registry() else { return vec![] };
    let found: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    let f = found.clone();
    let _reg = registry
        .add_listener_local()
        .global(move |g| {
            let Some(props) = g.props else { return };
            if g.type_ == pw::types::ObjectType::Node && props.get("media.class") == Some("Audio/Sink") {
                if let Some(name) = props.get("node.name") {
                    let label = props.get("node.description").or(props.get("node.nick")).unwrap_or(name);
                    f.borrow_mut().push((name.to_string(), label.to_string()));
                }
            }
        })
        .register();
    let Ok(pending) = core.sync(0) else { return vec![] };
    let ml = mainloop.clone();
    let _core = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                ml.quit();
            }
        })
        .register();
    mainloop.run();
    let v = found.borrow().clone();
    v
}

/// Starts the stream; `engine` moves to PipeWire's realtime thread.
/// `device`: a sink's node name, or empty for the default. Fails when no
/// PipeWire daemon is reachable (the engine is dropped).
pub fn start(engine: Engine, sample_rate: u32, max_frames: u32, device: &str) -> Result<Output, String> {
    let device = device.to_string();
    let (quit_tx, quit_rx) = pw::channel::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new()
        .name("corinzia-pipewire".into())
        .spawn(move || {
            if let Err(e) = run(engine, sample_rate, max_frames, &device, quit_rx, &ready_tx) {
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
        Err(_) => Err("the PipeWire thread ended".into()),
    }
}

struct State {
    engine: Engine,
    left: Vec<f32>,
    right: Vec<f32>,
    marked: bool,
}

fn run(engine: Engine, rate: u32, max_frames: u32, device: &str, quit: pw::channel::Receiver<()>, ready: &std::sync::mpsc::Sender<Result<(), String>>) -> Result<(), String> {
    pw::init();
    let mainloop = match pw::main_loop::MainLoop::new(None) {
        Ok(m) => m,
        Err(e) => return Err(format!("PipeWire: {e}")),
    };
    let context = match pw::context::Context::new(&mainloop) {
        Ok(c) => c,
        Err(e) => return Err(format!("PipeWire: {e}")),
    };
    let core = match context.connect(None) {
        Ok(c) => c,
        Err(e) => return Err(format!("PipeWire is not running ({e})")),
    };
    // Ask for the buffer the preferences set: it is the time every plugin
    // has to render a block. (Capping it at 256 frames left ~5 ms at 48 kHz,
    // which a couple of sampled instruments already overrun.)
    let latency = format!("{max_frames}/{rate}");
    let mut props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::NODE_NAME => "corinzia",
        *pw::keys::APP_NAME => "Corinzia",
        *pw::keys::NODE_LATENCY => latency.as_str(),
    };
    if !device.is_empty() {
        props.insert("target.object", device);
    }
    let stream = match pw::stream::Stream::new(&core, "Corinzia", props) {
        Ok(s) => s,
        Err(e) => return Err(format!("PipeWire stream: {e}")),
    };
    let block = max_frames as usize;
    let state = State { engine, left: vec![0.0; block], right: vec![0.0; block], marked: false };
    let listener = stream
        .add_local_listener_with_user_data(state)
        .process(|stream, st| {
            if !st.marked {
                crate::host::mark_audio_thread();
                st.marked = true;
            }
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            let data = &mut datas[0];
            let stride = 2 * std::mem::size_of::<f32>();
            let frames = match data.data() {
                Some(bytes) => {
                    let frames = bytes.len() / stride;
                    let mut at = 0;
                    while at < frames {
                        let n = (frames - at).min(st.left.len());
                        let (l, r) = (&mut st.left[..n], &mut st.right[..n]);
                        st.engine.process(l, r);
                        for i in 0..n {
                            let o = (at + i) * stride;
                            bytes[o..o + 4].copy_from_slice(&st.left[i].to_le_bytes());
                            bytes[o + 4..o + 8].copy_from_slice(&st.right[i].to_le_bytes());
                        }
                        at += n;
                    }
                    frames
                }
                None => 0,
            };
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as _;
            *chunk.size_mut() = (stride * frames) as _;
        })
        .register();
    let _listener = match listener {
        Ok(l) => l,
        Err(e) => return Err(format!("PipeWire listener: {e}")),
    };

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(rate);
    info.set_channels(2);
    let mut position = [0u32; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object { type_: spa::sys::SPA_TYPE_OBJECT_Format, id: spa::sys::SPA_PARAM_EnumFormat, properties: info.into() }),
    )
    .expect("audio format pod")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).expect("pod")];
    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )
    .map_err(|e| format!("PipeWire connect: {e}"))?;
    let ml = mainloop.clone();
    let _quit = quit.attach(mainloop.loop_(), move |_| ml.quit());
    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}
