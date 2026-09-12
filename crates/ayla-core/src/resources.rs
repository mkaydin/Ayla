use std::path::{Path, PathBuf};

pub struct DataWriterGuard(std::fs::File);

impl DataWriterGuard {
    pub fn acquire(directory: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("backend-writer.lock"))?;
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "Another backend owns this data directory",
                ));
            }
        }
        Ok(Self(file))
    }
}

impl Drop for DataWriterGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

/// Resolves runtime resources without depending on a desktop framework.
#[derive(Debug, Clone)]
pub struct ResourceLocator {
    resource_dir: Option<PathBuf>,
    development_root: PathBuf,
}

impl ResourceLocator {
    pub fn new(resource_dir: Option<PathBuf>, development_root: impl Into<PathBuf>) -> Self {
        Self {
            resource_dir,
            development_root: development_root.into(),
        }
    }

    pub fn resource_dir(&self) -> Option<&Path> {
        self.resource_dir.as_deref()
    }

    pub fn resource(&self, relative_path: impl AsRef<Path>) -> Option<PathBuf> {
        self.resource_dir
            .as_ref()
            .map(|directory| directory.join(relative_path))
    }

    /// Installed resources are authoritative, including missing-resource errors.
    /// Only development hosts resolve paths from the checkout.
    pub fn resolve_resource(
        &self,
        bundled_relative: impl AsRef<Path>,
        development_relative: impl AsRef<Path>,
    ) -> PathBuf {
        if let Some(path) = self.resource(bundled_relative) {
            return path;
        }
        self.development_root.join(development_relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_paths_are_host_supplied() {
        let locator = ResourceLocator::new(Some(PathBuf::from("/opt/ayla")), "/workspace");
        assert_eq!(
            locator.resource("tts/model.onnx"),
            Some(PathBuf::from("/opt/ayla/tts/model.onnx"))
        );
    }

    #[test]
    fn installed_resource_never_falls_back_to_a_checkout() {
        let locator = ResourceLocator::new(Some(PathBuf::from("/missing/ayla")), "/workspace");
        assert_eq!(
            locator.resolve_resource("tts/model.onnx", "tts/model.onnx"),
            PathBuf::from("/missing/ayla/tts/model.onnx")
        );
    }
}
