> Historical design record. Runtime and setup instructions are superseded by [README](../README.md) and the [production readiness plan](PRODUCTION_READINESS_PLAN.md).

# AI Assistant Memory Architecture

### Production-Ready Local Memory System

**Target Platform:** Raspberry Pi 5 & Desktop (Rust + Tauri)

---

# Objective

Design a modular, fully local memory architecture capable of supporting:

* Long-term memory
* Short-term memory
* Semantic search
* Knowledge graph reasoning
* Conversation history
* Episodic memory
* Fact extraction
* Memory consolidation
* Future multi-agent support

The system should be:

* Rust-first
* Lightweight
* Fast
* Offline
* Extensible
* Suitable for Raspberry Pi 5
* Independent of any specific LLM

---

# Design Principles

* Memory should not depend on the LLM.
* Every component has a single responsibility.
* The graph should live in memory for speed.
* Persistent storage should be separated from graph processing.
* Vector search should be independent from structured knowledge.
* Every module should be replaceable.
* Everything communicates through well-defined Rust traits.

---

# High-Level Architecture

```text
                           User

                            │

                            ▼

                  Conversation Manager

                            │

                            ▼

                   Memory Pipeline

                            │

        ┌───────────────────┼────────────────────┐

        ▼                   ▼                    ▼

  Memory Extraction    Context Retrieval    Memory Update

        │                   │                    │

        ▼                   ▼                    ▼

 Fact Extractor      Hybrid Retrieval      Memory Consolidator

        │                   │                    │

        ├────────────┐      │      ┌─────────────┤

        ▼            ▼      ▼      ▼             ▼

   SurrealDB     Petgraph  Qdrant  Embedding   Summarizer
```

---

# Memory Layers

## Layer 1 — Working Memory

Purpose

Current conversation only.

Lifetime

* Minutes
* Cleared after conversation ends.

Stores

* Current messages
* Current task
* Active tools
* Active context

Implementation

```text
In-memory Rust structures
```

No persistence.

---

## Layer 2 — Conversation Memory

Purpose

Complete conversation history.

Stores

* user messages
* assistant responses
* timestamps
* session metadata

Persistence

```text
SurrealDB
```

---

## Layer 3 — Episodic Memory

Purpose

Store important events.

Example

```text
User successfully integrated Whisper Base.

Assistant helped configure Sherpa.
```

Stores

* summary
* timestamp
* embedding
* importance score

Persistence

SurrealDB

Vectors

Qdrant

---

## Layer 4 — Semantic Memory

Purpose

Remember permanent knowledge.

Examples

```text
User prefers Rust.

User owns Raspberry Pi 5.

User develops AI assistants.

User uses Sherpa-ONNX.
```

Store

* facts
* confidence
* source
* timestamps

Persistence

SurrealDB

---

## Layer 5 — Knowledge Graph

Purpose

Represent relationships.

Example

```text
User

↓

WORKS_ON

↓

Voice Assistant

↓

USES

↓

Sherpa

↓

SUPPORTED_BY

↓

Whisper Base
```

Runtime

Petgraph

Persistence

SurrealDB

---

## Layer 6 — Vector Memory

Purpose

Semantic retrieval.

Stores

Embeddings only.

Backend

Qdrant

---

# Memory Pipeline

Whenever a conversation ends.

```text
Conversation

↓

Conversation Summarizer

↓

Entity Extraction

↓

Fact Extraction

↓

Relationship Extraction

↓

Importance Scoring

↓

Embedding Generation

↓

Memory Router
```

---

# Memory Router

The router decides where information belongs.

## Facts

```text
↓

SurrealDB
```

---

## Relationships

```text
↓

Petgraph

↓

SurrealDB
```

---

## Embeddings

```text
↓

Qdrant
```

---

## Conversation

```text
↓

SurrealDB
```

---

## Episode

```text
↓

SurrealDB

+

Qdrant
```

---

# Entity Extraction

Prompt

```text
Extract:

• entities
• relationships
• timestamps
• confidence

Return JSON only.
```

Example

```json
{
  "entities": [
    "Mustafa",
    "Sherpa-ONNX",
    "Whisper Base"
  ],
  "relations": [
    {
      "from":"Mustafa",
      "type":"USES",
      "to":"Sherpa-ONNX"
    }
  ]
}
```

---

# Fact Extraction

Example

Conversation

```text
I switched from Whisper Tiny to Whisper Base.
```

Stored

