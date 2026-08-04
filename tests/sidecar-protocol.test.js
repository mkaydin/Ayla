import assert from 'node:assert/strict';
import test from 'node:test';
import { decode } from '@msgpack/msgpack';
import {
  encodeFrame,
  FrameDecoder,
  MAX_FRAME_SIZE,
  SidecarProtocolError,
} from '../electron/sidecar-protocol.mjs';

test('sidecar frame decoder handles fragmented and coalesced messages', () => {
  const messages = [];
  const decoder = new FrameDecoder(message => messages.push(message));
  const first = encodeFrame({ kind: 'pong', id: 'one' });
  const second = encodeFrame({ kind: 'pong', id: 'two' });
  const combined = Buffer.concat([first, second]);

  decoder.push(combined.subarray(0, 2));
  decoder.push(combined.subarray(2, 11));
  decoder.push(combined.subarray(11));
  decoder.finish();

  assert.deepEqual(messages, [
    { kind: 'pong', id: 'one' },
    { kind: 'pong', id: 'two' },
  ]);
});

test('sidecar frame encoder emits a big-endian MessagePack frame', () => {
  const frame = encodeFrame({ kind: 'shutdown' });
  const size = frame.readUInt32BE(0);
  assert.equal(size, frame.length - 4);
  assert.deepEqual(decode(frame.subarray(4)), { kind: 'shutdown' });
});

test('sidecar decoder rejects oversized frames before receiving a payload', () => {
  const decoder = new FrameDecoder(() => {});
  const header = Buffer.alloc(4);
  header.writeUInt32BE(MAX_FRAME_SIZE + 1);
  assert.throws(
    () => decoder.push(header),
    error => error instanceof SidecarProtocolError && error.code === 'frame_too_large',
  );
});

test('sidecar decoder rejects incomplete shutdown frames', () => {
  const decoder = new FrameDecoder(() => {});
  decoder.push(encodeFrame({ kind: 'ping', id: 'partial' }).subarray(0, 6));
  assert.throws(
    () => decoder.finish(),
    error => error instanceof SidecarProtocolError && error.code === 'incomplete_frame',
  );
});
