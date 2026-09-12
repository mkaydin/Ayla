import { existsSync } from 'node:fs';
import { execFile } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Notification,
  session,
  shell,
} from 'electron';
import {
  developmentSidecarPath,
  packagedSidecarPath,
  SidecarManager,
} from './sidecar-manager.mjs';
import {
  isTrustedSenderUrl,
  normalizeDialogOptions,
  normalizeNotification,
  validateBackendCall,
  validateRequestId,
} from './ipc-policy.mjs';
import { startStaticServer } from './static-server.mjs';
import { startStorageRecovery } from './storage-recovery.mjs';
import { writeDiagnostic } from './support-diagnostics.mjs';

const electronDirectory = dirname(fileURLToPath(import.meta.url));
const projectDirectory = dirname(electronDirectory);
const preloadPath = join(electronDirectory, 'preload.cjs');
const developmentUrl = process.env.AYLA_ELECTRON_DEV_URL || '';
const installedResourceDirectory = process.env.AYLA_RESOURCE_DIR || (app.isPackaged ? process.resourcesPath : '');
const iconPath = join(
  installedResourceDirectory || projectDirectory,
  ...(installedResourceDirectory ? ['assets'] : ['electron', 'assets']),
  process.platform === 'win32' && !app.isPackaged ? 'icon.ico' : 'icon.png',
);
app.setName('Ayla');
if (app.commandLine.hasSwitch('no-sandbox')) {
  console.error('Ayla requires Chromium sandboxing. Use a supported host or native Arch package.');
  app.exit(64);
}
app.setPath('userData', process.env.AYLA_DATA_DIR || join(app.getPath('appData'), 'ayla'));
const smokeTest = process.argv.includes('--smoke-test');
if (!app.requestSingleInstanceLock()) app.quit();
app.on('second-instance', () => {
  if (applicationWindow && !applicationWindow.isDestroyed()) {
    if (applicationWindow.isMinimized()) applicationWindow.restore();
    applicationWindow.show();
    applicationWindow.focus();
  }
});

function findLocalModelDirectory() {
  if (process.env.AYLA_DISABLE_AVATAR === '1') return null;
  const candidates = [
    process.env.AYLA_LIVE2D_MODEL_DIR,
    join(process.cwd(), 'Mk6_1.0'),
    join(projectDirectory, 'Mk6_1.0'),
    join(dirname(app.getPath('exe')), 'Mk6_1.0'),
    join(app.getPath('userData'), 'Mk6_1.0'),
  ].filter(Boolean);
  return candidates.find(candidate => existsSync(join(candidate, 'Mk6.model3.json'))) || null;
}

let applicationWindow = null;
let staticServer = null;
let sidecar = null;
let trustedOrigin = '';
let quitAfterCleanup = false;
let stopStorageRecovery = () => {};
const recentNotifications = new Map();

const FORWARDED_BACKEND_EVENTS = new Set([
  'knowledge-changed',
  'ingest-progress',
  'kb-cleanup-progress',
  'ocr-started',
  'ocr-progress',
  'code-analysis-started',
  'code-analysis-progress',
  'search-started',
  'search-progress',
  'stt-state',
  'stt-level',
  'stt-partial',
  'stt-final',
  'stt-error',
]);

function assertTrustedSender(event) {
  const senderUrl = event.senderFrame?.url || '';
  if (!isTrustedSenderUrl(senderUrl, trustedOrigin)) {
    let senderOrigin = 'unknown';
    try {
      senderOrigin = new URL(senderUrl).origin;
    } catch {
      // Retain "unknown" for malformed or already destroyed frames.
    }
    throw new Error(`Rejected Electron IPC from untrusted origin: ${senderOrigin}`);
  }
}

function focusNotificationTarget(payload) {
  if (!applicationWindow || applicationWindow.isDestroyed()) return;
  if (applicationWindow.isMinimized()) applicationWindow.restore();
  applicationWindow.show();
  applicationWindow.focus();
  applicationWindow.webContents.send('ayla:event:notification-clicked', {
    target: payload.target,
    operationId: payload.operationId,
  });
}

