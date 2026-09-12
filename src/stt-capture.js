import { invoke } from './lib/ipc.js';

const TARGET_SAMPLE_RATE = 16_000;
const CHUNK_SECONDS = 0.1;
const MAX_QUEUED_CHUNKS = 4;
export const MAX_DEFERRED_PCM_BYTES = TARGET_SAMPLE_RATE * 2 * 60;

export class SttCapture {
  constructor({
    deferred = false,
    onState = () => {},
    onSession = () => {},
    onLevel = () => {},
    onError = () => {},
  } = {}) {
    this.deferred = deferred;
    this.onState = onState;
    this.onSession = onSession;
    this.onLevel = onLevel;
    this.onError = onError;
    this.sessionId = null;
    this.recording = false;
    this.stream = null;
    this.context = null;
    this.source = null;
    this.node = null;
    this.silentGain = null;
    this.input = [];
    this.inputSamples = 0;
    this.queuedChunks = 0;
    this.sendChain = Promise.resolve();
    this.stopping = false;
    this.recordedPcm = [];
    this.recordedBytes = 0;
  }

  get active() { return this.recording || Boolean(this.sessionId) || this.stopping; }

  async start() {
    if (this.active) return this.sessionId;
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error('Microphone capture is unavailable in this desktop runtime');
    }
    this.onState('starting');
    try {
      this.stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
        video: false,
      });
      if (!this.deferred) await this._startBackendSession();

      this.context = new (window.AudioContext || window.webkitAudioContext)();
      await this.context.audioWorklet.addModule('/audio-capture-worklet.js');
      this.source = this.context.createMediaStreamSource(this.stream);
      this.node = new AudioWorkletNode(this.context, 'ayla-audio-capture');
      this.silentGain = this.context.createGain();
      this.silentGain.gain.value = 0;
      this.node.port.onmessage = event => this._receive(event.data);
      this.source.connect(this.node);
      this.node.connect(this.silentGain);
      this.silentGain.connect(this.context.destination);
      this.recording = true;
      this.onState(this.deferred ? 'recording' : 'listening');
      return this.sessionId;
    } catch (error) {
      await this._releaseCapture();
      this.sessionId = null;
      this.recording = false;
      this.onState('idle');
      this.onError(error);
      throw error;
    }
  }

  async stop() {
    if (this.stopping || !this.active) return;
    this.stopping = true;
    this.onState(this.deferred ? 'processing' : 'stopping');
    try {
      this._flushInput();
      this.recording = false;
      await this.sendChain.catch(() => {});
      await this._releaseCapture();
      if (this.deferred && this.recordedBytes) {
        await this._startBackendSession();
        for (const pcm of this.recordedPcm) {
          await invoke('stt_push_audio', {
            sessionId: this.sessionId,
            pcm: Array.from(pcm),
          });
        }
      }
      if (this.sessionId) await invoke('stop_stt', { sessionId: this.sessionId });
    } catch (error) {
      if (this.sessionId) {
        await invoke('stop_stt', { sessionId: this.sessionId }).catch(() => {});
      }
      this.onError(error);
      throw error;
    } finally {
      this.sessionId = null;
      this.recording = false;
      this.stopping = false;
      this.input = [];
      this.inputSamples = 0;
      this.recordedPcm = [];
      this.recordedBytes = 0;
      this.onState('idle');
    }
  }

  _receive(value) {
    if (!this.recording || this.stopping || !(value instanceof Float32Array)) return;
    this.onLevel(measureLevel(value));
    this.input.push(value);
    this.inputSamples += value.length;
    const target = Math.max(1, Math.round(this.context.sampleRate * CHUNK_SECONDS));
    if (this.inputSamples < target) return;

    const combined = new Float32Array(this.inputSamples);
    let offset = 0;
    for (const chunk of this.input) {
      combined.set(chunk, offset);
      offset += chunk.length;
    }
    this.input = [];
    this.inputSamples = 0;
    this._queuePcm(floatToPcm16(resampleLinear(
      combined,
      this.context.sampleRate,
      TARGET_SAMPLE_RATE,
    )));
  }

  _flushInput() {
    if (!this.inputSamples || !this.context) return;
    const combined = new Float32Array(this.inputSamples);
    let offset = 0;
    for (const chunk of this.input) {
      combined.set(chunk, offset);
      offset += chunk.length;
    }
    this.input = [];
    this.inputSamples = 0;
    this._queuePcm(floatToPcm16(resampleLinear(
      combined,
      this.context.sampleRate,
      TARGET_SAMPLE_RATE,
    )));
  }

  _queuePcm(pcm) {
    if (!pcm.length) return;
    if (this.deferred) {
      try {
        this.recordedBytes = appendDeferredPcm(
          this.recordedPcm,
          this.recordedBytes,
          pcm,
        );
      } catch (error) {
        this.onError(error);
        void this.stop().catch(() => {});
      }
      return;
    }
    if (!this.sessionId) return;
    if (this.queuedChunks >= MAX_QUEUED_CHUNKS) {
      this.onError(new Error('STT audio processing is overloaded; an audio chunk was dropped'));
      return;
    }
    const sessionId = this.sessionId;
    this.queuedChunks++;
    this.sendChain = this.sendChain
      .catch(() => {})
      .then(() => invoke('stt_push_audio', {
        sessionId,
        pcm: Array.from(pcm),
      }))
      .catch(error => this.onError(error))
      .finally(() => { this.queuedChunks--; });
  }

  async _startBackendSession() {
    const result = await invoke('start_stt');
    this.sessionId = result?.sessionId;
    if (!this.sessionId) throw new Error('STT backend did not return a session ID');
    this.onSession(this.sessionId);
  }

  async _releaseCapture() {
    try { this.source?.disconnect(); } catch {}
    try { this.node?.disconnect(); } catch {}
    try { this.silentGain?.disconnect(); } catch {}
    for (const track of this.stream?.getTracks?.() || []) track.stop();
    if (this.context && this.context.state !== 'closed') {
      await this.context.close().catch(() => {});
    }
    this.stream = null;
    this.context = null;
    this.source = null;
    this.node = null;
    this.silentGain = null;
  }
}

