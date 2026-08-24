//! In-process speech-to-text: whisper-rs + cpal mic capture.
//!
//! A dedicated worker thread owns the whisper model (loaded from voxtype's
//! GGML files when present — zero downloads). The GTK thread feeds it PCM
//! samples captured while the hotkey is held and polls transcripts back.
//!
//! Everything whisper-related lives on the worker thread; nothing whisper
//! crosses a thread boundary.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Messages into the STT worker (single channel — std mpsc has no select).
pub enum SttMsg {
    /// 16kHz mono f32 samples captured from the mic
    Audio(Vec<f32>),
    /// hot-swap the model (settings panel changed it)
    LoadModel(PathBuf),
}

pub struct SttHandle {
    pub tx: Sender<SttMsg>,
    pub transcript_rx: Receiver<String>,
}

/// Spawn the worker; `model` is resolved and loaded before the loop starts.
/// If loading fails the worker stays alive (so settings can fix it) but
/// audio is dropped with an error on stdout.
pub fn spawn(model: PathBuf, language: String) -> SttHandle {
    // route whisper.cpp/ggml logs into nothing (we're not using a log backend);
    // without this whisper prints token-level debug spam to stderr
    whisper_rs::install_logging_hooks();
    let (tx, rx) = channel::<SttMsg>();
    let (tx_out, transcript_rx) = channel::<String>();
    std::thread::spawn(move || worker(rx, tx_out, model, language));
    SttHandle { tx, transcript_rx }
}

fn worker(rx: Receiver<SttMsg>, tx_out: Sender<String>, model_path: PathBuf, language: String) {
    let mut current_model = model_path.clone();
    let mut ctx = load_model(&model_path);

    loop {
        let msg = match rx.recv() {
            Ok(m) => m,
            Err(_) => return, // sender dropped — app shutting down
        };
        match msg {
            SttMsg::LoadModel(path) => {
                if path == current_model {
                    continue;
                }
                println!("STT: loading model {:?}", path);
                match load_model(&path) {
                    Some(c) => {
                        ctx = Some(c);
                        current_model = path;
                        println!("STT: model ready");
                    }
                    None => println!("STT: failed to load {:?}", path),
                }
            }
            SttMsg::Audio(samples) => {
                let Some(ctx) = &ctx else {
                    println!("STT: no model loaded, dropped {} samples", samples.len());
                    continue;
                };
                if samples.len() < 8000 {
                    continue; // < 0.5s — not worth a whisper pass
                }
                let started = std::time::Instant::now();
                let Ok(mut state) = ctx.create_state() else {
                    continue;
                };
                let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                params.set_language(if language.is_empty() { None } else { Some(&language) });
                params.set_translate(false);
                params.set_print_special(false);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_timestamps(false);
                if state.full(params, &samples).is_err() {
                    println!("STT: inference failed");
                    continue;
                }
                let n = state.full_n_segments();
                let mut text = String::new();
                for i in 0..n {
                    if let Some(seg) = state.get_segment(i) {
                        let t = seg.to_str().unwrap_or("").trim();
                        // whisper emits bracketed stage directions ([BLANK_AUDIO],
                        // [MUSIC], ...) when fed silence — drop them wholesale
                        if t.starts_with('[') || t.starts_with('(') {
                            continue;
                        }
                        text.push_str(t);
                    }
                }
                let text = text.trim().to_string();
                println!(
                    "STT: {:.2}s audio -> \"{}\" ({:.2}s infer)",
                    samples.len() as f32 / 16000.0,
                    text,
                    started.elapsed().as_secs_f32()
                );
                let _ = tx_out.send(text);
            }
        }
    }
}

fn load_model(path: &PathBuf) -> Option<WhisperContext> {
    match WhisperContext::new_with_params(path, WhisperContextParameters::new()) {
        Ok(c) => Some(c),
        Err(e) => {
            println!("STT: model load error ({}): {}", path.display(), e);
            None
        }
    }
}

/// Resolve a model name across known dirs, falling back to any available one.
pub fn resolve_model(preferred: &str) -> Option<PathBuf> {
    crate::settings::find_model(preferred)
        .or_else(|| {
            crate::settings::list_models()
                .first()
                .and_then(|name| crate::settings::find_model(name))
        })
        .or_else(|| {
            println!("STT: no GGML model found in heyclicky/voxtype model dirs");
            None
        })
}

