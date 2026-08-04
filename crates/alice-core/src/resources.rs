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

    pub fn resolve_tool(
        &self,
        environment_key: &str,
        bundled_name: &str,
        development_relative: &str,
    ) -> PathBuf {
        if let Ok(path) = std::env::var(environment_key) {
            let path = PathBuf::from(path);
            if path.exists() {
                return path;
            }
        }

        if let Some(resource_dir) = &self.resource_dir {
            let path = resource_dir.join("tools").join(bundled_name);
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
    fn development_tool_path_is_deterministic() {
        let locator = ResourceLocator::new(None, "/workspace/src-tauri");
        assert_eq!(
            locator.resolve_tool(
                "ALICE_TEST_TOOL_NOT_SET",
                "helper",
                "../../helper/target/release/helper"
            ),
            PathBuf::from("/workspace/src-tauri/../../helper/target/release/helper")
        );
    }

    #[test]
    fn resource_paths_are_host_supplied() {
        let locator = ResourceLocator::new(Some(PathBuf::from("/opt/alice")), "/workspace");
        assert_eq!(
            locator.resource("tts/model.onnx"),
            Some(PathBuf::from("/opt/alice/tts/model.onnx"))
        );
    }

    #[test]
    fn missing_installed_resource_falls_back_to_development_tree() {
        let locator = ResourceLocator::new(
            Some(PathBuf::from("/missing/alice")),
            "/workspace/src-tauri",
        );
        assert_eq!(
            locator.resolve_resource("tts/model.onnx", "../tts/model.onnx"),
            PathBuf::from("/workspace/src-tauri/../tts/model.onnx")
        );
    }
}
