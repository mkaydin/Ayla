# DemonGirl — Local AI Voice Assistant

A fully offline desktop AI assistant featuring an interactive **Live2D anime-style "Demon Girl" avatar** with voice interaction. Type or speak, and the avatar responds with synthesized speech while animating facial expressions, lip sync, blinking, and head tracking.

Everything runs locally — no cloud services required. Built with Tauri, Rust, PixiJS, and ONNX models.

## Features

- **Voice input** — speak via microphone with VAD-based speech detection and streaming STT
- **Streaming chat** — text input with local LLM via Ollama, streaming token-by-token
- **Text-to-Speech** — Piper VITS voice synthesis with audio-timed lip sync
- **Live2D avatar** — animated demon girl with 7 expressions, eye tracking, breathing, blink
- **Fully offline** — all models run locally; no API keys, no internet needed
- **Desktop app** — Tauri 2 native window (1280×800), AppImage/.deb packaging

## Architecture

```
Microphone ──► Silero VAD ──► SenseVoice STT ──► Chat Input
                                                      │
                                                      ▼
                                              Ollama (gemma4)
                                                      │ streaming tokens
                                                      ▼
                                         chatSendMessage()
                                        ┌─────────────────┐
                                        │ strip [tags]    │──► applyExpression() ──► Live2D face
                                        │ split sentences │──► ttsSchedule() ──► generate_tts
                                        │ render bubbles  │──► Chatbox UI
                                        └─────────────────┘
                                                      │
                                                      ▼
                                           Piper VITS TTS ──► speaker
                                              │
                                              ▼
                                        Web Audio Analyzer ──► mouth animation
```

### Tech Stack

| Layer | Technology |
|---|---|
| Desktop framework | Tauri 2 (Rust + WebView) |
| Frontend build | Vite 5 |
| 2D rendering | PixiJS 6 + pixi-live2d-display (Cubism 4) |
| LLM backend | Ollama (`gemma4:e4b`) |
| Speech-to-Text | Sherpa-ONNX SenseVoice |
| Voice Activity Detection | Sherpa-ONNX Silero VAD |
| Text-to-Speech | Sherpa-ONNX Piper VITS ("amy-low") |
| Audio capture | cpal (Rust) |
| Markdown parsing | marked |

## Prerequisites

- **Rust** (1.77+) — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **Node.js** (18+) — for Vite and npm packages
- **Ollama** — `ollama pull gemma4:e4b`
- **Tauri system dependencies**:
  - Arch: `sudo pacman -S webkit2gtk-4.1 libappindicator-gtk3`
  - Debian/Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev`

## Model Files

Download the ONNX models into the project directories:

```
models/
├── silero_vad.onnx           # Silero Voice Activity Detection
├── sense-voice.onnx          # SenseVoice speech-to-text (~240 MB)
└── sense-voice-tokens.txt    # Token vocabulary

tts/tts_piper_amy_low/
├── en_US-amy-low.onnx        # Piper TTS voice (~63 MB)
├── en_US-amy-low.onnx.json   # Model config
├── espeak-ng-data/           # Phonemizer data
└── tokens.txt                # Voice tokens
```

Model sources:
- SenseVoice: [Sherpa-ONNX model list](https://github.com/k2-fsa/sherpa-onnx/releases)
- Silero VAD: [Sherpa-ONNX Silero VAD](https://github.com/k2-fsa/sherpa-onnx/releases)
- Piper amy-low: [Sherpa-ONNX TTS models](https://github.com/k2-fsa/sherpa-onnx/releases)

## Setup

```bash
# 1. Clone the repo
git clone <repo-url>
cd Alice_AI_Demoness

# 2. Install Node dependencies
npm install

# 3. Download ONNX models (see Model Files section above)
#    Place them in models/ and tts/tts_piper_amy_low/

# 4. Pull the LLM model
ollama pull gemma4:e4b
ollama serve

# 5. Run in development
npm run tauri dev

