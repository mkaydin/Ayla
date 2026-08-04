const { contextBridge, ipcRenderer } = require('electron');

const api = Object.freeze({
  runtime: 'electron',
  runtimeInfo: () => ipcRenderer.invoke('alice:runtime-info'),
  backendStatus: () => ipcRenderer.invoke('alice:backend-status'),
  restartBackend: () => ipcRenderer.invoke('alice:backend-restart'),
  invoke: (command, args = {}) => ipcRenderer.invoke('alice:invoke', command, args),
  invokeRequest: (requestId, command, args = {}) => (
    ipcRenderer.invoke('alice:request-start', requestId, command, args)
  ),
  cancelRequest: requestId => ipcRenderer.invoke('alice:request-cancel', requestId),
  stream(command, args, callback, requestedStreamId = null) {
    if (typeof callback !== 'function') {
      throw new TypeError('Electron stream callback must be a function');
    }
    const streamId = requestedStreamId || globalThis.crypto?.randomUUID?.()
      || `${Date.now().toString(16)}-${Math.random().toString(16).slice(2)}`;
    const channel = `alice:stream:${streamId}`;
    const listener = (_event, payload) => callback(payload);
    ipcRenderer.on(channel, listener);
    return ipcRenderer
      .invoke('alice:stream-start', streamId, command, args || {})
      .finally(() => ipcRenderer.removeListener(channel, listener));
  },
  cancelStream: streamId => ipcRenderer.invoke('alice:stream-cancel', streamId),
  selectFiles: (options = {}) => ipcRenderer.invoke('alice:select-files', options),
  openPath: (filePath) => ipcRenderer.invoke('alice:open-path', filePath),
  showNotification: payload => ipcRenderer.invoke('alice:show-notification', payload),
  listen(eventName, callback) {
    if (typeof callback !== 'function') {
      throw new TypeError('Electron event listener must be a function');
    }

    const channel = `alice:event:${String(eventName)}`;
    const listener = (_event, payload) => callback({ event: eventName, payload });
    ipcRenderer.on(channel, listener);
    return () => ipcRenderer.removeListener(channel, listener);
  },
  listenBackendStatus(callback) {
    if (typeof callback !== 'function') {
      throw new TypeError('Electron backend status listener must be a function');
    }
    const listener = (_event, status) => callback(status);
    ipcRenderer.on('alice:backend-status-changed', listener);
    return () => ipcRenderer.removeListener('alice:backend-status-changed', listener);
  },
});

contextBridge.exposeInMainWorld('aliceDesktop', api);
globalThis.addEventListener?.('beforeunload', () => {
  ipcRenderer.send('alice:renderer-dispose');
});
