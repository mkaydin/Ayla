//! Decode a 16 kHz mono PCM16 fixture through Ayla's STT and VAD paths.
//!
//! Usage: cargo run -p ayla-stt --example transcribe_pcm -- FILE.pcm

use ayla_stt::{SttEngine, SttEvent, SttModel, SttOptions};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let pcm_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("pass a 16 kHz mono PCM16 fixture path");
    let root = PathBuf::from("models/stt");
    let model_root =
        root.join("sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-int8-2025-11-12");
    let engine = SttEngine::load(
        SttModel {
            model: model_root.join("model.int8.onnx"),
            tokens: model_root.join("tokens.txt"),
            vad: root.join("silero_vad.onnx"),
        },
        SttOptions::default(),
    )
    .expect("load STT engine");
    let pcm = std::fs::read(&pcm_path).expect("read PCM fixture");
    assert_eq!(
        pcm.len() % 2,
        0,
        "PCM fixture must contain complete i16 samples"
    );
    let samples = pcm
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32768.0)
        .collect::<Vec<_>>();
    let started = Instant::now();
    let text = engine.decode(&samples).expect("decode speech");
    let audio_seconds = samples.len() as f32 / 16_000.0;
    let decode_seconds = started.elapsed().as_secs_f32();
    println!("text={text:?}");
    println!(
        "audio={audio_seconds:.2}s decode={decode_seconds:.2}s rtf={:.2}",
        decode_seconds / audio_seconds
    );

    let mut session = engine.start_session().expect("start VAD session");
    let mut finals = Vec::new();
    for chunk in pcm.chunks(3_200) {
        for event in session.push_pcm16(&engine, chunk).expect("push PCM") {
            if let SttEvent::Final { text, .. } = event {
                finals.push(text);
            }
        }
    }
    let silence = vec![0_u8; 16_000 * 2];
    for chunk in silence.chunks(3_200) {
        for event in session.push_pcm16(&engine, chunk).expect("push silence") {
            if let SttEvent::Final { text, .. } = event {
                finals.push(text);
            }
        }
    }
    for event in session.finish(&engine).expect("finish VAD") {
        if let SttEvent::Final { text, .. } = event {
            finals.push(text);
        }
    }
    println!("vad_finals={finals:?}");
    assert!(!text.is_empty(), "fixture produced an empty transcript");
    assert!(!finals.is_empty(), "VAD produced no finalized transcript");
}
