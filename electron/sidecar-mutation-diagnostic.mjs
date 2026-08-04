import { randomUUID } from 'node:crypto';
import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const sessionId = `alice-phase3-diagnostic-${randomUUID()}`;
const title = 'ALICE Phase 3 mutation diagnostic';
const messages = [
  { role: 'user', content: 'Verify Electron to Rust mutation parity.' },
  { role: 'assistant', content: 'The diagnostic record will be deleted immediately.' },
];
const manager = new SidecarManager({
  binaryPath: developmentSidecarPath(root),
  cwd: root,
  env: {
    ...process.env,
    ALICE_DEVELOPMENT_ROOT: path.join(root, 'src-tauri'),
  },
  startupTimeout: 60_000,
  restartLimit: 0,
});

let created = false;
try {
  const hello = await manager.start();
  if (!await manager.request('check_db_ready')) {
    throw new Error(
      'Rust sidecar started, but SurrealDB/Qdrant initialization is offline. Check Docker services.',
    );
  }

  await manager.request('save_chat_session', {
    sessionId,
    title,
    messagesJson: JSON.stringify(messages),
  });
  created = true;

  const afterCreate = await manager.request('load_chat_sessions');
  const stored = afterCreate.find(session => session.id === sessionId);
  if (
    !stored
    || stored.title !== title
    || stored.messages?.length !== messages.length
    || stored.messages[0]?.content !== messages[0].content
  ) {
    throw new Error(`Created chat session did not round-trip correctly: ${JSON.stringify(stored)}`);
  }

  await manager.request('delete_chat_session', { sessionId });
  created = false;
  const afterDelete = await manager.request('load_chat_sessions');
  if (afterDelete.some(session => session.id === sessionId)) {
    throw new Error('Diagnostic chat session remains after deletion');
  }

  await manager.stop();
  console.log(JSON.stringify({
    status: 'ok',
    protocolVersion: hello.protocolVersion,
    backendVersion: hello.backendVersion,
    mutation: 'save_chat_session',
    readAfterWrite: 'ok',
    messagesRoundTripped: messages.length,
    cleanup: 'delete_chat_session',
    recordRemaining: false,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  if (created) {
    await manager.request('delete_chat_session', { sessionId }).catch(cleanupError => {
      error.message += `; cleanup also failed: ${cleanupError.message}`;
    });
  }
  await manager.stop().catch(() => {});
  throw error;
}
