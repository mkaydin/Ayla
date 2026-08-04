// ── Alice's auto-generated process status messages ──────────

export function generateAliceResponse(phase, process, data) {
  if (process === 'ocr') {
    if (phase === 'start') {
      return `[neutral] Processing document: ${data.filename}\u2014${data.pages} pages. This will take approximately ${data.estimate || 'a few minutes'}.`;
    }
    if (phase === 'complete') {
      return `[neutral] Document ${data.filename} extracted\u2014${data.chars} characters. The content is ready for review in the OCR panel.
You can ask me questions about it once you approve it for my knowledge base.`;
    }
    if (phase === 'error') {
      return `[angry] OCR failed for ${data.filename}: ${data.reason}. The document may be corrupted or the vision model is unavailable. Check that deepseek-ocr is pulled in Ollama.`;
    }
  }

  if (process === 'search') {
    if (phase === 'start') {
      return `[neutral] Searching for "${data.query}" across academic databases. This will take several minutes. I will notify you when the analysis is ready.`;
    }
    if (phase === 'progress' && data.current === 1) {
      return `[neutral] Search update: found ${data.results} papers. Beginning analysis of ${data.total} most relevant sources.`;
    }
    if (phase === 'complete') {
      return `[amazed] Research complete for "${data.query}". Analyzed ${data.papers} papers with ${data.themes} cross-cutting themes. View the full synthesis in the Search panel.
Once you approve the report, I can ingest all findings into my knowledge base for future queries.`;
    }
    if (phase === 'error') {
      return `[angry] Search for "${data.query}" failed: ${data.reason}. SearXNG may be unavailable or the query needs refinement. Check that Docker containers are running.`;
    }
  }

  if (process === 'kb-ingest') {
    if (phase === 'complete' && data.nodes != null) {
      return `[neutral] Knowledge base updated. Added ${data.nodes} nodes, ${data.edges} relations, and ${data.facts} facts. Total knowledge graph now contains ${data.totalNodes} nodes and ${data.totalEdges} edges.`;
    }
    if (phase === 'complete') {
      return `[neutral] ${data.source || 'Document'} added to the knowledge base.`;
    }
    if (phase === 'error') {
      return `[angry] Knowledge base update failed: ${data.reason}. The data is preserved locally; I will retry automatically when the database recovers.`;
    }
  }

  if (process === 'model') {
    if (phase === 'error') {
      return `[angry] The ${data.model} model is not responding. Check that Ollama is running. I cannot process requests until the model is available.`;
    }
    if (phase === 'complete') {
      return `[neutral] ${data.model} is responsive. Resuming normal operation.`;
    }
  }

  if (process === 'memory') {
    if (phase === 'error') {
      return `[nervous] Memory database ${data.db} is unreachable. I am operating with working memory only\u2014past knowledge is temporarily unavailable. Docker services may need to be started.`;
    }
    if (phase === 'complete') {
      return `[neutral] Memory database ${data.db} restored. All knowledge accessible again.`;
    }
  }

  return null;
}