# 6. Build for production
npm run tauri build
```

## Project Structure

```
├── index.html                  # Main HTML with HUD, canvas, and chatbox
├── vite.config.js              # Vite config (includes Ollama API proxy)
├── package.json                # JS dependencies and scripts
├── PLAN.md                     # Architecture design document
│
├── src/                        # Frontend (vanilla JavaScript)
│   ├── main.js                 # Live2D model, chat pipeline, voice control, animation loop
│   └── tts-player.js           # Web Audio TTS queue player + mouth level extraction
│
├── src-tauri/                  # Rust backend
│   ├── Cargo.toml              # Rust dependencies
│   ├── tauri.conf.json         # Tauri window and bundle config
│   └── src/
│       ├── main.rs             # Entry point (Linux/Wayland env vars)
│       ├── lib.rs              # App setup, Tauri commands, managed state
│       ├── tts.rs              # Piper VITS TTS engine (text → WAV)
│       └── voice.rs            # Microphone → VAD → STT pipeline
│
├── public/DemonGirl/           # Live2D Cubism 4 model files
│   ├── demongirl.model3.json   # Model descriptor
│   ├── demongirl.moc3          # Model mesh and rigging
│   ├── demongirl.physics3.json # Physics simulation
│   └── *.exp3.json             # Expression files (7 emotions)
│
├── models/                     # ONNX AI models (downloaded separately)
├── tts/                        # TTS voice models + sherpa-onnx vendored lib
└── sound_effects/              # Sound effects (not yet wired)
```

## Configuration

### Ollama model

Set in `src/main.js:581`:
```js
body: JSON.stringify({ model: 'gemma4:e4b', stream: true, messages })
```

### VAD thresholds

Set in `src-tauri/src/voice.rs:88-97`:
```rust
silero_vad_config.threshold = 0.5;          // Speech detection sensitivity
silero_vad_config.min_silence_duration = 0.15; // Seconds
silero_vad_config.min_speech_duration = 0.25;  // Seconds
silero_vad_config.max_speech_duration = 8.0;   // Seconds
```

### TTS speed

Set in `src-tauri/src/tts.rs`, speed parameter:
```rust
tts_config.speed = 1.10;  // 1.0 = normal, 1.10 = slightly faster
```

### Window size

Set in `src-tauri/tauri.conf.json`:
```json
"width": 1280, "height": 800
```

## Chat System Prompt

Alice is configured as a demon girl assistant. Every response must start with an expression tag that drives the Live2D avatar:

```
You are a cute demon girl. Reply with short answers. Keep it conversational.
Prefix every response with an emotion tag: [amazed] [angry] [cry] [customise]
[love] [nervous] [sleepy]
```

Available expressions: Amazed, Angry, Cry, Love, Nervous, Sleepy, Customise.

## Development

```bash
npm run dev            # Vite dev server only
npm run tauri dev      # Full Tauri app in dev mode
npm run tauri build    # Production build → AppImage + .deb
```

### Adding new expressions

1. Place `.exp3.json` file in `public/DemonGirl/`
2. Add the expression name to the `EXPRESSIONS` array in `src/main.js:12`
3. Add a HUD button in `index.html` controls section
4. Add to the system prompt's allowed tag list

## Roadmap

See [PLAN.md](PLAN.md) for the full architecture plan.

| Phase | Description | Status |
|---|---|---|
| 1 — Foundation | Tauri project, mic capture, config, logging | Done |
| 2 — STT | SenseVoice streaming transcription | Done |
| 3 — VAD | Silero VAD speech detection | Done |
| 4 — Ollama | Streaming REST client with conversation | Done |
| 5 — TTS | Piper TTS with sentence-level streaming | Done |
| 6 — UI | Live2D avatar, HUD, chatbox, transcripts | Done |
| 7 — Raspberry Pi | ARM64 build, ONNX optimization, INT8 | Planned |

## License

MIT