// --- mic capture (created + dropped on the GTK thread) ----------------------

pub struct MicCapture {
    samples: Arc<Mutex<Vec<f32>>>,
    stream: Option<cpal::Stream>,
}

impl MicCapture {
    /// Start capturing the default input device at 16kHz mono.
    pub fn start() -> Result<MicCapture, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no input device".to_string())?;
        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));

        // try 16k mono first (PipeWire resamples); fall back to the device's
        // native config with crude decimation
        let want = cpal::StreamConfig {
            channels: 1,
            sample_rate: 16000,
            buffer_size: cpal::BufferSize::Default,
        };
        let build = |cfg: &cpal::StreamConfig| {
            let sink = Arc::clone(&samples);
            let rate = cfg.sample_rate;
            let chans = cfg.channels as usize;
            device
                .build_input_stream(
                    *cfg,
                    move |data: &[f32], _| push_samples(&sink, data, rate, chans),
                    |err| println!("MIC: stream error: {}", err),
                    None,
                )
                .map_err(|e| e.to_string())
        };
        let stream = match build(&want) {
            Ok(s) => s,
            Err(e) => {
                println!("MIC: 16k mono rejected ({}), using device default", e);
                let native = device
                    .default_input_config()
                    .map_err(|e| e.to_string())?
                    .into();
                build(&native).map_err(|e| format!("native stream: {}", e))?
            }
        };
        stream.play().map_err(|e| format!("stream play: {}", e))?;
        println!("MIC: capturing");
        Ok(MicCapture { samples, stream: Some(stream) })
    }

    /// Stop capturing and return the recorded samples.
    pub fn stop(mut self) -> Vec<f32> {
        self.stream.take(); // drop closes the stream
        let data = self.samples.lock().map(|m| m.clone()).unwrap_or_default();
        println!("MIC: stopped ({} samples, {:.2}s)", data.len(), data.len() as f32 / 16000.0);
        data
    }
}

// decimate to ~16k mono when the device gave us a higher rate / more channels
fn push_samples(sink: &Mutex<Vec<f32>>, data: &[f32], rate: u32, chans: usize) {
    let Ok(mut buf) = sink.lock() else { return };
    let step = (rate / 16000).max(1) as usize;
    if step <= 1 {
        if chans == 1 {
            buf.extend_from_slice(data);
        } else {
            buf.extend(data.chunks(chans).filter_map(|c| c.first().copied()));
        }
    } else {
        let mono: Vec<f32> = if chans == 1 {
            data.to_vec()
        } else {
            data.chunks(chans).filter_map(|c| c.first().copied()).collect()
        };
        buf.extend(mono.iter().step_by(step).copied());
    }
}

// --- wav reading (stt-test mode only) ---------------------------------------

/// Minimal PCM16 WAV loader for `stt-test`.
pub fn load_wav_16k_mono(path: &PathBuf) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let read_u32 = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let read_u16 = |o: usize| u16::from_le_bytes(bytes[o..o + 2].try_into().unwrap());
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut fmt, mut data): (Option<(u16, u32)>, Option<&[u8]>) = (None, None);
    let mut o = 12;
    while o + 8 <= bytes.len() {
        let id = &bytes[o..o + 4];
        let size = read_u32(o + 4) as usize;
        let body = o + 8;
        if id == b"fmt " {
            let (ch, rate) = (read_u16(body), read_u32(body + 4));
            fmt = Some((ch, rate));
        } else if id == b"data" {
            data = Some(&bytes[body..(body + size).min(bytes.len())]);
        }
        o = body + size + (size & 1);
    }
    let (chans, rate) = fmt.ok_or("no fmt chunk")?;
    let raw = data.ok_or("no data chunk")?;
    let mono: Vec<f32> = raw
        .chunks_exact(2 * chans as usize)
        .enumerate()
        .filter_map(|(i, c)| {
            if chans == 1 || i % chans as usize == 0 {
                Some(i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
            } else {
                None
            }
        })
        .collect();
    // crude decimation to 16k if needed
    let step = (rate / 16000).max(1) as usize;
    Ok(if step <= 1 { mono } else { mono.into_iter().step_by(step).collect() })
}