export function appendDeferredPcm(chunks, currentBytes, pcm, maxBytes = MAX_DEFERRED_PCM_BYTES) {
  if (!(pcm instanceof Uint8Array)) throw new TypeError('Expected Uint8Array PCM audio');
  if (currentBytes + pcm.byteLength > maxBytes) {
    throw new RangeError('Chat voice recording reached the 60 second limit and will be transcribed');
  }
  chunks.push(pcm);
  return currentBytes + pcm.byteLength;
}

export function measureLevel(input) {
  if (!(input instanceof Float32Array) || !input.length) return { rms: 0, peak: 0 };
  let sum = 0;
  let peak = 0;
  for (const sample of input) {
    const absolute = Math.abs(sample);
    sum += sample * sample;
    peak = Math.max(peak, absolute);
  }
  return { rms: Math.sqrt(sum / input.length), peak };
}

export function resampleLinear(input, sourceRate, targetRate = TARGET_SAMPLE_RATE) {
  if (!(input instanceof Float32Array)) throw new TypeError('Expected Float32Array audio');
  if (!input.length) return new Float32Array();
  if (!(sourceRate > 0) || !(targetRate > 0)) throw new RangeError('Invalid sample rate');
  if (sourceRate === targetRate) return input.slice();
  const outputLength = Math.max(1, Math.floor(input.length * targetRate / sourceRate));
  const output = new Float32Array(outputLength);
  const ratio = sourceRate / targetRate;
  for (let index = 0; index < outputLength; index++) {
    const position = index * ratio;
    const left = Math.floor(position);
    const right = Math.min(left + 1, input.length - 1);
    const fraction = position - left;
    output[index] = input[left] * (1 - fraction) + input[right] * fraction;
  }
  return output;
}

export function floatToPcm16(input) {
  const bytes = new Uint8Array(input.length * 2);
  const view = new DataView(bytes.buffer);
  for (let index = 0; index < input.length; index++) {
    const sample = Math.max(-1, Math.min(1, input[index]));
    view.setInt16(index * 2, sample < 0 ? sample * 32768 : sample * 32767, true);
  }
  return bytes;
}
