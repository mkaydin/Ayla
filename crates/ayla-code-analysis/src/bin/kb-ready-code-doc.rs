use std::{env, fs, path::PathBuf, process::ExitCode};

use ayla_code_analysis::{
    write_documents, CodeDocumentationGenerator, CodeSource, GenerateOptions, Generator,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let mut input = None;
    let mut output_dir = PathBuf::from(".");
    let mut options = GenerateOptions::default();
    let mut language = None;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output-dir" | "-o" => {
                output_dir = PathBuf::from(required_value(&mut arguments, "--output-dir")?)
            }
            "--model" => options.model = required_value(&mut arguments, "--model")?,
            "--ollama-url" => {
                options.ollama_base_url = required_value(&mut arguments, "--ollama-url")?.parse()?
            }
            "--language" => language = Some(required_value(&mut arguments, "--language")?),
            "--no-semantic" => options.semantic_analysis = false,
            "--semantic" => options.semantic_analysis = true,
            "--allow-remote-git" => options.allow_remote_git = true,
            "--allow-ssh-git" => {
                options.allow_remote_git = true;
                options.allow_ssh_git = true;
            }
            "--semantic-limit" => {
                let value = required_value(&mut arguments, "--semantic-limit")?;
                options.semantic_symbol_limit = Some(
                    value
                        .parse()
                        .map_err(|_| "--semantic-limit must be a non-negative integer")?,
                );
            }
            "--document-semantic-limit" => {
                let value = required_value(&mut arguments, "--document-semantic-limit")?;
                options.semantic_document_limit = Some(
                    value
                        .parse()
                        .map_err(|_| "--document-semantic-limit must be a non-negative integer")?,
                );
            }
            "--full-semantic" => {
                options.semantic_symbol_limit = None;
                options.semantic_document_limit = None;
            }
            "--documents-only" => {
                options.source_scope = ayla_code_analysis::SourceScope::DocumentsOnly
            }
            "--all-files" => options.source_scope = ayla_code_analysis::SourceScope::All,
            "--fast" => options.analysis_mode = ayla_code_analysis::AnalysisMode::Fast,
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}").into())
            }
            value => {
                if input.replace(value.to_owned()).is_some() {
                    return Err("only one input path or Git URL may be supplied".into());
                }
            }
        }
    }

    let input = input.ok_or("missing input path or Git URL; use --help for usage")?;
    let source = source_from_input(&input, language)?;
    eprintln!("[kb-ready-code-doc] input: {input}");
    eprintln!(
        "[kb-ready-code-doc] source scope: {}",
        if options.source_scope == ayla_code_analysis::SourceScope::DocumentsOnly {
            "documents only"
        } else {
            "all project files"
        }
    );
    eprintln!(
        "[kb-ready-code-doc] mode: {}; semantic model: {}",
        if options.semantic_analysis {
            "complete semantic"
        } else {
            "structural-only"
        },
        options.model
    );
    if options.semantic_analysis {
        eprintln!(
            "[kb-ready-code-doc] semantic detail: code {}; documents {}",
            options
                .semantic_symbol_limit
                .map(|limit| format!("prioritized ({limit} code symbols)"))
                .unwrap_or_else(|| "full (all code symbols)".into()),
            options
                .semantic_document_limit
                .map(|limit| format!("prioritized ({limit})"))
                .unwrap_or_else(|| "full".into())
        );
    }
    eprintln!("[kb-ready-code-doc] stage: discovering and parsing source files");
    let documents = Generator::new().generate(source, options).await?;
    eprintln!("[kb-ready-code-doc] stage: writing Markdown documents");
    let (human, kb) = write_documents(&output_dir, &documents)?;
    println!("[kb-ready-code-doc] complete");
    println!("Human documentation: {}", human.display());
    println!("Knowledge-base Markdown: {}", kb.display());
    for diagnostic in documents.diagnostics {
        eprintln!("{:?}: {}", diagnostic.level, diagnostic.message);
    }
    Ok(())
}

fn source_from_input(
    input: &str,
    language: Option<String>,
) -> Result<CodeSource, Box<dyn std::error::Error>> {
    if input.starts_with("https://") || input.starts_with("git@") {
        return Ok(CodeSource::GitRepository {
            url: input.to_owned(),
            branch: None,
        });
    }
    let path = PathBuf::from(input);
    if path.is_dir() {
        return Ok(CodeSource::Directory { path });
    }
    if path.is_file() {
        let code = fs::read_to_string(&path)?;
        let inferred_language = language.or_else(|| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| match extension {
                    "rs" => "rust".to_owned(),
                    value => value.to_owned(),
                })
        });
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("snippet")
            .to_owned();
        return Ok(CodeSource::Snippet {
            code,
            language: inferred_language,
            name,
        });
    }
    Err(format!("input does not exist and is not a supported Git URL: {input}").into())
}

fn required_value(
    arguments: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    arguments
        .next()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn print_usage() {
    println!("Usage: kb-ready-code-doc [OPTIONS] <DIRECTORY | SOURCE_FILE | GIT_URL>");
    println!("\nOptions:");
    println!(
        "  -o, --output-dir <DIR>  Destination for <project>.md and <project>.kb.md (default: .)"
    );
    println!("      --model <MODEL>     Ollama model (default: nemotron-3-super:cloud)");
    println!("      --ollama-url <URL>  Ollama base URL (default: http://localhost:11434)");
    println!("      --language <NAME>   Source-file language override");
    println!("      --no-semantic       Skip Ollama and produce structural documentation only");
    println!("      --semantic-limit N  Interpret the N most important code symbols (default: 36)");
    println!(
        "      --document-semantic-limit N  Interpret the N most important documents (default: 6)"
    );
    println!("      --full-semantic     Interpret every discovered document and code symbol");
    println!("      --documents-only    Analyze documentation files only (default)");
    println!("      --all-files         Analyze documentation, source code, and infrastructure");
    println!("      --fast              Equivalent to structural-only analysis");
}
