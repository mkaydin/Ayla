import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import {
  SidecarManager,
  SidecarRequestError,
} from '../electron/sidecar-manager.mjs';
import { encodeFrame, FrameDecoder } from '../electron/sidecar-protocol.mjs';

class FakeSidecar extends EventEmitter {
  constructor({ crash = false } = {}) {
    super();
    this.stdin = new PassThrough();
    this.stdout = new PassThrough();
    this.stderr = new PassThrough();
    this.exitCode = null;
    this.killed = false;

    const decoder = new FrameDecoder(message => this.handleMessage(message));
    this.stdin.on('data', chunk => decoder.push(chunk));
    queueMicrotask(() => {
      this.send({
        kind: 'hello',
        protocol_version: 1,
        backend_version: 'fixture',
        pid: 1234,
        methods: ['check_db_ready', 'hang'],
        capabilities: ['messagepack', 'cancellation'],
      });
      if (crash) setTimeout(() => this.exit(17), 5);
    });
  }

  send(message) {
    this.stdout.write(encodeFrame(message));
  }

  handleMessage(message) {
    if (message.kind === 'ping') {
      this.send({ kind: 'pong', id: message.id });
    } else if (message.kind === 'request' && message.method !== 'hang') {
      this.send({ kind: 'response', id: message.id, result: false });
    } else if (message.kind === 'cancel_request') {
      this.send({
        kind: 'event',
        name: 'cancellation-observed',
        operation_id: message.request_id,
        payload: { request_id: message.request_id },
      });
    } else if (message.kind === 'shutdown') {
      queueMicrotask(() => this.exit(0));
    }
  }

  exit(code, signal = null) {
    if (this.killed || this.exitCode !== null) return;
    this.exitCode = code;
    this.emit('exit', code, signal);
  }

  kill(signal = 'SIGTERM') {
    if (this.exitCode !== null) return false;
    this.killed = true;
    this.emit('exit', null, signal);
    return true;
  }
}

function createManager(spawnProcess, extra = {}) {
  return new SidecarManager({
    binaryPath: process.execPath,
    cwd: process.cwd(),
    spawnProcess,
    startupTimeout: 200,
    shutdownTimeout: 50,
    restartDelays: [5, 10, 15],
    stableResetMs: 1_000,
    ...extra,
  });
}

function waitForStatus(manager, predicate, timeout = 500) {
  const current = manager.snapshot();
  if (predicate(current)) return Promise.resolve(current);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      manager.off('status', listener);
      reject(new Error(`Timed out waiting for sidecar status; current=${manager.snapshot().state}`));
    }, timeout);
    const listener = status => {
      if (!predicate(status)) return;
      clearTimeout(timer);
      manager.off('status', listener);
      resolve(status);
    };
    manager.on('status', listener);
  });
}

test('SidecarManager restarts once after an unexpected crash without replaying requests', async () => {
  let starts = 0;
  const manager = createManager(() => new FakeSidecar({ crash: starts++ === 0 }));
  const states = [];
  manager.on('status', status => states.push(status.state));

  try {
    await manager.start();
    await waitForStatus(
      manager,
      status => status.state === 'ready' && status.restartAttempt === 1,
    );
    assert.equal(starts, 2);
    assert.ok(states.includes('restarting'));
    assert.equal(await manager.request('check_db_ready'), false);
  } finally {
    await manager.stop();
  }
});

test('SidecarManager stops after the configured crash restart limit', async () => {
  let starts = 0;
  const manager = createManager(
    () => {
      starts += 1;
      return new FakeSidecar({ crash: true });
    },
    { restartLimit: 2, restartDelays: [5, 5] },
  );
  try {
    await manager.start();
    const failed = await waitForStatus(
      manager,
      status => status.state === 'failed' && status.restartAttempt === 2,
    );
    assert.equal(starts, 3);
    assert.match(failed.error, /Restart limit reached/);
  } finally {
    await manager.stop();
  }
});

test('renderer owner cancellation rejects local work and notifies the sidecar', async () => {
  const manager = createManager(() => new FakeSidecar());
  const requestId = '11111111-1111-4111-8111-111111111111';
  const observed = new Promise(resolve => {
    manager.on('backend-event', message => {
      if (message.name === 'cancellation-observed') resolve(message);
    });
  });

  try {
    await manager.start();
    const request = manager.request('hang', {}, {
      ownerId: 42,
      requestId,
      timeout: 200,
    });
    assert.equal(manager.cancelOwner(42), 1);
    await assert.rejects(
      request,
      error => error instanceof SidecarRequestError && error.code === 'request_cancelled',
    );
    assert.equal((await observed).operation_id, requestId);
    assert.equal(manager.pending.size, 0);
  } finally {
    await manager.stop();
  }
});

test('request cancellation is restricted to the renderer that owns it', async () => {
  const manager = createManager(() => new FakeSidecar());
  const requestId = '22222222-2222-4222-8222-222222222222';
  try {
    await manager.start();
    const request = manager.request('hang', {}, {
      ownerId: 7,
      requestId,
      timeout: 200,
    });
    assert.equal(manager.cancelRequest(requestId, 'wrong owner', 8), false);
    assert.equal(manager.pending.has(requestId), true);
    assert.equal(manager.cancelRequest(requestId, 'right owner', 7), true);
    await assert.rejects(
      request,
      error => error instanceof SidecarRequestError && error.code === 'request_cancelled',
    );
  } finally {
    await manager.stop();
  }
});
