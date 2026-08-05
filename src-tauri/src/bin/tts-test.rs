// Minimal TTS smoke test — verify the Piper amy model loads and generates audio.
// Run from src-tauri/ with: cargo run --example tts_smoke
// Or place in src-tauri/src/bin/ and run with: cargo run --bin tts-test

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsVitsModelConfig,
};
use std::time::Instant;

fn main() {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    eprintln!("base: {}", base.display());

    let model = base.join("tts/tts_piper_amy_low/en_US-amy-low.onnx");
    let tokens = base.join("tts/tts_piper_amy_low/tokens.txt");
    let data_dir = base.join("tts/tts_piper_amy_low/espeak-ng-data");

    eprintln!("model: {} (exists: {})", model.display(), model.exists());
    eprintln!("tokens: {} (exists: {})", tokens.display(), tokens.exists());
    eprintln!(
        "data_dir: {} (exists: {})",
        data_dir.display(),
        data_dir.exists()
    );

    let config = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            vits: OfflineTtsVitsModelConfig {
                model: Some(model.to_str().unwrap().to_string()),
                tokens: Some(tokens.to_str().unwrap().to_string()),
                data_dir: Some(data_dir.to_str().unwrap().to_string()),
                noise_scale: 0.667,
                noise_scale_w: 0.8,
                length_scale: 1.0,
                ..Default::default()
            },
            num_threads: 2,
            debug: true,
            ..Default::default()
        },
        ..Default::default()
    };

    eprintln!("Creating TTS engine...");
    let start = Instant::now();
    let tts = OfflineTts::create(&config).expect("Failed to create TTS");
    eprintln!(
        "TTS created in {:.1}s  sr={}  speakers={}",
        start.elapsed().as_secs_f32(),
        tts.sample_rate(),
        tts.num_speakers()
    );

    let gen = GenerationConfig {
        sid: 0,
        speed: 1.0,
        ..Default::default()
    };
    let text = "Hello! Ayla is ready to work with you.";
    eprintln!("Generating: \"{}\"", text);

    let start2 = Instant::now();
    let audio = tts
        .generate_with_config(
            text,
            &gen,
            Some(|_s: &[f32], p: f32| -> bool {
                eprintln!("  progress: {:.0}%", p * 100.0);
                true
            }),
        )
        .expect("Generation failed");

    let elapsed = start2.elapsed().as_secs_f32();
    let dur = audio.samples().len() as f32 / audio.sample_rate() as f32;
    eprintln!(
        "Generated {:.1}s audio in {:.1}s (RTF {:.2})",
        dur,
        elapsed,
        elapsed / dur
    );

    let out = "/tmp/tts-test.wav";
    if audio.save(out) {
        eprintln!("Saved to {}", out);
    } else {
        eprintln!("Failed to save");
    }
}
