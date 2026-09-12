//! Private durable stage journal. A cancelled future owns no background workers.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct ResearchJob {
    path: PathBuf,
    record: Mutex<Value>,
}
impl ResearchJob {
    pub fn create(root: &Path, id: &str, query: &str, mode: &str, model: &str) -> Result<Self> {
        let directory = root.join("jobs");
        std::fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let job = Self {
            path: directory.join(format!("{id}.json")),
            record: Mutex::new(
                json!({"schema_version": 1, "operation_id": id, "query": query, "mode": mode, "model": model, "status": "running", "stage": "queued"}),
            ),
        };
        job.update("queued", "running")?;
        Ok(job)
    }
    pub fn update(&self, stage: &str, status: &str) -> Result<()> {
        let mut record = self
            .record
            .lock()
            .map_err(|_| anyhow::anyhow!("Research journal lock poisoned"))?;
        record["stage"] = json!(stage);
        record["status"] = json!(status);
        let temporary = self
            .path
            .with_extension(format!("{}.part", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(&*record)?;
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.context("Could not save research progress")
    }
    pub fn recover(root: &Path) -> Result<usize> {
        let directory = root.join("jobs");
        if !directory.exists() {
            return Ok(0);
        }
        let mut recovered = 0;
        for entry in std::fs::read_dir(directory)?.take(10_000) {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
                || entry.metadata()?.len() > 64 * 1024
            {
                continue;
            }
            let record: Value = serde_json::from_slice(&std::fs::read(entry.path())?)?;
            if record["status"] == "running" {
                let job = Self {
                    path: entry.path(),
                    record: Mutex::new(record),
                };
                job.update("interrupted", "interrupted")?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }
}
impl Drop for ResearchJob {
    fn drop(&mut self) {
        let running = self
            .record
            .lock()
            .map(|record| record["status"] == "running")
            .unwrap_or(false);
        if running {
            let _ = self.update("cancelled", "cancelled");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_restart_keep_an_explicit_durable_outcome() {
        let root = std::env::temp_dir().join(format!("ayla-job-test-{}", uuid::Uuid::new_v4()));
        {
            let job = ResearchJob::create(
                &root,
                "fixture",
                "public test query",
                "papers",
                "fixture-model",
            )
            .unwrap();
            job.update("fetching", "running").unwrap();
        }
        let record: Value =
            serde_json::from_slice(&std::fs::read(root.join("jobs/fixture.json")).unwrap())
                .unwrap();
        assert_eq!(record["status"], "cancelled");
        let job = ResearchJob::create(
            &root,
            "crash",
            "public test query",
            "papers",
            "fixture-model",
        )
        .unwrap();
        std::mem::forget(job);
        assert_eq!(ResearchJob::recover(&root).unwrap(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
