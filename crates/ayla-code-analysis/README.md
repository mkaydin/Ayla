# kb_ready_code_doc

`kb_ready_code_doc` is a Rust crate and CLI that turns a source file, project directory, or Git repository into two complementary Markdown documents:

- `<project>.md`: concise, human-readable project documentation.
- `<project>.kb.md`: a complete, machine-parseable knowledge-base and graph document.

Documentation-only analysis is the default. It gathers project documentation and structured metadata without parsing source code. Use `--all-files` when code-level analysis is needed.

## What it produces

The human document contains:

- an interpreted project overview;
- prioritized project-document summaries;
- a complete compact documentation inventory;
- architecture and important code entities when source analysis is enabled;
- declared dependencies, licenses, configuration keys, and analysis diagnostics.

The KB-ready document contains:

- an explicit graph ontology;
- stable entity UIDs and typed relations;
- provenance, confidence, source locations, and structured JSON properties;
- deterministic repository, document, manifest, dependency, license, configuration, and infrastructure facts;
- Nemotron-interpreted descriptions, embedding text, concepts, and concept relationships;
- semantic coverage metadata and deterministic fallbacks for units that were not interpreted.

## Supported inputs

- A local project or repository directory.
- A public HTTPS or SSH Git repository URL.
- A single source file, treated as a code snippet.
- A snippet supplied through the library API.

In `--all-files` mode, Tree-sitter currently provides structural parsing for Rust, Python, JavaScript, and TypeScript/TSX.

## Requirements

- A recent stable Rust toolchain.
- Git when analyzing a remote repository.
- Ollama at `http://localhost:11434` for semantic interpretation.
- The `nemotron-3-super:cloud` Ollama model, unless another model is selected.

Check the configured model:

```bash
ollama list
```

Deterministic generation works without Ollama by passing `--no-semantic`.

## Quick start

Generate both outputs from project documents and metadata using Nemotron:

```bash
cargo run --release --bin kb-ready-code-doc -- \
  ./my-project \
  --output-dir ./output
```

Generate the same complete deterministic graph without an LLM call:

```bash
cargo run --release --bin kb-ready-code-doc -- \
  ./my-project \
  --output-dir ./output \
  --no-semantic
```

Analyze documentation, metadata, infrastructure, and supported source code:

```bash
cargo run --release --bin kb-ready-code-doc -- \
  ./my-project \
  --output-dir ./output \
  --all-files
```

Analyze a remote repository:

```bash
cargo run --release --bin kb-ready-code-doc -- \
  https://github.com/owner/repository \
  --output-dir ./output
```

## Analysis modes

| Mode | Selection | Behavior |
| :--- | :--- | :--- |
| Documents only | Default or `--documents-only` | Documentation, manifests, license, safe configuration metadata, Compose infrastructure, and external IDs; source code is excluded. |
| All files | `--all-files` | Adds supported source-code AST entities and import relationships. |
| Deterministic | `--no-semantic` | Skips Ollama while retaining all discovered facts and fallback embedding text. |
| Reduced semantic | `--document-semantic-limit N`, `--semantic-limit N` | Controls the number of prioritized documents and code symbols sent to Ollama. |
| Exhaustive semantic | `--full-semantic` | Interprets every discovered document and supported code symbol. This can be slow and costly. |

By default, semantic mode interprets the repository overview, the synthesized system overview, six prioritized documents, and up to 36 prioritized code symbols in `--all-files` mode. Every discovered entity remains in the KB document even when it is outside the semantic limit or an Ollama request fails.

## Deterministic document and metadata extraction

The default pass recognizes:

- README, Markdown/MDX, reStructuredText, AsciiDoc, and text documents;
- changelogs, contribution guides, security policies, notices, authors, and licenses;
- `requirements*.txt`, `pyproject.toml`, `package.json`, `Cargo.toml`, and `go.mod` dependencies;
- dependency ecosystem, canonical package identity, scope, and version constraints;
- `.env.example`, `.env.sample`, and `.env.template` key names;
- Docker Compose services, image references, tags, and environment key names;
- license type, copyright notice, and detected years;
- MITRE ATT&CK IDs in document bodies or filenames;
- explicit MITRE tactic, tool, technique, and playbook relationships;
- a top-level `SystemOverview` anchor node.

Environment and Compose values are deliberately not stored. Only safe key names and structural facts are emitted.

## Semantic graph behavior

The default semantic model is `nemotron-3-super:cloud`. It generates compact purposes, responsibilities, domain concepts, and a constrained set of concept-to-concept relations.

Concept handling includes:

- filtering language primitives and generic syntax terms;
- tiered confidence based on direct source evidence versus inference;
- project-level UID namespacing;
- canonical identity across casing, whitespace, punctuation, and CamelCase variants;
- retained aliases and multiple source locations;
- resolution of package names to deterministic `DependencyPackage` nodes instead of duplicate generic concepts.

The crate generates `embeddingText` suitable for a later vector-ingestion step. It does not currently calculate or store embedding vectors, so `qwen3-embedding:0.6b` is not required during document generation.

## CLI reference

```text
Usage: kb-ready-code-doc [OPTIONS] <DIRECTORY | SOURCE_FILE | GIT_URL>

Options:
  -o, --output-dir <DIR>          Output directory
      --model <MODEL>             Ollama model
      --ollama-url <URL>          Ollama base URL
      --language <NAME>           Source-file language override
      --no-semantic               Skip Ollama
      --semantic-limit N          Maximum prioritized code symbols
      --document-semantic-limit N Maximum prioritized documents
      --full-semantic             Interpret all documents and code symbols
      --documents-only            Select the default documentation mode
      --all-files                 Include supported source-code analysis
      --fast                      Skip semantic interpretation
```

Run `cargo run --bin kb-ready-code-doc -- --help` for the current built-in help.

## Library integration

```rust,no_run
use std::path::PathBuf;

use kb_ready_code_doc::{
    write_documents, CodeDocumentationGenerator, CodeSource, GenerateOptions, Generator,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let generator = Generator::new();
    let documents = generator
        .generate(
            CodeSource::Directory {
                path: PathBuf::from("./my-project"),
            },
            GenerateOptions::default(),
        )
        .await?;

    write_documents(&PathBuf::from("./output"), &documents)?;
    Ok(())
}
```

AYLA can implement the public `LlmClient` trait to route semantic requests through its own model gateway while reusing the deterministic extraction and Markdown rendering pipeline.

## Validation

Run the test suite:

```bash
cargo test
```

The saved COM example is available at:

- [`output/com.md`](output/com.md)
- [`output/com.kb.md`](output/com.kb.md)

That KB output covers 67 documents, with six prioritized documents interpreted by `nemotron-3-super:cloud`; all other discovered facts remain present through the deterministic layer.
