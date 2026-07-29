// ── Subprocess registry ────────────────────────────────────
// Stores handles to running OCR/search subprocesses for cancellation.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};
use tokio::process::Child;

/// Entry stored for each registered process.
pub struct ProcessEntry {
    pub child: Option<Child>,
    /// Sender fired by `kill()` to interrupt an in-flight `wait()`.
    cancel_tx: Option<oneshot::Sender<()>>,
}

pub struct ProcessRegistry {
    processes: Arc<Mutex<HashMap<String, ProcessEntry>>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self { processes: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn processes_ref(&self) -> Arc<Mutex<HashMap<String, ProcessEntry>>> {
        self.processes.clone()
    }
}

/// Register a running child under `id`. Returns the cancellation receiver that
/// `wait_or_cancel` races against so cancellation works *during* the wait (issue R7).
///
/// If `id` is already taken, the previous child is killed+reaped first so it isn't
/// orphaned (issue R9 — tokio `Child` is detached on drop, not killed).
pub async fn register(
    procs: &Arc<Mutex<HashMap<String, ProcessEntry>>>,
    id: String,
    child: Child,
) -> oneshot::Receiver<()> {
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let mut map = procs.lock().await;
    if let Some(prev) = map.insert(id.clone(), ProcessEntry { child: Some(child), cancel_tx: Some(cancel_tx) }) {
        // Replaced an existing entry — reap the previous child.
        if let Some(mut prev_child) = prev.child {
            if let Err(e) = prev_child.kill().await {
                log::warn!("Failed to kill orphaned process '{}': {}", id, e);
            }
            let _ = prev_child.wait().await; // reap (issue R8 — avoid zombie)
        }
    }
    cancel_rx
}

/// Kill the child registered under `id` and reap it. The cancellation channel also fires so
/// a concurrent `wait_or_cancel` wakes up and reports "Cancelled" (issue R7).
///
/// Returns false if the id was never registered (or already completed). Note: a return value
/// of true means "we found and signalled it", not necessarily "the OS kill succeeded".
pub async fn kill(procs: &Arc<Mutex<HashMap<String, ProcessEntry>>>, id: &str) -> bool {
    let removed = procs.lock().await.remove(id);
    if let Some(entry) = removed {
        // Signal any in-flight waiter first.
        if let Some(tx) = entry.cancel_tx {
            let _ = tx.send(());
        }
        if let Some(mut child) = entry.child {
            // SIGKILL then wait() to reap — otherwise the kernel keeps a zombie until init
            // reaps it (issue R8).
            if let Err(e) = child.kill().await {
                log::warn!("Failed to kill process '{}': {}", id, e);
                return false;
            }
            let _ = child.wait().await;
        }
        true
    } else {
        false
    }
}
