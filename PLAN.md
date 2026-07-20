# Local Voice Assistant Architecture Plan

**Target Platform:** Tauri Desktop Application (Rust + React/TypeScript)

## Goal

Build a fully local voice assistant that uses:

* **Sherpa-ONNX Whisper Base** for Speech-to-Text
* **Sherpa-ONNX Silero VAD** for Voice Activity Detection
* **Ollama** as the local LLM backend
* **Sherpa-ONNX TTS (Piper model)** for speech synthesis
* **Rust** as the core backend
* **Tauri** for the desktop UI

The architecture should be modular so it can later be deployed on a **Raspberry Pi 5** with minimal code changes.

---

# High-Level Architecture

```text
                    Tauri Desktop Application

            ┌────────────────────────────────────┐
            │      React + TypeScript UI         │
            └────────────────────────────────────┘
                          ▲
                          │ Events
                          ▼
            ┌────────────────────────────────────┐
            │          Rust Backend              │
            ├────────────────────────────────────┤
            │ Audio Service                      │
            │ Voice Activity Detection Service   │
            │ Speech-to-Text Service             │
            │ Conversation Manager              │
            │ Ollama Client                     │
            │ Text-to-Speech Service            │
            │ Configuration Manager             │
            └────────────────────────────────────┘
                          │
        ┌─────────────────┼──────────────────┐
        ▼                 ▼                  ▼
 Sherpa-ONNX         Ollama REST API     Sherpa-ONNX
 Whisper Base            Local LLM           TTS
```

---

# Project Structure

```text
src-tauri/
│
├── src/
│
├── audio/
│   ├── microphone.rs
│   ├── recorder.rs
│   ├── audio_buffer.rs
│   └── player.rs
│
├── vad/
│   └── silero.rs
│
├── stt/
│   ├── whisper.rs
│   ├── recognizer.rs
│   └── streaming.rs
│
├── llm/
│   ├── ollama.rs
│   ├── streaming.rs
│   └── conversation.rs
│
├── tts/
│   ├── sherpa.rs
│   └── player.rs
│
├── services/
│   ├── conversation_manager.rs
│   ├── model_manager.rs
│   └── settings.rs
│
├── commands/
│   ├── start_listening.rs
│   ├── stop_listening.rs
│   ├── send_message.rs
│   └── settings.rs
│
├── events/
│
├── models/
│
└── main.rs

resources/

    whisper-base/

    silero-vad/

    tts/

    configs/
```

---

# Core Components

## 1. Audio Service

### Responsibilities

* Capture microphone audio
* Produce 16 kHz PCM audio
* Maintain a ring buffer
* Stream audio to the VAD

Suggested Rust crates:

* cpal
* ringbuf

Output:

```
Microphone
      ↓
PCM Audio Stream
```

---

## 2. Voice Activity Detection (VAD)

Use Sherpa-ONNX's Silero VAD.

Responsibilities:

* Detect speech
* Ignore silence
* Segment speech
* Reduce unnecessary STT inference

Pipeline:

```
PCM Audio

↓

Silero VAD

↓

Speech?

 ├── No → discard

 └── Yes → STT
```

---

## 3. Speech-to-Text Service

Use Sherpa-ONNX Rust bindings with Whisper Base.

Responsibilities:

* Load Whisper Base model
* Accept streaming audio
* Produce partial transcription
* Produce final transcription

Pipeline:

```
Audio

↓

Recognizer

↓

Partial Result

↓

Partial Result

↓

Final Result
```

The frontend should display partial text in real time.

---

## 4. Conversation Manager

This service orchestrates the entire pipeline.

Responsibilities:

* Receive STT output
* Maintain conversation history
* Send prompts to Ollama
* Forward LLM output to TTS

Pipeline:

```
Speech

↓

Whisper

↓

Text

↓

Conversation Manager

↓

Ollama

↓

LLM Response

↓

TTS
```

---

# Conversation State Machine

```text
Idle

↓

Listening

↓

Recognizing

↓

Thinking

↓

Speaking

↓

Idle
```

Only one state should be active at a time.

---

## 5. Ollama Client

