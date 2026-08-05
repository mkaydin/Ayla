use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager};

const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn model_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(configured) = std::env::var_os("ALICE_LIVE2D_MODEL_DIR") {
        roots.push(PathBuf::from(configured));
    }
    if let Ok(current) = std::env::current_dir() {
        roots.push(current.join("Mk6_1.0"));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            roots.push(directory.join("Mk6_1.0"));
        }
    }
    roots.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
            .join("Mk6_1.0"),
    );
    if let Ok(app_data) = app.path().app_data_dir() {
        roots.push(app_data.join("Mk6_1.0"));
    }
    roots
}

#[tauri::command]
pub async fn read_live2d_asset(
    app: AppHandle,
    relative_path: String,
) -> Result<tauri::ipc::Response, String> {
    let relative = PathBuf::from(relative_path);
    if !is_safe_relative_path(&relative) {
        return Err("Invalid Live2D asset path".into());
    }

    for root in model_roots(&app) {
        if !root.join("Mk6.model3.json").is_file() {
            continue;
        }
        let canonical_root = root
            .canonicalize()
            .map_err(|error| format!("Live2D model directory: {error}"))?;
        let candidate = canonical_root.join(&relative);
        let canonical_file = match candidate.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !canonical_file.starts_with(&canonical_root) || !canonical_file.is_file() {
            continue;
        }
        let size = canonical_file
            .metadata()
            .map_err(|error| format!("Live2D asset metadata: {error}"))?
            .len();
        if size > MAX_ASSET_BYTES {
            return Err(format!(
                "Live2D asset exceeds the {MAX_ASSET_BYTES}-byte limit"
            ));
        }
        return tokio::fs::read(canonical_file)
            .await
            .map(tauri::ipc::Response::new)
            .map_err(|error| format!("Live2D asset read failed: {error}"));
    }

    Err(
        "Mk6 model is unavailable. Install Mk6_1.0, set ALICE_LIVE2D_MODEL_DIR, or copy it to the Ayla app-data directory."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::is_safe_relative_path;
    use std::path::Path;

    #[test]
    fn rejects_paths_outside_the_model_root() {
        assert!(is_safe_relative_path(Path::new(
            "Expressions/Happy.exp3.json"
        )));
        assert!(!is_safe_relative_path(Path::new("../secrets")));
        assert!(!is_safe_relative_path(Path::new("/etc/passwd")));
        assert!(!is_safe_relative_path(Path::new("")));
    }
}
