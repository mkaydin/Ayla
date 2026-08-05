// ── Ayla's auto-generated process status messages ───────────

export function generateAylaResponse(phase, process, data) {
  if (process === 'ocr') {
    if (phase === 'start') {
      return `[neutral] Processing document: ${data.filename}\u2014${data.pages} pages. This may take ${data.estimate || 'a few minutes'}.`;
    }
    if (phase === 'complete') {
      return `[neutral] Document ${data.filename} extracted\u2014${data.chars} characters. It is ready for review in the OCR panel and can be added to our knowledge base after approval.`;
    }
    if (phase === 'error') {
      return `[confused] OCR failed for ${data.filename}: ${data.reason}. The document may be damaged or the OCR model may be unavailable.`;
    }
  }

  if (process === 'search') {
    if (phase === 'start') {
      return `[neutral] Searching for "${data.query}" across academic and web sources. I will report when the analysis is ready.`;
    }
    if (phase === 'progress' && data.current === 1) {
      return `[neutral] Search update: found ${data.results} sources. Analyzing the ${data.total} most relevant results.`;
    }
    if (phase === 'complete') {
      return `[surprised] Research complete for "${data.query}". Analyzed ${data.papers} papers with ${data.themes} cross-cutting themes. The synthesis is available in the Search panel.`;
    }
    if (phase === 'error') {
      return `[confused] Search for "${data.query}" failed: ${data.reason}. Check the search services or refine the query.`;
    }
  }

  if (process === 'kb-ingest') {
    if (phase === 'complete' && data.nodes != null) {
      return `[neutral] Knowledge base updated. Added ${data.nodes} nodes, ${data.edges} relations, and ${data.facts} facts. The graph now contains ${data.totalNodes} nodes and ${data.totalEdges} edges.`;
    }
    if (phase === 'complete') {
      return `[neutral] ${data.source || 'Document'} added to the knowledge base.`;
    }
    if (phase === 'error') {
      return `[confused] Knowledge base update failed: ${data.reason}. The source remains available so we can retry after the underlying service recovers.`;
    }
  }

  if (process === 'model') {
    if (phase === 'error') {
      return `[confused] The ${data.model} model is not responding. Check that Ollama is running before retrying this work.`;
    }
    if (phase === 'complete') {
      return `[neutral] ${data.model} is responsive. Normal operation has resumed.`;
    }
  }

  if (process === 'memory') {
    if (phase === 'error') {
      return `[confused] Memory database ${data.db} is unreachable. I am using working memory only until the database reconnects.`;
    }
    if (phase === 'complete') {
      return `[neutral] Memory database ${data.db} is available again.`;
    }
  }

  return null;
}
