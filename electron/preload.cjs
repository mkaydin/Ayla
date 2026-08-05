const { contextBridge, ipcRenderer } = require('electron');

const api = Object.freeze({
  runtime: 'electron',
  runtimeInfo: () => ipcRenderer.invoke('ayla:runtime-info'),
  backendStatus: () => ipcRenderer.invoke('ayla:backend-status'),
  restartBackend: () => ipcRenderer.invoke('ayla:backend-restart'),
  invoke: (command, args = {}) => ipcRenderer.invoke('ayla:invoke', command, args),
  invokeRequest: (requestId, command, args = {}) => (
    ipcRenderer.invoke('ayla:request-start', requestId, command, args)
  ),
  cancelRequest: requestId => ipcRenderer.invoke('ayla:request-cancel', requestId),
  stream(command, args, callback, requestedStreamId = null) {
    if (typeof callback !== 'function') {
      throw new TypeError('Electron stream callback must be a function');
    }
    const streamId = requestedStreamId || globalThis.crypto?.randomUUID?.()
      || `${Date.now().toString(16)}-${Math.random().toString(16).slice(2)}`;
    const channel = `ayla:stream:${streamId}`;
    const listener = (_event, payload) => callback(payload);
    ipcRenderer.on(channel, listener);
    return ipcRenderer
      .invoke('ayla:stream-start', streamId, command, args || {})
      .finally(() => ipcRenderer.removeListener(channel, listener));
  },
  cancelStream: streamId => ipcRenderer.invoke('ayla:stream-cancel', streamId),
  selectFiles: (options = {}) => ipcRenderer.invoke('ayla:select-files', options),
  openPath: (filePath) => ipcRenderer.invoke('ayla:open-path', filePath),
  showNotification: payload => ipcRenderer.invoke('ayla:show-notification', payload),
  listen(eventName, callback) {
    if (typeof callback !== 'function') {
      throw new TypeError('Electron event listener must be a function');
    }

    const channel = `ayla:event:${String(eventName)}`;
    const listener = (_event, payload) => callback({ event: eventName, payload });
    ipcRenderer.on(channel, listener);
    return () => ipcRenderer.removeListener(channel, listener);
  },
  listenBackendStatus(callback) {
    if (typeof callback !== 'function') {
      throw new TypeError('Electron backend status listener must be a function');
    }
    const listener = (_event, status) => callback(status);
    ipcRenderer.on('ayla:backend-status-changed', listener);
    return () => ipcRenderer.removeListener('ayla:backend-status-changed', listener);
  },
});

contextBridge.exposeInMainWorld('aylaDesktop', api);
globalThis.addEventListener?.('beforeunload', () => {
  ipcRenderer.send('ayla:renderer-dispose');
});
