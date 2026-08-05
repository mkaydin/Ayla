import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  encodeFrame,
  FrameDecoder,
  PROTOCOL_VERSION,
  SidecarProtocolError,
} from './sidecar-protocol.mjs';

const electronDirectory = dirname(fileURLToPath(import.meta.url));
const projectDirectory = dirname(electronDirectory);
const DEFAULT_TIMEOUT = 30_000;
const MAX_OUTSTANDING_REQUESTS = 64;
const LONG_COMMANDS = new Set([
  'analyze_and_merge_kb',
  'generate_tts',
  'ingest_text',
  'process_message',
  'process_turn',
  'run_consolidation',
  'run_ocr',
  'run_search',
  'stream_chat',
]);

function timeoutForRequest(method, params, requestedTimeout) {
  if (requestedTimeout != null) return requestedTimeout;
  if (LONG_COMMANDS.has(method)) {
    // Long extraction/OCR/search work must not be cancelled merely because a
    // slower machine needs more time. A finite safety deadline is opt-in.
    if (process.env.AYLA_DISABLE_LONG_REQUEST_TIMEOUT === '1') return null;
    const configured = Number(process.env.AYLA_LONG_REQUEST_TIMEOUT_MS);
    return Number.isFinite(configured) && configured > 0 ? configured : null;
  }
  return DEFAULT_TIMEOUT;
}

export class SidecarRequestError extends Error {
  constructor(error = {}) {
    super(String(error.message || 'Rust backend request failed'));
    this.name = 'SidecarRequestError';
    this.code = String(error.code || 'backend_error');
    this.retryable = Boolean(error.retryable);
    this.details = error.details;
  }
}

export function developmentSidecarPath(root = projectDirectory) {
  const executable = process.platform === 'win32' ? 'ayla-sidecar.exe' : 'ayla-sidecar';
  return join(root, 'target', 'debug', executable);
}

export function packagedSidecarPath(resourcesPath) {
  const executable = process.platform === 'win32' ? 'ayla-sidecar.exe' : 'ayla-sidecar';
  return join(resourcesPath, 'bin', executable);
}

export class SidecarManager extends EventEmitter {
  constructor({
    binaryPath = developmentSidecarPath(),
    args = [],
    cwd = projectDirectory,
    env = process.env,
    startupTimeout = 30_000,
    shutdownTimeout = 2_500,
    restartLimit = 3,
    restartDelays = [500, 1_500, 4_000],
    stableResetMs = 60_000,
    spawnProcess = spawn,
  } = {}) {
    super();
    this.binaryPath = resolve(binaryPath);
    this.args = [...args];
    this.cwd = resolve(cwd);
    this.env = { ...env };
    this.startupTimeout = startupTimeout;
    this.shutdownTimeout = shutdownTimeout;
    this.restartLimit = Math.max(0, restartLimit);
    this.restartDelays = restartDelays.length ? [...restartDelays] : [500];
    this.stableResetMs = Math.max(0, stableResetMs);
    this.spawnProcess = spawnProcess;
    this.child = null;
    this.decoder = null;
    this.pending = new Map();
    this.ownerEpochs = new Map();
    this.pings = new Map();
    this.methods = new Set();
    this.capabilities = new Set();
    this.stderr = '';
    this.writeChain = Promise.resolve();
    this.stopping = false;
    this.restartAttempts = 0;
    this.restartTimer = null;
    this.stableTimer = null;
    this.restartBlocked = false;
    this.status = {
      state: 'stopped',
      backendAvailable: false,
      protocolVersion: null,
      backendVersion: null,
      pid: null,
      error: null,
      restartAttempt: 0,
      nextRetryAt: null,
    };
  }

  snapshot() {
    return { ...this.status };
  }

  _setStatus(patch) {
    this.status = { ...this.status, ...patch };
    this.emit('status', this.snapshot());
  }