function showDesktopNotification(rawPayload) {
  const payload = normalizeNotification(rawPayload);
  const now = Date.now();
  if (payload.dedupeKey) {
    const previous = recentNotifications.get(payload.dedupeKey);
    if (previous != null && now - previous < 5_000) return { shown: false, deduplicated: true };
    recentNotifications.set(payload.dedupeKey, now);
  }
  for (const [key, timestamp] of recentNotifications) {
    if (now - timestamp >= 5_000) recentNotifications.delete(key);
  }

  if (Notification.isSupported()) {
    try {
      const nativeNotification = new Notification({
        title: payload.title,
        body: payload.body,
        icon: iconPath,
        urgency: payload.urgency,
        silent: true,
      });
      nativeNotification.on('click', () => focusNotificationTarget(payload));
      nativeNotification.show();
      return { shown: true, backend: 'electron' };
    } catch (error) {
      console.warn(`Native desktop notification failed: ${error.message}`);
    }
  }

  if (process.platform === 'linux' && existsSync('/usr/bin/notify-send')) {
    execFile('/usr/bin/notify-send', [
      '--app-name=Ayla',
      `--urgency=${payload.urgency}`,
      `--icon=${iconPath}`,
      payload.title,
      payload.body,
    ], { windowsHide: true }, error => {
      if (error) console.warn(`KDE notification fallback failed: ${error.message}`);
    });
    return { shown: true, backend: 'notify-send' };
  }
  return { shown: false, backend: 'unsupported' };
}

function createSidecar() {
  const binaryPath = process.env.AYLA_SIDECAR_BIN || (
    installedResourceDirectory
      ? packagedSidecarPath(installedResourceDirectory)
      : developmentSidecarPath(projectDirectory)
  );
  const resourceDirectory = installedResourceDirectory;
  const backend = new SidecarManager({
    binaryPath,
    cwd: app.isPackaged ? app.getPath('userData') : projectDirectory,
    env: {
      ...process.env,
      AYLA_DATA_DIR: app.getPath('userData'),
      ...(resourceDirectory ? { AYLA_CONFIG_FILE: process.env.AYLA_CONFIG_FILE || join(app.getPath('userData'), 'settings.env') } : {}),
      ...(resourceDirectory ? { AYLA_RESOURCE_DIR: resourceDirectory } : {}),
      ...(!app.isPackaged
        ? { AYLA_DEVELOPMENT_ROOT: projectDirectory }
        : {}),
    },
  });
  backend.on('status', status => {
    writeDiagnostic(app.getPath('userData'), { event: 'backend-status', ...status });
    if (applicationWindow && !applicationWindow.isDestroyed()) {
      applicationWindow.webContents.send('ayla:backend-status-changed', status);
    }
  });
  backend.on('backend-event', message => {
    if (
      !FORWARDED_BACKEND_EVENTS.has(message.name)
      || !applicationWindow
      || applicationWindow.isDestroyed()
    ) return;
    const payload = message.payload && typeof message.payload === 'object'
      ? { ...message.payload, requestId: message.operation_id ?? null }
      : { value: message.payload, requestId: message.operation_id ?? null };
    applicationWindow.webContents.send(`ayla:event:${message.name}`, payload);
  });
  backend.on('stderr', chunk => {
    if (process.env.AYLA_SIDECAR_LOG === '1') process.stderr.write(`[ayla-sidecar] ${chunk}`);
  });
  return backend;
}

