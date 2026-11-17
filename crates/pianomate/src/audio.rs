//! Sound out (SoundFont synth and metronome on the audio thread) and MIDI in.

use std::{
    error::Error,
    f32::consts::TAU,
    fs::File,
    io::BufReader,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use midir::{Ignore, MidiInput};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};

#[derive(Clone, Copy)]
pub enum Msg {
    /// Raw MIDI for the piano: status, data1, data2.
    Midi([u8; 3]),
    /// Start the metronome now, with a higher click every `accent_every` beats (0 = never).
    Click { bpm: f32, accent_every: u32 },
    ClickOff,
}

const CHUNK: usize = 1024;

/// The one clock all timing is judged on: milliseconds since the app started.
pub fn now_ms() -> f64 {
    static START: LazyLock<Instant> = LazyLock::new(Instant::now);
    START.elapsed().as_secs_f64() * 1000.0
}

pub struct Audio {
    /// Keeps the output running until dropped.
    _stream: cpal::Stream,
    /// One-line description of the audio setup.
    pub info: String,
    click_start: Arc<AtomicU64>,
    tx: Arc<Mutex<rtrb::Producer<Msg>>>,
}

impl Audio {
    pub fn start(sf_path: &str) -> Result<Audio, Box<dyn Error>> {
        now_ms(); // start the clock here, not on the realtime thread
        let file = File::open(sf_path).map_err(|e| format!("cannot open SoundFont {sf_path}: {e}"))?;
        let sf = Arc::new(SoundFont::new(&mut BufReader::new(file))?);

        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config()?;
        // shortcut: f32 output only (what PipeWire/CoreAudio/WASAPI give by default); convert samples if a device needs i16.
        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("audio device wants {}, only f32 is supported", supported.sample_format()).into());
        }
        let mut config: cpal::StreamConfig = supported.into();
        let click_start = Arc::new(AtomicU64::new(f64::NAN.to_bits()));
        // Small buffer for low key-to-sound latency; fall back to the device default if refused.
        config.buffer_size = cpal::BufferSize::Fixed(256);
        let (stream, tx, buffer) = match build_stream(&device, &config, &sf, &click_start) {
            Ok((s, tx)) => (s, tx, format!("256 frames ({:.1} ms)", 256e3 / config.sample_rate as f64)),
            Err(_) => {
                config.buffer_size = cpal::BufferSize::Default;
                let (s, tx) = build_stream(&device, &config, &sf, &click_start)?;
                (s, tx, "device default".to_string())
            }
        };
        stream.play()?;
        Ok(Audio { _stream: stream, info: format!("{} Hz, buffer {buffer}", config.sample_rate), click_start, tx: Arc::new(Mutex::new(tx)) })
    }

    /// A handle any thread can use to send messages to the audio thread.
    pub fn sender(&self) -> impl Fn(Msg) + Clone + Send + 'static {
        let tx = self.tx.clone();
        move |msg| {
            let _ = tx.lock().unwrap().push(msg); // full queue: drop the event rather than block
        }
    }

    /// Starts the metronome. `click_start` reads as `None` until the audio thread has begun it.
    pub fn click(&self, bpm: f32, accent_every: u32) {
        self.click_start.store(f64::NAN.to_bits(), Ordering::Relaxed);
        self.sender()(Msg::Click { bpm, accent_every });
    }

    /// When the first click of the running metronome began, on the `now_ms` clock.
    pub fn click_start(&self) -> Option<f64> {
        Some(f64::from_bits(self.click_start.load(Ordering::Relaxed))).filter(|t| !t.is_nan())
    }
}

