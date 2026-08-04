// Web Audio TTS player with queue management and mouth animation.
// Receives WAV bytes (Uint8Array → ArrayBuffer → AudioBuffer) from the shared
// desktop backend adapter and plays them sequentially.
// Exposes `getMouthLevel()` for Live2D mouth sync.
// Supports mute via gain node.

export class TtsPlayer {
  constructor(coreModel) {
    this.core = coreModel;
    this.ctx = null;
    this.analyser = null;
    this.gainNode = null;
    this.source = null;
    this.queue = [];
    this.playing = false;
    this.mouthData = null;
    this._muted = false;
  }

  async init() {
    this.ctx = new (window.AudioContext || window.webkitAudioContext)();
    this.analyser = this.ctx.createAnalyser();
    this.analyser.fftSize = 256;
    this.analyser.smoothingTimeConstant = 0.3;

    this.gainNode = this.ctx.createGain();
    this.gainNode.gain.value = this._muted ? 0 : 1;

    this.analyser.connect(this.gainNode);
    this.gainNode.connect(this.ctx.destination);

    this.mouthData = new Uint8Array(this.analyser.frequencyBinCount);
    if (this.ctx.state === 'suspended') {
      const resume = () => { this.ctx.resume(); };
      document.addEventListener('click', resume, { once: true });
      document.addEventListener('keydown', resume, { once: true });
    }
  }

  setMuted(muted) {
    if (this.gainNode) this.gainNode.gain.value = muted ? 0 : 1;
    this._muted = muted;
  }

  isMuted() { return this._muted; }

  enqueue(wavBytes) {
    if (!this.ctx) return;
    this.queue.push(new Uint8Array(wavBytes));
    if (!this.playing) this._playNext();
  }

  stop() {
    this.queue.length = 0;
    this.playing = false;
    try { this.source?.stop(); } catch (e) {}
    this.source = null;
  }

  isActive() { return this.playing; }

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

  async _playNext() {
    if (this.queue.length === 0) { this.playing = false; return; }
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