function registerIpcHandlers() {
  ipcMain.handle('ayla:runtime-info', event => {
    assertTrustedSender(event);
    return {
      runtime: 'electron',
      backendAvailable: sidecar?.snapshot().backendAvailable ?? false,
      backend: sidecar?.snapshot() ?? null,
      chrome: process.versions.chrome,
      electron: process.versions.electron,
      node: process.versions.node,
    };
  });

  ipcMain.handle('ayla:backend-status', event => {
    assertTrustedSender(event);
    return sidecar?.snapshot() ?? {
      state: 'stopped',
      backendAvailable: false,
      error: 'Rust backend has not been initialized',
    };
  });

  ipcMain.handle('ayla:invoke', async (event, command, args = {}) => {
    assertTrustedSender(event);
    validateBackendCall(command, args);
    if (!sidecar) throw new Error('Rust backend has not been initialized');
    return sidecar.request(command, args, { ownerId: event.sender.id });
  });

  ipcMain.handle('ayla:request-start', async (event, requestId, command, args = {}) => {
    assertTrustedSender(event);
    validateRequestId(requestId);
    validateBackendCall(command, args);
    if (!sidecar) throw new Error('Rust backend has not been initialized');
    return sidecar.request(command, args, {
      ownerId: event.sender.id,
      requestId,
    });
  });

  ipcMain.handle('ayla:request-cancel', (event, requestId) => {
    assertTrustedSender(event);
    validateRequestId(requestId);
    return sidecar?.cancelRequest(
      requestId,
      'Operation cancelled by renderer',
      event.sender.id,
    ) ?? false;
  });

  ipcMain.handle('ayla:stream-start', async (event, streamId, command, args = {}) => {
    assertTrustedSender(event);
    validateBackendCall(command, args);
    validateRequestId(streamId);
    if (!sidecar) throw new Error('Rust backend has not been initialized');
    const sender = event.sender;
    return sidecar.request(command, args, {
      ownerId: sender.id,
      requestId: streamId,
      onStream: payload => {
        if (!sender.isDestroyed()) sender.send(`ayla:stream:${streamId}`, payload);
      },
    });
  });

  ipcMain.handle('ayla:stream-cancel', (event, streamId) => {
    assertTrustedSender(event);
    validateRequestId(streamId);
    return sidecar?.cancelRequest(
      streamId,
      'Stream cancelled by renderer',
      event.sender.id,
    ) ?? false;
  });

  ipcMain.handle('ayla:backend-restart', async event => {
    assertTrustedSender(event);
    if (!sidecar) throw new Error('Rust backend has not been initialized');
    return sidecar.restart();
  });

  ipcMain.on('ayla:renderer-dispose', event => {
    try {
      assertTrustedSender(event);
      sidecar?.cancelOwner(event.sender.id);
    } catch {
      // Ignore disposal messages from a frame that has already navigated.
    }
  });

  ipcMain.handle('ayla:select-files', async (event, options) => {
    assertTrustedSender(event);
    const owner = BrowserWindow.fromWebContents(event.sender) || applicationWindow;
    const result = await dialog.showOpenDialog(owner, normalizeDialogOptions(options));
    return result.canceled ? [] : result.filePaths;
  });

  ipcMain.handle('ayla:open-path', async (event, filePath) => {
    assertTrustedSender(event);
    if (typeof filePath !== 'string' || filePath.length === 0 || filePath.length > 4096) {
      throw new TypeError('A valid local path is required');
    }
    const error = await shell.openPath(filePath);
    if (error) throw new Error(error);
  });

  ipcMain.handle('ayla:show-notification', (event, payload) => {
    assertTrustedSender(event);
    return showDesktopNotification(payload);
  });
}