```text
Fact

Subject:
User

Predicate:
uses

Object:
Whisper Base
```

---

# Graph Builder

Responsibilities

* Create nodes
* Update nodes
* Merge duplicates
* Create relationships
* Version relationships
* Track timestamps

Runtime

Petgraph

Persistence

SurrealDB

---

# Memory Consolidation

Runs periodically.

Responsibilities

* Merge duplicate facts
* Remove obsolete memories
* Summarize conversations
* Increase confidence
* Reduce noise

Example

Before

```text
User likes Rust.

User prefers Rust.

User mainly writes Rust.
```

After

```text
User strongly prefers Rust.
```

---

# Importance Scoring

Each memory receives a score.

Example

```text
Importance

0.0

↓

1.0
```

Factors

* User explicitly says "remember"
* Frequently referenced
* Long-term preference
* Current project
* Emotional significance
* Repeated mentions

---

# Embedding Generation

Generate embeddings for

* conversations
* summaries
* documents
* notes
* code snippets

Store only vectors inside Qdrant.

Metadata remains in SurrealDB.

---

# Hybrid Retrieval

When the assistant receives a question.

```text
User Question

↓

Intent Detection

↓

Parallel Retrieval

↓

Graph Search

+

Semantic Search

+

Fact Search

+

Conversation Search

↓

Context Ranking

↓

Context Builder

↓

LLM
```

---

# Graph Search

Petgraph performs

* neighbors
* shortest path
* dependency traversal
* relationship lookup

Everything happens in RAM.

---

# Semantic Search

Qdrant returns

Top-K

Most relevant memories.

---

# Fact Search

SurrealDB returns

* preferences
* settings
* user profile
* permanent knowledge

---

# Context Builder

Merge

* graph results
* semantic results
* facts
* active conversation

Remove duplicates.

Rank by relevance.

Produce the final prompt.

---

# Synchronization

Runtime

```text
Petgraph
```

Persistence

```text
SurrealDB
```

Synchronization

```text
Load graph

↓

Runtime updates

↓

Dirty nodes

↓

Batch save

↓

SurrealDB
```

No database query should be required for every graph traversal.

---

# Memory Manager

Expose only interfaces.

```rust
pub trait MemoryStore {
    async fn save_conversation(&self);

    async fn save_fact(&self);

    async fn save_episode(&self);

    async fn save_graph_update(&self);

    async fn semantic_search(&self);

    async fn graph_search(&self);

    async fn fact_search(&self);

    async fn retrieve_context(&self);
}
```

The rest of the assistant should never know which database is used.

---

# Suggested Rust Modules

```text
memory/

    manager/

    working/

    episodic/

    semantic/

    graph/

    embeddings/

    retrieval/

    extraction/

    summarizer/

    consolidator/

    synchronization/

    ranking/

    context/

database/

    surreal/

vector/

    qdrant/

graph/

    petgraph/

llm/

    ollama/

audio/

stt/

tts/
```

---

# Future Features

The architecture should support:

* Multi-agent memory sharing
* Agent-specific memory namespaces
* Memory expiration
* Memory compression
* Temporal graph queries
* Tool usage history
* User profile evolution
* Project-based memory isolation
* Document memory
* Image memory
* Voice profile memory
* Reinforcement-based memory ranking
* Automatic forgetting
* Knowledge graph visualization
* Local web search cache
* Codebase memory
* Cross-device synchronization

---

# Final Architecture

```text
                              User

                                │

                                ▼

                      Conversation Manager

                                │

                                ▼

                         Memory Manager
                                │
      ┌─────────────────────────┼─────────────────────────┐
      │                         │                         │
      ▼                         ▼                         ▼

 Working Memory           Persistent Memory         Semantic Memory

    (RAM)                  SurrealDB                 Qdrant
      │                         ▲                         ▲
      │                         │                         │
      └──────────────┐          │          ┌──────────────┘
                     ▼          │          ▼

                 Petgraph (Knowledge Graph)

                     ▲

                     │

            Entity & Fact Extraction

                     ▲

                     │

            Conversation Summarizer

                     ▲

                     │

              Sherpa STT → Ollama → Sherpa TTS
```

This design keeps **Petgraph** responsible for high-speed graph reasoning, **SurrealDB** as the durable source of truth for structured memory, and **Qdrant** dedicated to semantic retrieval. The result is a modular architecture that is lightweight enough for a Raspberry Pi 5 while remaining scalable to much more capable AI assistants in the future.
