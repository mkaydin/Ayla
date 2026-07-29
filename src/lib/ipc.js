// ── Tauri IPC abstraction ──────────────────────────────────

export function getIpc() {
  const ipc = window.__TAURI_INTERNALS__;
  return ipc?.invoke || ipc?.ipc?.invoke || ipc?.core?.invoke || null;
}

export async function invoke(cmd, args = {}) {
  const fn = getIpc();
  if (!fn) throw new Error('Tauri IPC not available (browser dev mode)');
  return fn(cmd, args);
}

let eventListen = null;

export async function listenEvent(event, cb) {
  if (!window.__TAURI_INTERNALS__) {
    console.log('[listen] Mock event:', event);
    return () => {};
  }
  if (!eventListen) {
    const mod = await import('@tauri-apps/api/event');
    eventListen = mod.listen;
  }
  return eventListen(event, cb);
}

let shellOpen = null;

export async function openPdf(filePath) {
  if (window.__TAURI_INTERNALS__) {
    if (!shellOpen) {
      const mod = await import('@tauri-apps/plugin-shell');
      shellOpen = mod.open;
    }
    await shellOpen(filePath);
  } else {
    window.open(filePath, '_blank');
  }
}
