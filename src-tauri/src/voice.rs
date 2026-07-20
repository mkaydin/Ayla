// Voice input pipeline: microphone → Silero VAD → SenseVoice STT.
//
// Adapted from sherpa-onnx's sense_voice_simulate_streaming_microphone.rs.
// Runs on a background thread; communicates with the UI via Tauri events.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use sherpa_onnx::{
    DisplayManager, LinearResampler, OfflineRecognizer, OfflineRecognizerConfig,
    VadModelConfig, VoiceActivityDetector,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

// ---------------------------------------------------------------------------
// Tauri managed state
// ---------------------------------------------------------------------------

pub struct VoiceState {
    running: Arc<AtomicBool>,
}

impl Default for VoiceState {
    fn default() -> Self {
        Self { running: Arc::new(AtomicBool::new(false)) }
    }
}

// ---------------------------------------------------------------------------
// Resolve model paths (same pattern as tts.rs)
// ---------------------------------------------------------------------------

fn models_dir() -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    base.join("models").to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn start_listening(state: tauri::State<'_, VoiceState>, app: AppHandle) -> Result<(), String> {
    if state.running.load(Ordering::SeqCst) {
        return Err("Already listening".into());
    }
    state.running.store(true, Ordering::SeqCst);

    let running = state.running.clone();
    let silero_vad_model = format!("{}/silero_vad.onnx", models_dir());
    let sense_voice_model = format!("{}/sense-voice.onnx", models_dir());
    let tokens = format!("{}/sense-voice-tokens.txt", models_dir());

    log::info!("Starting voice: vad={} stt={}", silero_vad_model, sense_voice_model);

    std::thread::spawn(move || {
        if let Err(e) = voice_loop(app, running, &silero_vad_model, &sense_voice_model, &tokens) {
            log::error!("Voice loop error: {}", e);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn stop_listening(state: tauri::State<'_, VoiceState>) -> Result<(), String> {
    state.running.store(false, Ordering::SeqCst);
    Ok(())
}

// ---------------------------------------------------------------------------
// Background voice loop (runs on a dedicated thread)
// ---------------------------------------------------------------------------

fn voice_loop(
    app: AppHandle,
    running: Arc<AtomicBool>,
    silero_vad_model: &str,
    sense_voice_model: &str,
    tokens: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = app.emit("listening-started", ());

    // ---- VAD ----
    let mut vad_cfg = VadModelConfig::default();
    vad_cfg.silero_vad.model = Some(silero_vad_model.into());
    vad_cfg.silero_vad.threshold = 0.5;
    vad_cfg.silero_vad.min_silence_duration = 0.15;
    vad_cfg.silero_vad.min_speech_duration = 0.25;
    vad_cfg.silero_vad.max_speech_duration = 8.0;
    vad_cfg.silero_vad.window_size = 512;
    vad_cfg.sample_rate = 16000;
    let vad = VoiceActivityDetector::create(&vad_cfg, 20.0)
        .ok_or("Failed to create VAD")?;

    // ---- STT ----
    let mut rec_cfg = OfflineRecognizerConfig::default();
    rec_cfg.model_config.sense_voice.model = Some(sense_voice_model.into());
    rec_cfg.model_config.sense_voice.language = Some("auto".into());
    rec_cfg.model_config.sense_voice.use_itn = false;
    rec_cfg.model_config.tokens = Some(tokens.into());
    rec_cfg.model_config.num_threads = 2;
    let recognizer = OfflineRecognizer::create(&rec_cfg)
        .ok_or("Failed to create STT recognizer")?;

    log::info!("Voice: VAD + STT loaded");

    // ---- Microphone ----
    let host = cpal::default_host();
    let device = host.default_input_device()
        .ok_or_else(|| {
            let _ = app.emit("voice-error", "No microphone found on this system.");
            "No microphone found"
        })?;
    let device_name = device.name().unwrap_or_default();
    let supported = device.default_input_config().map_err(|e| {
        let msg = format!("Cannot open microphone '{}': {}", device_name, e);
        let _ = app.emit("voice-error", &msg);
        msg
    })?;
    let mic_sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();

    let resampler = if mic_sample_rate != 16000 {
        Some(LinearResampler::create(mic_sample_rate as i32, 16000_i32)
            .ok_or("Failed to create resampler")?)
    } else {
        None
    };

    log::info!("Voice: mic={}hz ch={}", mic_sample_rate, channels);

    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let stream_err = |err| { log::error!("Audio stream error: {:?}", err); };

    let audio_stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(&supported.config(),
            move |data: &[f32], _| {
                if data.is_empty() { return; }
                let mono: Vec<f32> = data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect();
                let _ = tx.send(mono);
            }, stream_err, None),
        SampleFormat::I16 => device.build_input_stream(&supported.config(),
            move |data: &[i16], _| {
                if data.is_empty() { return; }
                let mono: Vec<f32> = data.chunks(channels).map(|f| f.iter().map(|&s| s as f32 / 32767.0).sum::<f32>() / channels as f32).collect();
                let _ = tx.send(mono);
            }, stream_err, None),
        SampleFormat::U16 => device.build_input_stream(&supported.config(),
            move |data: &[u16], _| {
                if data.is_empty() { return; }
                let mono: Vec<f32> = data.chunks(channels).map(|f| f.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum::<f32>() / channels as f32).collect();
                let _ = tx.send(mono);
            }, stream_err, None),
        other => return Err(format!("Unsupported sample format: {:?}", other).into()),
    }.map_err(|e| {
        let msg = format!("Cannot open audio stream: {}", e);
        let _ = app.emit("voice-error", &msg);
        msg
    })?;

    audio_stream.play()?;
    let _ = app.emit("listening-started", ());

    // ---- Main processing loop ----
    let window_size = 512usize;
    let mut buffer = Vec::<f32>::new();
    let mut offset: usize = 0;
    let mut speech_started = false;
    let mut started_time = Instant::now();
    let mut display = DisplayManager::new();

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(samples) => {
                if let Some(ref resampler) = resampler {
                    let resampled = resampler.resample(&samples, false);
                    buffer.extend_from_slice(&resampled);
                } else {
                    buffer.extend_from_slice(&samples);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        }

        // Feed VAD
        while offset + window_size <= buffer.len() {
            vad.accept_waveform(&buffer[offset..offset + window_size]);
            if !speech_started && vad.detected() {
                speech_started = true;
                started_time = Instant::now();
                let _ = app.emit("speech-detected", ());
            }
            offset += window_size;
        }

        // Trim buffer when idle to prevent unbounded growth
        if !speech_started && buffer.len() > 10 * window_size {
            let trim = buffer.len() - 10 * window_size;
            offset = offset.saturating_sub(trim);
            buffer.drain(..trim);
        }

        // Interim partial decode (~5 Hz)
        let elapsed = started_time.elapsed().as_secs_f32();
        if speech_started && elapsed > 0.2 {
            let stream = recognizer.create_stream();
            stream.accept_waveform(16000, &buffer);
            recognizer.decode(&stream);
            if let Some(result) = stream.get_result() {
                let _ = app.emit("partial-transcript", result.text.clone());
                display.update_text(&result.text);
            }
            started_time = Instant::now();
        }

        // Final decode on completed VAD segments
        while !vad.is_empty() {
            if let Some(segment) = vad.front() {
                vad.pop();
                let stream = recognizer.create_stream();
                stream.accept_waveform(16000, segment.samples());
                recognizer.decode(&stream);
                if let Some(result) = stream.get_result() {
                    let text = result.text;
                    if !text.trim().is_empty() {
                        let _ = app.emit("final-transcript", text.clone());
                        let _ = app.emit("voice-query", text.clone());
                    }
                    display.update_text(&text);
                    display.finalize_sentence();
                }
            }
            buffer.clear();
            offset = 0;
            speech_started = false;
        }
    }

    let _ = app.emit("listening-stopped", ());
    log::info!("Voice: stopped");
    Ok(())
}