#[derive(Default)]
struct Metronome {
    /// Frames between clicks; 0 = off.
    period: f64,
    until_next: f64,
    beat: u32,
    accent_every: u32,
    // The click currently sounding: a short decaying sine.
    freq: f32,
    phase: f32,
    level: f32,
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sf: &Arc<SoundFont>,
    click_start: &Arc<AtomicU64>,
) -> Result<(cpal::Stream, rtrb::Producer<Msg>), Box<dyn Error>> {
    let (tx, mut rx) = rtrb::RingBuffer::<Msg>::new(1024);
    let rate = config.sample_rate as f32;
    let mut synth = Synthesizer::new(sf, &SynthesizerSettings::new(rate as i32))?;
    let channels = config.channels as usize;
    let (mut left, mut right) = ([0f32; CHUNK], [0f32; CHUNK]);
    let mut click = Metronome::default();
    let decay = 0.001f32.powf(1.0 / (0.04 * rate)); // each click fades out over 40 ms
    let click_start = click_start.clone();
    let stream = device.build_output_stream(
        *config,
        // Realtime thread: no locks, no allocation.
        move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
            while let Ok(msg) = rx.pop() {
                match msg {
                    Msg::Midi([status, d1, d2]) => synth.process_midi_message(0, status as i32, d1 as i32, d2 as i32),
                    Msg::Click { bpm, accent_every } => {
                        click = Metronome { period: rate as f64 * 60.0 / bpm as f64, accent_every, ..Default::default() };
                        click_start.store(now_ms().to_bits(), Ordering::Relaxed);
                    }
                    Msg::ClickOff => click.period = 0.0,
                }
            }
            for block in out.chunks_mut(CHUNK * channels) {
                let n = block.len() / channels;
                synth.render(&mut left[..n], &mut right[..n]);
                for (i, frame) in block.chunks_mut(channels).enumerate() {
                    if click.period > 0.0 {
                        if click.until_next <= 0.0 {
                            let accent = click.accent_every > 0 && click.beat % click.accent_every == 0;
                            (click.freq, click.phase, click.level) = (if accent { 1760.0 } else { 1175.0 }, 0.0, 0.4);
                            click.beat += 1;
                            click.until_next += click.period;
                        }
                        click.until_next -= 1.0;
                    }
                    let mut tick = 0.0;
                    if click.level > 0.0005 {
                        tick = click.phase.sin() * click.level;
                        click.phase = (click.phase + TAU * click.freq / rate) % TAU;
                        click.level *= decay;
                    }
                    frame.fill(0.0);
                    frame[0] = left[i] + tick;
                    if channels > 1 {
                        frame[1] = right[i] + tick;
                    }
                }
            }
        },
        |e| eprintln!("audio error: {e}"),
        None,
    )?;
    Ok((stream, tx))
}

type KeyFn = Arc<dyn Fn(u8, bool, f64) + Send + Sync>;
type SendFn = Arc<dyn Fn(Msg) + Send + Sync>;

/// Every connected MIDI keyboard. Notes go to `send` (for sound) and `on_key(note, down, time)`.
pub struct Midi {
    conns: Vec<(String, midir::MidiInputConnection<()>)>,
    send: SendFn,
    on_key: KeyFn,
}

impl Midi {
    pub fn new(send: impl Fn(Msg) + Send + Sync + 'static, on_key: impl Fn(u8, bool, f64) + Send + Sync + 'static) -> Midi {
        Midi { conns: Vec::new(), send: Arc::new(send), on_key: Arc::new(on_key) }
    }

    pub fn names(&self) -> Vec<String> {
        // Port names look like "Roland Digital Piano:Roland Digital Piano MIDI 1 24:0": keep the device.
        self.conns.iter().map(|(name, _)| name.split(':').next().unwrap_or(name).to_string()).collect()
    }

    /// Connects keyboards plugged in since the last call and drops ones unplugged.
    /// Returns whether the set of keyboards changed.
    pub fn refresh(&mut self) -> bool {
        let Ok(probe) = MidiInput::new("PianoMate") else { return false };
        let present: Vec<(String, midir::MidiInputPort)> = probe
            .ports()
            .into_iter()
            .filter_map(|p| Some((probe.port_name(&p).ok()?, p)))
            .filter(|(name, _)| !name.contains("Midi Through"))
            .collect();
        let before = self.conns.len();
        self.conns.retain(|(name, _)| present.iter().any(|(n, _)| n == name));
        let mut changed = self.conns.len() != before;
        for (name, port) in present {
            if self.conns.iter().any(|(n, _)| *n == name) {
                continue;
            }
            // midir consumes the client on connect, so each port gets its own.
            let Ok(mut input) = MidiInput::new("PianoMate") else { continue };
            input.ignore(Ignore::All);
            let (send, on_key) = (self.send.clone(), self.on_key.clone());
            let conn = input.connect(
                &port,
                "in",
                move |_, msg, _| {
                    let at = now_ms();
                    let &[status, d1, d2] = msg else { return };
                    // Notes and sustain pedal only, forced to channel 0 (piano): no program changes or drum channel.
                    let kind = status & 0xF0;
                    if kind == 0x80 || kind == 0x90 {
                        send(Msg::Midi([kind, d1, d2]));
                        on_key(d1, kind == 0x90 && d2 > 0, at);
                    } else if kind == 0xB0 && d1 == 64 {
                        send(Msg::Midi([kind, d1, d2]));
                    }
                },
                (),
            );
            if let Ok(conn) = conn {
                self.conns.push((name, conn));
                changed = true;
            }
        }
        changed
    }
}
