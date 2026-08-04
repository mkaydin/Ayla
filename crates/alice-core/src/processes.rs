use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::{oneshot, Mutex};

/// Entry stored for each cancellable helper process or in-process task.
pub struct ProcessEntry {
    pub child: Option<Child>,
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
                let _ = kill(&processes, &id).await;
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
        kill(&self.processes, id).await
    }

    pub async fn register_child(&self, id: String, child: Child) -> oneshot::Receiver<()> {
        register(&self.processes, id, child).await
    }

    pub async fn register_task(&self, id: String) -> oneshot::Receiver<()> {
        register_task(&self.processes, id).await
    }

    pub async fn finish(&self, id: &str) {
        self.processes.lock().await.remove(id);
    }

    /// Ensures a registered process/task is removed if its owning async
    /// request is aborted before reaching the normal `finish` path.
    pub fn cleanup_on_drop(&self, id: impl Into<String>) -> ProcessCleanupGuard {
        ProcessCleanupGuard {
            processes: Arc::clone(&self.processes),
            id: Some(id.into()),
        }
    }

    /// Wait for a registered child without removing its cancellation sender.
    /// This lets a concurrent `cancel` interrupt the wait and ensures
    /// `kill_on_drop(true)` reaps a child whose wait future is cancelled.
    pub async fn wait_child(
        &self,
        id: &str,
        output_path: Option<&str>,
        cancel_rx: oneshot::Receiver<()>,
    ) -> Result<String, String> {
        let child = {
            let mut processes = self.processes.lock().await;
            processes.get_mut(id).and_then(|entry| entry.child.take())
        }
        .ok_or_else(|| "Cancelled".to_string())?;

        let output = tokio::select! {
            result = child.wait_with_output() => match result {
                Ok(output) => output,
                Err(error) => {
                    self.finish(id).await;
                    remove_output_file(output_path).await;
                    return Err(error.to_string());
                }
            },
            _ = cancel_rx => {
                self.finish(id).await;
                remove_output_file(output_path).await;
                return Err("Cancelled".to_string());
            }
        };
        self.finish(id).await;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            remove_output_file(output_path).await;
            return Err(stderr.lines().last().unwrap_or("unknown").to_string());
        }
        match output_path.filter(|path| !path.is_empty()) {
            Some(path) => {
                let result = tokio::fs::read_to_string(path)
                    .await
                    .map_err(|error| error.to_string());
                remove_output_file(Some(path)).await;
                result
            }
            None => Ok(String::from_utf8_lossy(&output.stdout).to_string()),
        }
    }
}

async fn remove_output_file(output_path: Option<&str>) {
    let Some(path) = output_path.filter(|path| !path.is_empty()) else {
        return;
    };
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("Failed to remove helper output '{}': {}", path, error);
        }
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn register(
    processes: &Arc<Mutex<HashMap<String, ProcessEntry>>>,
    id: String,
    child: Child,
) -> oneshot::Receiver<()> {
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let mut map = processes.lock().await;
    if let Some(previous) = map.insert(
        id.clone(),
        ProcessEntry {
            child: Some(child),
            cancel_tx: Some(cancel_tx),
        },
    ) {
        if let Some(mut previous_child) = previous.child {
            if let Err(error) = previous_child.kill().await {
                log::warn!("Failed to kill orphaned process '{}': {}", id, error);
            }
            let _ = previous_child.wait().await;
        }
    }
    cancel_rx
}

pub async fn register_task(
    processes: &Arc<Mutex<HashMap<String, ProcessEntry>>>,
    id: String,
) -> oneshot::Receiver<()> {
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let mut map = processes.lock().await;
    if let Some(previous) = map.insert(
        id.clone(),
        ProcessEntry {
            child: None,
            cancel_tx: Some(cancel_tx),
        },
    ) {
        if let Some(mut child) = previous.child {
            if let Err(error) = child.kill().await {
                log::warn!("Failed to kill replaced process '{}': {}", id, error);
            }
            let _ = child.wait().await;
        }
    }
    cancel_rx
}

pub async fn kill(processes: &Arc<Mutex<HashMap<String, ProcessEntry>>>, id: &str) -> bool {
    let removed = processes.lock().await.remove(id);
    if let Some(entry) = removed {
        if let Some(sender) = entry.cancel_tx {
            let _ = sender.send(());
        }
        if let Some(mut child) = entry.child {
            if let Err(error) = child.kill().await {
                log::warn!("Failed to kill process '{}': {}", id, error);
                return false;
            }
            let _ = child.wait().await;
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
        let processes = registry.processes_ref();
        let receiver = register_task(&processes, "task-1".into()).await;

        assert!(registry.cancel("task-1").await);
        assert!(receiver.await.is_ok());
        assert!(!registry.cancel("task-1").await);
    }

    #[tokio::test]
    async fn helper_output_cleanup_removes_temporary_file() {
        let path =
            std::env::temp_dir().join(format!("alice-process-output-{}.txt", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, "temporary helper output")
            .await
            .expect("write temporary helper output");
        remove_output_file(path.to_str()).await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cleanup_guard_removes_aborted_registration() {
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
