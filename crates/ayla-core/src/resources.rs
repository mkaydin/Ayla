use std::path::{Path, PathBuf};

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

    /// Prefer an installed resource when it exists, otherwise resolve a
    /// development-tree path supplied by the host.
    pub fn resolve_resource(
        &self,
        bundled_relative: impl AsRef<Path>,
        development_relative: impl AsRef<Path>,
    ) -> PathBuf {
        if let Some(path) = self.resource(bundled_relative) {
            if path.exists() {
                return path;
            }
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
    fn missing_installed_resource_falls_back_to_development_tree() {
        let locator =
            ResourceLocator::new(Some(PathBuf::from("/missing/ayla")), "/workspace/src-tauri");
        assert_eq!(
            locator.resolve_resource("tts/model.onnx", "../tts/model.onnx"),
            PathBuf::from("/workspace/src-tauri/../tts/model.onnx")
        );
    }
}
