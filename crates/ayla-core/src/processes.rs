use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

/// Cancellation entry for an in-process backend operation.
pub struct ProcessEntry {
    cancel_tx: Option<oneshot::Sender<()>>,
}

pub struct ProcessRegistry {
    processes: Arc<Mutex<HashMap<String, ProcessEntry>>>,
}

pub struct ProcessCleanupGuard {
    processes: Arc<Mutex<HashMap<String, ProcessEntry>>>,
    id: Option<String>,
}

impl Drop for ProcessCleanupGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let processes = Arc::clone(&self.processes);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = cancel(&processes, &id).await;
            });
        }
    }
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn processes_ref(&self) -> Arc<Mutex<HashMap<String, ProcessEntry>>> {
        self.processes.clone()
    }

    pub async fn cancel(&self, id: &str) -> bool {
        cancel(&self.processes, id).await
    }

    pub async fn register_task(&self, id: String) -> oneshot::Receiver<()> {
        register_task(&self.processes, id).await
    }

    pub async fn finish(&self, id: &str) {
        self.processes.lock().await.remove(id);
    }

    /// Removes and cancels a task if its owning request is aborted before its
    /// normal completion path runs.
    pub fn cleanup_on_drop(&self, id: impl Into<String>) -> ProcessCleanupGuard {
        ProcessCleanupGuard {
            processes: Arc::clone(&self.processes),
            id: Some(id.into()),
        }
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn register_task(
    processes: &Arc<Mutex<HashMap<String, ProcessEntry>>>,
    id: String,
) -> oneshot::Receiver<()> {
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let previous = processes.lock().await.insert(
        id,
        ProcessEntry {
            cancel_tx: Some(cancel_tx),
        },
    );
    if let Some(previous) = previous {
        if let Some(sender) = previous.cancel_tx {
            let _ = sender.send(());
        }
    }
    cancel_rx
}

pub async fn cancel(processes: &Arc<Mutex<HashMap<String, ProcessEntry>>>, id: &str) -> bool {
    let removed = processes.lock().await.remove(id);
    if let Some(entry) = removed {
        if let Some(sender) = entry.cancel_tx {
            let _ = sender.send(());
        }
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registered_task_can_be_cancelled_once() {
        let registry = ProcessRegistry::new();
        let receiver = registry.register_task("task-1".into()).await;

        assert!(registry.cancel("task-1").await);
        assert!(receiver.await.is_ok());
        assert!(!registry.cancel("task-1").await);
    }

    #[tokio::test]
    async fn replacing_a_task_cancels_the_previous_owner() {
        let registry = ProcessRegistry::new();
        let previous = registry.register_task("task-1".into()).await;
        let current = registry.register_task("task-1".into()).await;

        assert!(previous.await.is_ok());
        assert!(registry.cancel("task-1").await);
        assert!(current.await.is_ok());
    }

    #[tokio::test]
    async fn cleanup_guard_cancels_aborted_registration() {
        let registry = ProcessRegistry::new();
        let receiver = registry.register_task("aborted-request".into()).await;
        let guard = registry.cleanup_on_drop("aborted-request");
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(1), receiver)
            .await
            .expect("cleanup guard should signal promptly")
            .expect("cleanup guard should signal cancellation");
        assert!(registry.processes.lock().await.is_empty());
    }
}
