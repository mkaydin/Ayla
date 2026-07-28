// ── Subprocess registry ────────────────────────────────────
// Stores handles to running OCR/search subprocesses for cancellation.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::process::Child;

pub struct ProcessRegistry {
    processes: Arc<Mutex<HashMap<String, Option<Child>>>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self { processes: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn processes_ref(&self) -> Arc<Mutex<HashMap<String, Option<Child>>>> {
        self.processes.clone()
    }
}

pub async fn register(procs: &Arc<Mutex<HashMap<String, Option<Child>>>>, id: String, child: Child) {
    procs.lock().await.insert(id, Some(child));
}

pub async fn kill(procs: &Arc<Mutex<HashMap<String, Option<Child>>>>, id: &str) -> bool {
    if let Some(mut child) = procs.lock().await.remove(id).flatten() {
        let _ = child.kill().await;
        true
    } else {
        false
    }
}
