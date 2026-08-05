// ── Desktop runtime abstraction ────────────────────────────

function tauriInvoke() {
  const ipc = window.__TAURI_INTERNALS__;
  return ipc?.invoke || ipc?.ipc?.invoke || ipc?.core?.invoke || null;
}

export function getDesktopRuntime() {
  if (window.__TAURI_INTERNALS__) return 'tauri';
  if (window.aylaDesktop?.runtime === 'electron') return 'electron';
  return 'browser';
}

function getIpc() {
  const invokeTauri = tauriInvoke();
  if (invokeTauri) return invokeTauri;
  if (window.aylaDesktop?.invoke) {
    return window.aylaDesktop.invoke;
  }
  return null;
}

export function hasDesktopBackend() {
  return Boolean(getIpc());
}

export async function getRuntimeInfo() {
  if (window.aylaDesktop?.runtimeInfo) return window.aylaDesktop.runtimeInfo();
  const runtime = getDesktopRuntime();
  return {
    runtime,
    backendAvailable: runtime === 'tauri',
    backend: runtime === 'tauri' ? await getBackendStatus() : null,
    chrome: navigator.userAgentData?.brands
      ?.find(brand => brand.brand === 'Chromium')?.version
      || null,
    electron: null,
    node: null,
  };
}

export function canRestartBackend() {
  return typeof window.aylaDesktop?.restartBackend === 'function';
}

export function serviceHealthUrl(id, directUrl) {
  return getDesktopRuntime() === 'electron' ? `/api/services/${id}` : directUrl;
}

export function invokeStreamCancelable(cmd, args, onEvent) {
  const requestId = crypto.randomUUID();
  const electron = window.aylaDesktop;
  let settled = false;
  const promise = window.__TAURI_INTERNALS__
    ? import('@tauri-apps/api/core').then(({ Channel }) => {
      const channel = new Channel(onEvent);
      return invoke(cmd, { ...args, onEvent: channel });
    })
    : electron?.stream
      ? electron.stream(cmd, args, onEvent, requestId)
      : Promise.reject(
        new Error(`Rust streaming backend is not available in ${getDesktopRuntime()} mode`),
      );
  promise.then(
    () => { settled = true; },
    () => { settled = true; },
  );
  return Object.freeze({
    requestId,
    promise,
    async cancel() {
      if (settled || !electron?.cancelStream) return false;
      return Boolean(await electron.cancelStream(requestId));
    },
  });
}

export async function invokeStream(cmd, args, onEvent) {
  return invokeStreamCancelable(cmd, args, onEvent).promise;
}

export async function invoke(cmd, args = {}) {
  const fn = getIpc();
  if (!fn) {
    const runtime = getDesktopRuntime();
    throw new Error(`Rust command backend is not available in ${runtime} mode`);
  }
  return fn(cmd, args);
}

export function invokeCancelable(cmd, args = {}) {
  const requestId = crypto.randomUUID();
  const electron = window.aylaDesktop;
  let processId = null;
  let cancelRequested = false;
  let settled = false;

  const promise = electron?.invokeRequest
    ? electron.invokeRequest(requestId, cmd, args)
    : invoke(cmd, args);
  promise.then(
    () => { settled = true; },
    () => { settled = true; },
  );

  const cancelTauriProcess = async () => {
    if (!processId) return false;
    return Boolean(await invoke('cancel_process', { pid: processId }));
  };

  return Object.freeze({
    requestId,
    promise,
    setProcessId(value) {
      if (typeof value !== 'string' || !value) return;
      processId = value;
      if (cancelRequested && !electron?.cancelRequest) {
        void cancelTauriProcess().catch(() => {});
      }
    },
    async cancel() {
      if (settled) return false;
      cancelRequested = true;
      if (electron?.cancelRequest) {
        return Boolean(await electron.cancelRequest(requestId));
      }
      return cancelTauriProcess();
    },
  });
}

export async function getBackendStatus() {
  if (window.aylaDesktop?.backendStatus) return window.aylaDesktop.backendStatus();
  if (window.__TAURI_INTERNALS__) {
    return {
      state: 'ready',
      backendAvailable: true,
      backendVersion: 'tauri',
      restartAttempt: 0,
      nextRetryAt: null,
      error: null,
    };
  }
  return {
    state: 'unavailable',
    backendAvailable: false,
    backendVersion: null,
    restartAttempt: 0,
    nextRetryAt: null,
    error: 'Desktop backend is unavailable',
  };
}

export async function restartBackend() {
  if (window.aylaDesktop?.restartBackend) return window.aylaDesktop.restartBackend();
  throw new Error('Backend restart is only available in Electron mode');
}

export function listenBackendStatus(callback) {
  if (window.aylaDesktop?.listenBackendStatus) {
    return window.aylaDesktop.listenBackendStatus(callback);
  }
  return () => {};
}

let eventListen = null;

export async function listenEvent(event, cb) {
  if (window.__TAURI_INTERNALS__) {
    if (!eventListen) {
      const mod = await import('@tauri-apps/api/event');
      eventListen = mod.listen;
    }
    return eventListen(event, cb);
  }
  if (window.aylaDesktop?.listen) return window.aylaDesktop.listen(event, cb);
  return () => {};
}

export async function selectFiles(options = {}) {
  if (window.__TAURI_INTERNALS__) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const selected = await open(options);
    if (!selected) return [];
    return Array.isArray(selected) ? selected : [selected];
  }
  if (window.aylaDesktop?.selectFiles) {
    return window.aylaDesktop.selectFiles(options);
  }
  return null;
}

let shellOpen = null;

export async function openPdf(filePath) {
  if (window.__TAURI_INTERNALS__) {
    if (!shellOpen) {
      const mod = await import('@tauri-apps/plugin-shell');
      shellOpen = mod.open;
    }
    await shellOpen(filePath);
    return;
  }
  if (window.aylaDesktop?.openPath) {
    await window.aylaDesktop.openPath(filePath);
    return;
  }
  window.open(filePath, '_blank');
}

export async function showDesktopNotification(payload) {
  if (window.aylaDesktop?.showNotification) {
    return window.aylaDesktop.showNotification(payload);
  }
  return { shown: false, backend: 'unsupported' };
}
