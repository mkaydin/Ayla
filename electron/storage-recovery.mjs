// Reinitialize degraded storage by restarting an idle sidecar. Renderer state
// survives and active requests are never interrupted by automatic recovery.
export function startStorageRecovery(backend, { intervalMs = 60000, setTimer = setTimeout, clearTimer = clearTimeout } = {}) {
  let stopped = false;
  let timer;
  async function check() {
    try {
      if (!stopped && backend.snapshot().backendAvailable && backend.pending.size === 0) {
        const ready = await backend.request('check_db_ready');
        if (!ready && !stopped && backend.pending.size === 0) {
          const capabilities = await backend.request('get_runtime_capabilities');
          // Dictation is a long-lived native session after its start request completes.
          if (!capabilities?.stt?.listening && !stopped && backend.pending.size === 0) await backend.restart();
        }
      }
    } catch { /* Backend lifecycle already reports failures and retry state. */ }
    finally { if (!stopped) timer = setTimer(check, intervalMs); }
  }
  timer = setTimer(check, intervalMs);
  return () => { stopped = true; clearTimer(timer); };
}