  async start() {
    if (this.status.state === 'ready') return this.snapshot();
    if (this.status.state === 'starting' && this.startPromise) return this.startPromise;
    if (!existsSync(this.binaryPath)) {
      const error = new Error(
        `Rust sidecar is missing at ${this.binaryPath}. Run npm run sidecar:build.`,
      );
      this._setStatus({ state: 'failed', backendAvailable: false, error: error.message });
      throw error;
    }

    this.stopping = false;
    if (this.restartTimer) {
      clearTimeout(this.restartTimer);
      this.restartTimer = null;
    }
    this.stderr = '';
    this.methods.clear();
    this.capabilities.clear();
    this._setStatus({
      state: 'starting',
      backendAvailable: false,
      protocolVersion: null,
      backendVersion: null,
      pid: null,
      error: null,
      restartAttempt: this.restartAttempts,
      nextRetryAt: null,
    });

    this.startPromise = new Promise((resolve, reject) => {
      let settled = false;
      const settle = (callback, value) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        callback(value);
      };
      const timer = setTimeout(() => {
        const error = new Error(
          `Timed out waiting for Rust sidecar handshake. ${this.stderr.trim()}`,
        );
        settle(reject, error);
        void this.stop();
      }, this.startupTimeout);

      try {
        const child = this.spawnProcess(this.binaryPath, this.args, {
          cwd: this.cwd,
          env: this.env,
          shell: false,
          stdio: ['pipe', 'pipe', 'pipe'],
        });
        this.child = child;
      } catch (error) {
        settle(reject, error);
        return;
      }

      this.decoder = new FrameDecoder(message => {
        try {
          if (message?.kind === 'hello' && this.status.state === 'starting') {
            this._acceptHello(message);
            settle(resolve, this.snapshot());
            return;
          }
          this._handleMessage(message);
        } catch (error) {
          this._protocolFailure(error);
          settle(reject, error);
        }
      });

      const child = this.child;
      child.stdout.on('data', chunk => {
        try {
          this.decoder.push(chunk);
        } catch (error) {
          this._protocolFailure(error);
          settle(reject, error);
        }
      });
      child.stderr.setEncoding('utf8');
      child.stderr.on('data', chunk => {
        this.stderr = `${this.stderr}${chunk}`.slice(-65_536);
        this.emit('stderr', chunk);
      });
      child.once('error', error => {
        settle(reject, error);
        this._handleExit(child, null, null, error);
      });
      child.once('exit', (code, signal) => {
        if (!settled) {
          settle(
            reject,
            new Error(
              `Rust sidecar exited before handshake (code ${code}, signal ${signal}): ${this.stderr.trim()}`,
            ),
          );
        }
        this._handleExit(child, code, signal);
      });
    });