Use the streaming REST API.

```
POST /api/chat
```

Enable streaming.

Receive tokens incrementally.

Example:

```
H

He

Hel

Hello

Hello,

Hello, how

...
```

Display tokens immediately in the UI.

Do not wait for the full response.

---

## 6. Text-to-Speech Service

Use Sherpa-ONNX TTS.

Instead of waiting for the complete LLM response:

```
LLM Tokens

↓

Sentence Splitter

↓

Sherpa TTS

↓

Audio Queue

↓

Speaker
```

As soon as a sentence is complete, start generating speech.

This significantly reduces perceived latency.

---

# Internal Event System

Recommended events:

```
MicrophoneStarted

MicrophoneStopped

SpeechDetected

SpeechEnded

PartialTranscript

FinalTranscript

LLMStarted

LLMToken

LLMFinished

SpeechStarted

SpeechFinished

ErrorOccurred
```

These events should be emitted to the Tauri frontend.

---

# Thread / Task Architecture

Each component should run independently.

```
Audio Task

↓

VAD Task

↓

STT Task

↓

Conversation Task

↓

LLM Task

↓

TTS Task
```

Communication:

```
tokio::mpsc
```

Avoid blocking operations.

---

# Data Flow

```
Microphone

↓

Audio Buffer

↓

Silero VAD

↓

Speech Segment

↓

Whisper Base

↓

Partial Transcript

↓

Frontend

↓

Final Transcript

↓

Conversation Manager

↓

Ollama

↓

Streaming Tokens

↓

Frontend

↓

Sentence Splitter

↓

Sherpa TTS

↓

Speaker
```

---

# Model Management

Store every model under a dedicated resources directory.

```
resources/

    whisper-base/

    silero-vad/

    tts/

    configs/
```

Example configuration:

```toml
[stt]
model = "resources/whisper-base"

[vad]
model = "resources/silero-vad"

[tts]
model = "resources/tts"

[ollama]
url = "http://localhost:11434"
model = "qwen3:4b"
```

---

# Development Roadmap

## Phase 1 — Project Foundation

* Create the Tauri project
* Set up the Rust backend architecture
* Implement microphone capture
* Add configuration management
* Add logging

---

## Phase 2 — Speech-to-Text

* Integrate Sherpa-ONNX Rust API
* Load Whisper Base
* Stream microphone audio
* Produce partial and final transcripts

---

## Phase 3 — Voice Activity Detection

* Integrate Silero VAD
* Detect speech boundaries
* Ignore silence
* Optimize CPU usage

---

## Phase 4 — Ollama Integration

* Implement the streaming REST client
* Support conversation history
* Stream tokens to the UI

---

## Phase 5 — Text-to-Speech

* Integrate Sherpa-ONNX TTS
* Generate streaming speech
* Play synthesized audio

---

## Phase 6 — User Interface

Features:

* Live microphone indicator
* Partial transcription
* Final transcription
* Streaming LLM response
* Conversation history
* Model selection
* Audio settings
* Device settings

---

## Phase 7 — Raspberry Pi Optimization

* Build for ARM64
* Optimize ONNX Runtime
* Tune CPU thread count
* Reduce model loading time
* Use INT8 models if necessary
* Benchmark latency

---

# Future Extensions

The modular architecture should allow the following features to be added without major refactoring:

* Wake Word Detection (Sherpa Keyword Spotting)
* Speaker Identification
* Speaker Verification
* Retrieval-Augmented Generation (RAG)
* Local memory database
* Vision module (camera integration)
* MQTT / Home Assistant integration
* ESP32 peripheral support
* Bluetooth media controls
* Multi-language STT
* Multi-language TTS
* Emotion recognition
* Offline translation
* Raspberry Pi standalone deployment

---

# Design Principles

* **Fully offline** (except optional external APIs)
* **Modular architecture**
* **Streaming-first design**
* **Low latency**
* **Cross-platform (Linux, Windows, Raspberry Pi)**
* **Rust-centric implementation**
* **Minimal UI/backend coupling**
* **Easy model replacement**
* **Production-ready service separation**
* **Scalable for future AI assistant capabilities**
