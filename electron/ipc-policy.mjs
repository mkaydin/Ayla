const DEFAULT_MAX_BACKEND_PAYLOAD = 60 * 1024 * 1024;
const REQUEST_ID_PATTERN = /^[a-f0-9-]{16,64}$/i;
const NOTIFICATION_TARGETS = new Set(['search', 'ocr', 'code', 'kb', 'chat', 'status']);
const NOTIFICATION_URGENCY = new Set(['low', 'normal', 'critical']);

export function isTrustedSenderUrl(senderUrl, trustedOrigin) {
  if (!trustedOrigin) return false;
  try {
    return new URL(senderUrl).origin === trustedOrigin;
  } catch {
    return false;
  }
}

export function validateBackendCall(
  command,
  args,
  maxPayloadBytes = DEFAULT_MAX_BACKEND_PAYLOAD,
) {
  if (typeof command !== 'string' || !/^[a-z][a-z0-9_]{0,63}$/.test(command)) {
    throw new TypeError('Invalid Rust backend command');
  }
  if (!args || typeof args !== 'object' || Array.isArray(args)) {
    throw new TypeError('Rust backend arguments must be an object');
  }
  const bytes = Buffer.byteLength(JSON.stringify(args));
  if (bytes > maxPayloadBytes) {
    throw new RangeError('Rust backend request exceeds the Electron payload limit');
  }
}

export function validateRequestId(requestId) {
  if (typeof requestId !== 'string' || !REQUEST_ID_PATTERN.test(requestId)) {
    throw new TypeError('Invalid Rust backend request identifier');
  }
  return requestId;
}

export function normalizeDialogOptions(value) {
  if (value?.directory === true) {
    return {
      properties: ['openDirectory'],
      filters: [],
    };
  }
  const filters = Array.isArray(value?.filters)
    ? value.filters
      .slice(0, 10)
      .map(filter => ({
        name: String(filter?.name || 'Files').slice(0, 80),
        extensions: Array.isArray(filter?.extensions)
          ? filter.extensions
            .map(extension => String(extension).replace(/^\./, '').replace(/[^a-zA-Z0-9]/g, ''))
            .filter(Boolean)
            .slice(0, 30)
          : [],
      }))
      .filter(filter => filter.extensions.length > 0)
    : [];

  return {
    properties: value?.multiple === false
      ? ['openFile']
      : ['openFile', 'multiSelections'],
    filters,
  };
}

export function normalizeNotification(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('Notification payload must be an object');
  }
  const title = String(value.title || '').replace(/[\r\n\u0000-\u001f]/g, ' ').trim().slice(0, 100);
  const body = String(value.body || '').replace(/[\r\n\u0000-\u001f]/g, ' ').trim().slice(0, 300);
  if (!title || !body) throw new TypeError('Notification title and body are required');
  const target = NOTIFICATION_TARGETS.has(value.target) ? value.target : 'status';
  const urgency = NOTIFICATION_URGENCY.has(value.urgency) ? value.urgency : 'normal';
  const operationId = String(value.operationId || '').replace(/[^a-zA-Z0-9_-]/g, '').slice(0, 80);
  const dedupeKey = String(value.dedupeKey || '').replace(/[^a-zA-Z0-9:_-]/g, '').slice(0, 160);
  return { title, body, target, urgency, operationId, dedupeKey };
}