async function createWindow() {
  if (developmentUrl) {
    trustedOrigin = new URL(developmentUrl).origin;
  } else {
    const distDirectory = join(projectDirectory, 'dist');
    if (!existsSync(join(distDirectory, 'index.html'))) {
      throw new Error('Electron production assets are missing. Run `npm run build` first.');
    }
    if (!staticServer) {
      staticServer = await startStaticServer(distDirectory, {
        modelDirectory: findLocalModelDirectory(),
      });
    }
    trustedOrigin = staticServer.origin;
  }

  session.defaultSession.setPermissionRequestHandler((webContents, permission, callback, details) => {
    let requestingOrigin = '';
    try { requestingOrigin = new URL(details.requestingUrl || '').origin; } catch {}
    const mediaTypes = details.mediaTypes || [];
    const audioOnly = mediaTypes.includes('audio') && !mediaTypes.includes('video');
    callback(
      permission === 'media'
      && audioOnly
      && requestingOrigin === trustedOrigin
      && webContents === applicationWindow?.webContents,
    );
  });

  applicationWindow = new BrowserWindow({
    width: 1280,
    height: 800,
    minWidth: 900,
    minHeight: 600,
    show: false,
    backgroundColor: '#07111e',
    icon: iconPath,
    autoHideMenuBar: true,
    webPreferences: {
      preload: preloadPath,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      backgroundThrottling: true,
      spellcheck: false,
    },
  });

  applicationWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (url.startsWith('https://') || url.startsWith('http://')) {
      void shell.openExternal(url);
    }
    return { action: 'deny' };
  });
  applicationWindow.webContents.on('will-navigate', (event, url) => {
    if (new URL(url).origin !== trustedOrigin) {
      event.preventDefault();
      if (url.startsWith('https://') || url.startsWith('http://')) void shell.openExternal(url);
    }
  });
  const rendererId = applicationWindow.webContents.id;
  const cancelRendererRequests = () => sidecar?.cancelOwner(rendererId);
  applicationWindow.webContents.on(
    'did-start-navigation',
    (_event, _url, _isInPlace, isMainFrame) => {
      if (isMainFrame) cancelRendererRequests();
    },
  );
  applicationWindow.webContents.on('render-process-gone', (_event, details) => {
    writeDiagnostic(app.getPath('userData'), { event: 'renderer-crash', exitCode: details.exitCode });
    cancelRendererRequests();
  });
  applicationWindow.webContents.once('destroyed', cancelRendererRequests);
  applicationWindow.once('ready-to-show', () => { if (!smokeTest) applicationWindow?.show(); });
  applicationWindow.on('closed', () => {
    applicationWindow = null;
  });

  await applicationWindow.loadURL(developmentUrl || staticServer.origin);
}

app.whenReady()
  .then(async () => {
    writeDiagnostic(app.getPath('userData'), { event: 'application-start', appVersion: app.getVersion(), electronVersion: process.versions.electron, nodeVersion: process.versions.node });
    registerIpcHandlers();
    sidecar = createSidecar();
    const backendStart = sidecar.start().catch(error => {
      console.error(`Rust backend unavailable: ${error.message}`);
    });
    await createWindow();
    await backendStart;
    stopStorageRecovery = startStorageRecovery(sidecar);
    if (smokeTest) {
      const result = await applicationWindow.webContents.executeJavaScript(`(async () => {
        const deadline = Date.now() + 30000;
        while (!document.querySelector('#chat-input') && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 100));
        const panels = [...document.querySelectorAll('[data-panel-id]')].map(el => el.dataset.panelId);
        const capabilities = await window.aylaDesktop.invoke('get_runtime_capabilities', {});
        return { panels, avatarAvailable: window.__app?.avatarAvailable, backend: await window.aylaDesktop.backendStatus(), capabilities };
      })()`);
      const expected = ['chat', 'ocr', 'code', 'search', 'kb', 'status'];
      if (expected.some(panel => !result.panels.includes(panel)) || !result.backend.backendAvailable) {
        throw new Error(`Packaged smoke test failed: ${JSON.stringify(result)}`);
      }
      console.log(JSON.stringify({ status: 'ok', ...result }));
      app.quit();
    }
  })
  .catch(error => {
    if (smokeTest) { console.error(error.stack || error.message); process.exitCode = 1; }
    else dialog.showErrorBox('Ayla failed to start', error.stack || error.message);
    app.quit();
  });

app.on('activate', () => {
  if (BrowserWindow.getAllWindows().length === 0) void createWindow();
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});

app.on('before-quit', event => {
  if (quitAfterCleanup) return;
  event.preventDefault();
  quitAfterCleanup = true;
  writeDiagnostic(app.getPath('userData'), { event: 'application-stop' });
  stopStorageRecovery();
  // Stop renderer timers/IPC before tearing down their backend.
  applicationWindow?.destroy();
  Promise.allSettled([
    sidecar?.stop(),
    staticServer?.close(),
  ]).finally(() => app.quit());
});
