import { decode, encode } from '@msgpack/msgpack';

export const PROTOCOL_VERSION = 1;
export const MAX_FRAME_SIZE = 64 * 1024 * 1024;

export class SidecarProtocolError extends Error {
  constructor(message, code = 'protocol_error') {
    super(message);
    this.name = 'SidecarProtocolError';
    this.code = code;
  }
}

export function encodeFrame(message) {
  const payload = Buffer.from(encode(message));
  if (payload.length > MAX_FRAME_SIZE) {
    throw new SidecarProtocolError(
      `MessagePack frame is ${payload.length} bytes; maximum is ${MAX_FRAME_SIZE}`,
      'frame_too_large',
    );
  }
  const header = Buffer.allocUnsafe(4);
  header.writeUInt32BE(payload.length);
  return Buffer.concat([header, payload]);
}

export class FrameDecoder {
  constructor(onMessage) {
    if (typeof onMessage !== 'function') {
      throw new TypeError('FrameDecoder requires an onMessage callback');
    }
    this.onMessage = onMessage;
    this.buffer = Buffer.alloc(0);
  }

  push(chunk) {
    if (!chunk?.length) return;
    this.buffer = this.buffer.length
      ? Buffer.concat([this.buffer, chunk])
      : Buffer.from(chunk);

    while (this.buffer.length >= 4) {
      const frameSize = this.buffer.readUInt32BE(0);
      if (frameSize > MAX_FRAME_SIZE) {
        throw new SidecarProtocolError(
          `Sidecar frame is ${frameSize} bytes; maximum is ${MAX_FRAME_SIZE}`,
          'frame_too_large',
        );
      }
      if (this.buffer.length < frameSize + 4) return;

      const payload = this.buffer.subarray(4, frameSize + 4);
      this.buffer = this.buffer.subarray(frameSize + 4);
      try {
        this.onMessage(decode(payload));
      } catch (error) {
        throw new SidecarProtocolError(
          `Cannot decode sidecar MessagePack frame: ${error.message}`,
          'invalid_messagepack',
        );
      }
    }
  }

  finish() {
    if (this.buffer.length !== 0) {
      throw new SidecarProtocolError(
        `Sidecar closed with ${this.buffer.length} bytes of an incomplete frame`,
        'incomplete_frame',
      );
    }
  }
}