    try {
      return await this.startPromise;
    } catch (error) {
      if (this.status.state !== 'restarting') {
        this._setStatus({ state: 'failed', backendAvailable: false, error: error.message });
      }
      throw error;
    } finally {
      this.startPromise = null;
    }
  }

  _acceptHello(message) {
    if (message.protocol_version !== PROTOCOL_VERSION) {
      throw new SidecarProtocolError(
        `Rust protocol ${message.protocol_version} is incompatible with Electron protocol ${PROTOCOL_VERSION}`,
        'protocol_version_mismatch',
      );
    }
    if (!Array.isArray(message.methods) || !Array.isArray(message.capabilities)) {
      throw new SidecarProtocolError('Rust handshake is missing methods or capabilities');
    }
    this.methods = new Set(message.methods);
    this.capabilities = new Set(message.capabilities);
    this._setStatus({
      state: 'ready',
      backendAvailable: true,
      protocolVersion: message.protocol_version,
      backendVersion: message.backend_version,
      pid: message.pid,
      error: null,
      restartAttempt: this.restartAttempts,
      nextRetryAt: null,
    });
    if (this.stableTimer) clearTimeout(this.stableTimer);
    this.stableTimer = setTimeout(() => {
      this.stableTimer = null;
      this.restartAttempts = 0;
      if (this.status.state === 'ready') {
        this._setStatus({ restartAttempt: 0, nextRetryAt: null });
      }
    }, this.stableResetMs);
    this.stableTimer.unref?.();
  }

  _handleMessage(message) {
    if (!message || typeof message.kind !== 'string') {
      throw new SidecarProtocolError('Rust sidecar emitted a message without a kind');
    }
    if (message.kind === 'response') {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.error) pending.reject(new SidecarRequestError(message.error));
      else pending.resolve(message.result);
      return;
    }
    if (message.kind === 'stream') {
      this.pending.get(message.request_id)?.onStream?.(message.event);
      return;
    }
    if (message.kind === 'event') {
      this.emit('backend-event', message);
      return;
    }
    if (message.kind === 'pong') {
      const ping = this.pings.get(message.id);
      if (ping) {
        this.pings.delete(message.id);
        clearTimeout(ping.timer);
        ping.resolve(true);
      }
      return;
    }
    if (message.kind === 'hello') {
      throw new SidecarProtocolError('Rust sidecar emitted a duplicate handshake');
    }
    throw new SidecarProtocolError(`Unknown Rust sidecar message '${message.kind}'`);
  }

  _protocolFailure(error) {
    if (['protocol_version_mismatch', 'invalid_messagepack', 'frame_too_large'].includes(error.code)) {
      this.restartBlocked = true;
    }
    this._setStatus({
      state: 'failed',
      backendAvailable: false,
      error: error.message,
    });
    this._rejectAll(error);
    if (this.child?.exitCode === null) this.child.kill();
  }

  _handleExit(child, code, signal, processError = null) {
    if (this.child !== child) return;
    const wasStopping = this.stopping;
    if (this.stableTimer) {
      clearTimeout(this.stableTimer);
      this.stableTimer = null;
    }
    try {
      this.decoder?.finish();
    } catch (error) {
      processError ||= error;
    }
    this.child = null;
    this.decoder = null;
    const error = processError || new Error(
      `Rust sidecar exited (code ${code ?? 'null'}, signal ${signal ?? 'none'})`,
    );
    this._rejectAll(error);
    this.emit('exit', { code, signal, expected: wasStopping });
    if (wasStopping) {
      this._setStatus({
        state: 'stopped',
        backendAvailable: false,
        pid: null,
        error: null,
        nextRetryAt: null,
      });
    } else if (this.restartBlocked) {
      this._setStatus({
        state: 'failed',
        backendAvailable: false,
        pid: null,
        error: error.message,
        nextRetryAt: null,
      });
    } else {
      this._scheduleRestart(error);
    }
  }

  _scheduleRestart(error) {
    if (this.stopping || this.restartBlocked) return;
    if (this.restartAttempts >= this.restartLimit) {
      this._setStatus({
        state: 'failed',
        backendAvailable: false,
        pid: null,
        error: `${error.message} Restart limit reached.`,
        restartAttempt: this.restartAttempts,
        nextRetryAt: null,
      });
      return;
    }
    const index = Math.min(this.restartAttempts, this.restartDelays.length - 1);
    const delay = Math.max(0, this.restartDelays[index]);
    this.restartAttempts += 1;
    const nextRetryAt = Date.now() + delay;
    this._setStatus({
      state: 'restarting',
      backendAvailable: false,
      pid: null,
      error: error.message,
      restartAttempt: this.restartAttempts,
      nextRetryAt,
    });
    this.restartTimer = setTimeout(() => {
      this.restartTimer = null;
      this.start().catch(startError => {
        if (!this.stopping && this.status.state === 'failed') {
          this._scheduleRestart(startError);
        }
      });
    }, delay);
    this.restartTimer.unref?.();
  }

  _rejectAll(error) {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    for (const ping of this.pings.values()) {
      clearTimeout(ping.timer);
      ping.reject(error);
    }
    this.pings.clear();
  }

  async _write(message) {
    const frame = encodeFrame(message);
    const operation = this.writeChain.then(() => new Promise((resolve, reject) => {
      const input = this.child?.stdin;
      if (!input || input.destroyed || !input.writable) {
        reject(new Error('Rust sidecar input pipe is unavailable'));
        return;
      }
      const onError = error => {
        input.off('drain', onDrain);
        reject(error);
      };
      const onDrain = () => {
        input.off('error', onError);
        resolve();
      };
      input.once('error', onError);
      if (input.write(frame)) {
        input.off('error', onError);
        resolve();
      } else {
        input.once('drain', onDrain);
      }
    }));
    this.writeChain = operation.catch(() => {});
    return operation;
  }

  request(method, params = {}, {
    timeout,
    onStream,
    ownerId = null,
    requestId = randomUUID(),
    ownerEpoch = ownerId === null ? null : (this.ownerEpochs.get(ownerId) || 0),
  } = {}) {
    if (ownerId !== null && ownerEpoch !== (this.ownerEpochs.get(ownerId) || 0)) {
      return Promise.reject(new SidecarRequestError({
        code: 'request_cancelled',
        message: 'Renderer is no longer available',
        retryable: true,
      }));
    }
    if (this.status.state === 'starting' && this.startPromise) {
      return this.startPromise.then(() => this.request(method, params, {
        timeout,
        onStream,
        ownerId,
        requestId,
        ownerEpoch,
      }));
    }
    if (this.status.state !== 'ready') {
      return Promise.reject(new Error(`Rust backend is ${this.status.state}`));
    }
    if (!this.methods.has(method)) {
      return Promise.reject(new Error(`Rust backend does not advertise '${method}'`));
    }
    if (this.pending.size >= MAX_OUTSTANDING_REQUESTS) {
      return Promise.reject(new Error('Too many outstanding Rust backend requests'));
    }

    const id = requestId;
    if (typeof id !== 'string' || !/^[a-f0-9-]{16,64}$/i.test(id)) {
      return Promise.reject(new TypeError('Invalid Rust backend request identifier'));
    }
    if (this.pending.has(id)) {
      return Promise.reject(new Error(`Rust backend request '${id}' already exists`));
    }
    const timeoutMs = timeoutForRequest(method, params, timeout);
    return new Promise((resolve, reject) => {
      const timer = timeoutMs == null ? null : setTimeout(() => {
        this._cancelPending(
          id,
          new Error(`Rust command '${method}' timed out after ${timeoutMs} ms`),
          true,
        );
      }, timeoutMs);
      this.pending.set(id, {
        resolve,
        reject,
        timer,
        onStream,
        ownerId,
        method,
      });
      this._write({ kind: 'request', id, method, params }).catch(error => {
        const pending = this.pending.get(id);
        if (!pending) return;
        this.pending.delete(id);
        clearTimeout(pending.timer);
        pending.reject(error);
      });
    });
  }

  _cancelPending(requestId, error, notifyBackend) {
    const pending = this.pending.get(requestId);
    if (!pending) return false;
    this.pending.delete(requestId);
    clearTimeout(pending.timer);
    pending.reject(error);
    if (notifyBackend && ['ready', 'starting'].includes(this.status.state)) {
      void this._write({ kind: 'cancel_request', request_id: requestId }).catch(() => {});
    }
    return true;
  }

  cancelRequest(requestId, reason = 'Rust backend request cancelled', ownerId = null) {
    const pending = this.pending.get(requestId);
    if (!pending || (ownerId !== null && pending.ownerId !== ownerId)) return false;
    const error = new SidecarRequestError({
      code: 'request_cancelled',
      message: reason,
      retryable: true,
    });
    return this._cancelPending(requestId, error, true);
  }

  cancelOwner(ownerId, reason = 'Renderer was destroyed') {
    this.ownerEpochs.set(ownerId, (this.ownerEpochs.get(ownerId) || 0) + 1);
    let cancelled = 0;
    for (const [requestId, pending] of [...this.pending]) {
      // Long-running extraction/OCR/search requests are persistence jobs. Keep
      // them alive if the renderer reloads or a panel is destroyed so completed
      // work is still written to the knowledge base.
      if (LONG_COMMANDS.has(pending.method)) continue;
      if (pending.ownerId === ownerId && this.cancelRequest(requestId, reason, ownerId)) {
        cancelled += 1;
      }
    }
    return cancelled;
  }

  ping(timeout = 5_000) {
    if (this.status.state === 'starting' && this.startPromise) {
      return this.startPromise.then(() => this.ping(timeout));
    }
    if (this.status.state !== 'ready') {
      return Promise.reject(new Error(`Rust backend is ${this.status.state}`));
    }
    const id = randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pings.delete(id);
        reject(new Error('Rust sidecar ping timed out'));
      }, timeout);
      this.pings.set(id, { resolve, reject, timer });
      this._write({ kind: 'ping', id }).catch(error => {
        const ping = this.pings.get(id);
        if (!ping) return;
        this.pings.delete(id);
        clearTimeout(ping.timer);
        ping.reject(error);
      });
    });
  }

  async stop() {
    if (this.restartTimer) {
      clearTimeout(this.restartTimer);
      this.restartTimer = null;
    }
    if (this.stableTimer) {
      clearTimeout(this.stableTimer);
      this.stableTimer = null;
    }
    if (!this.child) {
      this.stopping = true;
      this._rejectAll(new Error('Rust sidecar stopped'));
      this._setStatus({
        state: 'stopped',
        backendAvailable: false,
        pid: null,
        error: null,
        nextRetryAt: null,
      });
      return;
    }
    this.stopping = true;
    this._setStatus({ state: 'stopping', backendAvailable: false });
    const child = this.child;
    const exited = new Promise(resolve => child.once('exit', resolve));
    try {
      await this._write({ kind: 'shutdown' });
      child.stdin.end();
    } catch {
      // The exit path below still terminates and cleans pending requests.
    }
    const clean = await Promise.race([
      exited.then(() => true),
      new Promise(resolve => setTimeout(() => resolve(false), this.shutdownTimeout)),
    ]);
    if (clean || child.exitCode !== null) return;
    child.kill('SIGTERM');
    const terminated = await Promise.race([
      exited.then(() => true),
      new Promise(resolve => setTimeout(() => resolve(false), 1_000)),
    ]);
    if (!terminated && child.exitCode === null) child.kill('SIGKILL');
  }

  async restart() {
    this.restartBlocked = false;
    this.restartAttempts = 0;
    await this.stop();
    this.stopping = false;
    return this.start();
  }
}
