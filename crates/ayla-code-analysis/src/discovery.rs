use crate::{
    infrastructure_parser, javascript_parser, metadata_parser,
    model::{
        AnalysisControl, AnalysisProgress, Diagnostic, DiagnosticLevel, Entity, GenerateError,
        KnowledgePackage, Relation, SourceScope,
    },
    python_parser, readme_parser, rust_parser,
};
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const MAX_FILES: usize = 20_000;
const MAX_TOTAL_READ_BYTES: u64 = 100 * 1024 * 1024;

pub(crate) fn analyze_directory(
    root: &Path,
    scope: SourceScope,
    control: Option<&AnalysisControl>,
) -> Result<KnowledgePackage, GenerateError> {
    let name = root
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("project")
        .to_owned();
    let mut package = KnowledgePackage::new(name, &directory_fingerprint(root, control)?);
    let repository_uid = package.repository_uid.clone();
    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .build();

    let mut files_seen = 0usize;
    let mut bytes_read = 0u64;
    for entry in walker {
        if let Some(control) = control {
            control.check()?;
        }
        let entry = match entry {
            Ok(value) => value,
            Err(error) => {
                package.diagnostics.push(Diagnostic {
                    level: DiagnosticLevel::Warning,
                    message: error.to_string(),
                });
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        files_seen += 1;
        if files_seen > MAX_FILES {
            return Err(GenerateError::Limit(format!(
                "repository contains more than {MAX_FILES} files"
            )));
        }
        if files_seen == 1 || files_seen % 50 == 0 {
            if let Some(control) = control {
                control.report(AnalysisProgress {
                    stage: "discovering".into(),
                    current: files_seen,
                    total: 0,
                    message: format!("Discovered {files_seen} files"),
                });
            }
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let file_bytes = fs::metadata(path)?.len();
        if excluded(&relative) || file_bytes > 1_000_000 {
            continue;
        }
        bytes_read = bytes_read.saturating_add(file_bytes);
        if bytes_read > MAX_TOTAL_READ_BYTES {
            return Err(GenerateError::Limit(format!(
                "repository analysis exceeds {} MiB",
                MAX_TOTAL_READ_BYTES / 1024 / 1024
            )));
        }
        let is_document = readme_parser::is_document(&relative);
        let is_structured = metadata_parser::is_structured_file(&relative);
        let is_compose = infrastructure_parser::is_compose_file(&relative);
        if scope == SourceScope::DocumentsOnly {
            if is_document || is_structured || is_compose {
                let source = fs::read_to_string(path)?;
                if is_document {
                    readme_parser::analyze_document(&mut package, &relative, &source)?;
                    metadata_parser::enrich_document(&mut package, &relative, &source);
                }
                if is_structured {
                    metadata_parser::analyze_structured_file(&mut package, &relative, &source);
                }
                if is_compose {
                    infrastructure_parser::analyze_compose(&mut package, &relative, &source)?;
                }
            }
            continue;
        }
        if is_document || is_structured || is_compose {
            let source = fs::read_to_string(path)?;
            if is_document {
                readme_parser::analyze_document(&mut package, &relative, &source)?;
                metadata_parser::enrich_document(&mut package, &relative, &source);
            }
            if is_structured {
                metadata_parser::analyze_structured_file(&mut package, &relative, &source);
            }
            if is_compose {
                infrastructure_parser::analyze_compose(&mut package, &relative, &source)?;
            }
            continue;
        }
        match path.extension().and_then(|v| v.to_str()) {
            Some("rs") => {
                rust_parser::analyze_file(&mut package, &relative, &fs::read_to_string(path)?)?
            }
            Some("py") => {
                python_parser::analyze_file(&mut package, &relative, &fs::read_to_string(path)?)?
            }
            Some("js") | Some("mjs") | Some("cjs") => javascript_parser::analyze_file(
                &mut package,
                &relative,
                &fs::read_to_string(path)?,
            )?,
            Some("ts") | Some("tsx") => crate::typescript_parser::analyze_file(
                &mut package,
                &relative,
                &fs::read_to_string(path)?,
            )?,
            _ if useful_metadata(&relative) => {
                let uid = format!("{}:file:{}", repository_uid, relative);
                package.add_entity(Entity {
                    uid: uid.clone(),
                    label: "File".into(),
                    name: relative.clone(),
                    description: Some(
                        "Project manifest, configuration, or documentation file.".into(),
                    ),
                    source_location: Some(relative.clone()),
                    origin: "DISCOVERY".into(),
                    confidence: 1.0,
                    embedding_text: None,
                    properties: Default::default(),
                    source_excerpt: None,
                });
                package.add_relation(Relation {
                    source: repository_uid.clone(),
                    relation: "CONTAINS".into(),
                    target: uid,
                    origin: "DISCOVERY".into(),
                    confidence: 1.0,
                    source_location: Some(relative),
                    properties: Default::default(),
                });
            }
            _ => {}
        }
    }
    metadata_parser::add_system_overview(&mut package);
    package.deduplicate();
    Ok(package)
}

/// Content-based repository identity remains stable when the same checkout is
/// moved to another machine or directory.
fn directory_fingerprint(
    root: &Path,
    control: Option<&AnalysisControl>,
) -> Result<String, GenerateError> {
    let mut files = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let path = entry.into_path();
            let relative = path
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = fs::metadata(&path).ok()?;
            (!excluded(&relative) && metadata.len() <= 1_000_000).then_some((relative, path))
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    let mut bytes_read = 0u64;
    for (index, (relative, path)) in files.into_iter().enumerate() {
        if let Some(control) = control {
            control.check()?;
            if index == 0 || index % 100 == 0 {
                control.report(AnalysisProgress {
                    stage: "fingerprinting".into(),
                    current: index + 1,
                    total: 0,
                    message: format!("Fingerprinting file {}", index + 1),
                });
            }
        }
        digest.update(relative.as_bytes());
        digest.update([0]);
        let contents = fs::read(path)?;
        bytes_read = bytes_read.saturating_add(contents.len() as u64);
        if bytes_read > MAX_TOTAL_READ_BYTES {
            return Err(GenerateError::Limit(format!(
                "repository fingerprint exceeds {} MiB",
                MAX_TOTAL_READ_BYTES / 1024 / 1024
            )));
        }
        digest.update(contents);
        digest.update([0]);
    }
    Ok(format!("{:x}", digest.finalize())[..12].to_string())
}

fn excluded(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts.iter().any(|segment| {
        matches!(
            *segment,
            ".git"
                | "target"
                | "node_modules"
                | "dist"
                | "build"
                | ".venv"
                | "venv"
                | "__pycache__"
                | "coverage"
                | "vendor"
                | "generated"
                | ".cache"
        )
    }) || path.ends_with(".env")
        || (path.contains(".env.")
            && !matches!(
                path.rsplit('/').next().unwrap_or(path),
                ".env.example" | ".env.sample" | ".env.template"
            ))
        || path.ends_with(".pem")
        || path.ends_with(".key")
}

fn useful_metadata(path: &str) -> bool {
    matches!(
        path,
        "Cargo.toml" | "Cargo.lock" | "README.md" | "README" | "LICENSE" | "LICENSE.md"
    ) || path.ends_with(".toml")
        || path.ends_with(".yaml")
        || path.ends_with(".yml")
}
