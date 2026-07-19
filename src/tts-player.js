// Web Audio TTS player with queue management and mouth animation.
//
// Receives WAV bytes (Uint8Array → ArrayBuffer → AudioBuffer) from the
// Tauri `generate_tts` Rust command and plays them back sequentially.
// Exposes `getMouthLevel()` so the main render loop can drive the Live2D
// mouth parameter in sync with audio.

export class TtsPlayer {
  constructor(coreModel) {
    this.core = coreModel;
    this.ctx = null;
    this.analyser = null;
    this.source = null;
    this.queue = [];
    this.playing = false;
    this.mouthData = null;
  }

  async init() {
    this.ctx = new (window.AudioContext || window.webkitAudioContext)();
    this.analyser = this.ctx.createAnalyser();
    this.analyser.fftSize = 256;
    this.analyser.smoothingTimeConstant = 0.3;
    this.analyser.connect(this.ctx.destination);
    this.mouthData = new Uint8Array(this.analyser.frequencyBinCount);
    if (this.ctx.state === 'suspended') {
      const resume = () => { this.ctx.resume(); };
      document.addEventListener('click', resume, { once: true });
      document.addEventListener('keydown', resume, { once: true });
    }
  }

  enqueue(wavBytes) {
    if (!this.ctx) return;
    this.queue.push(new Uint8Array(wavBytes));
    if (!this.playing) this._playNext();
  }

  stop() {
    this.queue.length = 0;
    this.playing = false;
    try { this.source?.stop(); } catch(e) {}
    this.source = null;
  }

  /** Whether audio is actively playing. */
  isActive() {
    return this.playing;
  }

  /** Read current audio amplitude (0–1). Call from the render loop. */
  getMouthLevel() {
    if (!this.analyser || !this.mouthData) return 0;
    this.analyser.getByteTimeDomainData(this.mouthData);
    let sum = 0;
    for (let i = 0; i < this.mouthData.length; i++) {
      const v = (this.mouthData[i] - 128) / 128;
      sum += v * v;
    }
    return Math.sqrt(sum / this.mouthData.length);
  }

  // ---- internals --------------------------------------------------------

  async _playNext() {
    if (this.queue.length === 0) {
      this.playing = false;
      return;
    }
    this.playing = true;
    const bytes = this.queue.shift();

    try {
      if (this.ctx.state === 'suspended') await this.ctx.resume();
      const buf = await this.ctx.decodeAudioData(
        bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
      this.source = this.ctx.createBufferSource();
      this.source.buffer = buf;
      this.source.connect(this.analyser);
      this.source.onended = () => this._playNext();
      this.source.start();
    } catch (e) {
      console.warn('TTS decode failed', e);
      this._playNext();
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const SENTENCE_RE = /[^.!?\n]+[.!?\n]+(?:\s|$)*/g;

/** Split text into sentences, returning the portion since last call. */
export function extractSentences(fullText, fromIdx) {
  const tail = fullText.slice(fromIdx);
  const matches = tail.match(SENTENCE_RE);
  if (!matches) return [];
  const parts = [];
  let lastEnd = 0;
  for (const m of matches) {
    parts.push(m);
    lastEnd = tail.indexOf(m) + m.length;
  }
  // If there's a hanging fragment at the end, don't include it yet
  const covered = parts.join('');
  if (tail.length > covered.length) {
    // trailing partial sentence stays in the buffer
  }
  return parts.length ? parts : [];
}
