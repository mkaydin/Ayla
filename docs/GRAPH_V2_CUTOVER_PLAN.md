> Historical design record. Runtime and setup instructions are superseded by [README](../README.md) and the [production readiness plan](PRODUCTION_READINESS_PLAN.md).

# Graph-v2-Only Cutover Plan

Status: audit complete; implementation pending  
Audit date: 2026-08-18  
Scope: Ayla desktop, sidecar, Tauri bridge, memory pipelines, Knowledge Base UI,
OCR, Search, Chat, Code Analyzer, SurrealDB, and Qdrant

## 1. Target outcome

Graph-v2 becomes Ayla's only knowledge-graph implementation and durable graph
schema. Every knowledge producer writes a provenance-preserving
`GraphImportBatch`; every graph consumer reads UID-keyed Graph-v2 data. The old
name-keyed Petgraph/SurrealDB graph, its mirror migration, legacy UI mode, and
legacy maintenance commands are removed.

A user-managed snapshot service protects the complete knowledge state—not just
graph tables—so users can create, verify, restore, pin, export, and delete known
good versions of Ayla's knowledge and mind data without producing cross-store
corruption.

The cutover must preserve the separation between:

- graph knowledge in `graph_node_v2`, `graph_edge_v2`, and
  `graph_import_run_v2`;
- generic document retrieval vectors in Qdrant `documents`;
- code-analysis vectors in Qdrant `code_analysis`;
- episodic and fact memory, which are not saved KB documents;
- source files and research downloads, which are not deleted when a KB record
  is removed.

## 2. Current persisted-data inventory

The 2026-08-18 read-only inventory found:

| Store | Current data | Cutover treatment |
| :--- | ---: | :--- |
| `kb_file` | 6 records | Delete saved KB records |
| `text_store` | 8 records | Delete duplicated saved-document text |
| Qdrant `documents` | 300 points | Delete and recreate empty |
| `graph_node` | 746 records | Remove table after cutover |
| `graph_edge` | 1,661 records | Remove table after cutover |
| Graph-v2 legacy namespace | 746 nodes / 1,661 edges | Remove from Graph-v2 |
| Native Graph-v2 code import | 169 nodes / 283 edges | Preserve |
| Qdrant `code_analysis` | 195 points | Preserve |
| `graph_import_run_v2` | 5 legacy syncs / 1 native import | Delete legacy syncs; preserve native import |
| Graph-v2 backup pairs | 6 generations | Retire after creating and verifying one sealed cutover snapshot |
| Search sessions/documents | 4 / 54 | Preserve history and downloads; reset KB-ingested flags only |
| Chat/conversation records | separate | Preserve |
| OCR/code artifacts | separate | Preserve unless their own run is explicitly deleted |

Original files referenced by `kb_file`, including files under `~/Documents`,
are outside the destructive cleanup scope. Ayla-owned search downloads also
remain unless a separate search-history purge is requested.

## 3. Compatibility findings

### Critical: split-brain graph writes

Code Analyzer approval and direct OCR graph approval write Graph-v2. Generic
document ingestion, Search, Chat-to-KB, automatic conversation memory, tool
ingestion, deletion, retrieval neighbors, consolidation, and AI cleanup still
write or read `KnowledgeGraph`. The startup mirror is not a live dual-write; it
only backfills legacy data when a marker is missing. Both graphs therefore
diverge after startup.

### Critical: legacy data is self-recreating

`MemoryManager::init_stores` creates and loads `graph_node`/`graph_edge`, then
creates `legacy-mirror:v1` when its run marker is absent. Deleting legacy tables
before removing this code merely recreates an empty legacy system, and the next
generic ingestion repopulates it.

### Critical: imported code knowledge is not available to Chat retrieval

Code Analyzer stores 195 vectors in the dedicated `code_analysis` collection,
but `HybridRetrieval` searches only generic documents, episodes, facts, and the
legacy graph. Approved code knowledge is visible in the KB panel but is not
retrieved into Ayla's chat context.

### Critical: generic document commits are not atomic

Search and Quick Extract call `save_file_record` and `ingest_text` separately.
The first call can commit file metadata and vectors while graph extraction or
the second call fails. OCR has separate "ingest KB document" and "approve
Graph-v2" actions, allowing duplicate or partial state.

### High: delete is incomplete and legacy-only

`delete_file_record` removes `kb_file`, in-memory text, legacy provenance, and
generic vectors, but does not immediately persist the `text_store` deletion.
A restart can reload deleted text. It also cannot remove native Graph-v2
provenance because generic documents do not yet have Graph-v2 source IDs.

### High: code run deletion leaves Graph-v2 audit history

Run-level deletion removes graph provenance, vectors, artifacts, and analyzer
history, but Graph-v2 import events and their backup tables remain. This does
not meet the run-level deletion contract.

### High: KB UI silently masks Graph-v2 read failures

The KB panel uses `Promise.allSettled` for legacy and v2 reads and does not
surface a Graph-v2 read error. It can silently show the legacy graph when v2 is
offline. The stored Legacy/Graph-v2 selector, legacy backfill button, and
legacy-only AI cleanup keep the obsolete model user-visible.

### High: Graph-v2 lacks workspace ingestion and retrieval adapters

There is no adapter from the common extractor result to stable workspace UIDs,
no stable source-ID policy for Chat/Search/Quick Extract, and no Graph-v2
neighbor-query API for hybrid retrieval.

### High: current backups are not knowledge snapshots

The current Graph-v2 backup tables contain only graph nodes and edges. They do
not capture document metadata/text, Qdrant vectors, run records, facts,
episodes, or referenced OCR/code artifacts. Restoring one of these graph-only
backups can therefore create dangling provenance, missing retrieval vectors,
or graph/document disagreement. Backup validity is not checksummed, and users
cannot name, pin, verify, export, or inspect a backup before restoring it.

### Medium: Graph-v2 mutation cost and pagination

Every import loads and rewrites the entire graph and creates a full backup. That
is appropriate for explicit OCR/code approval but too expensive for every
conversation turn. Offset pagination ordered by mutable timestamps can skip or
repeat rows if the graph changes between pages.

### Medium: legacy type ownership

`GraphNodeV2` imports `NodeType` from `graph.rs`, so the new implementation
cannot compile after the old graph module is removed. Extraction also exposes
the legacy type directly.

### Medium: reset consistency across panels

Deleting KB documents must reset `search_session.knowledge_ingested` and change
ingested `search_document` rows back to `processed`; otherwise Search keeps its
"In KB" state and prevents a clean re-import. KB, OCR, Search, Chat, and Code
Analyzer must all receive the same refresh/invalidation event after a commit or
delete.

## 4. Graph-v2-only architecture

### 4.1 Shared graph types

Move `NodeType` into a graph-neutral `memory::graph_types` module. Extraction,
Graph-v2 adapters, cleanup, and UI DTOs use that type. Delete the old
`KnowledgeGraph`, `GraphNode`, and `GraphEdge` types after all consumers move.

### 4.2 Stable UID and source-ID policy

Use opaque stable hashes, never display names, as identities:

| Producer | Source ID | Entity UID namespace |
| :--- | :--- | :--- |
| Saved document/Search/Quick Extract | `document:<document-id>` | `knowledge:<type>:<normalized-name-hash>` |
| OCR run | `ocr:<operation-id>` | existing OCR evidence/concept UIDs |
| Code Analyzer run | `code-analysis:<operation-id>` | existing repository/entity UIDs |
| Explicit Chat-to-KB | `chat-document:<document-id>` | `knowledge:<type>:<normalized-name-hash>` |
| Automatic conversation memory | `conversation:<session-id>:<turn-id>` | `memory:<type>:<normalized-name-hash>` |
| Tool result | `tool:<tool>:<operation-id>` | `knowledge:<type>:<normalized-name-hash>` |

Cross-document canonical knowledge may share stable `knowledge:` UIDs while
retaining multiple sources. Repository-local code symbols keep their existing
repository-scoped UIDs.

### 4.3 Graph-v2 store capabilities

Add bounded, tested operations for:

- atomic source import and source replacement;
- source removal with optional audit/backup erasure;
- incremental conversation batches without a full snapshot backup per turn;
- name/alias/property search and bounded neighbor expansion for retrieval;
- cursor pagination using `(updated, uid)` rather than mutable offsets;
- Graph-v2-native merge/refine cleanup with checkpoint validation;
- complete backup pruning, including retention zero for an authorized reset;
- a dry-run and execute reset report with before/after counts.

Explicit user imports keep rollback backups. Automatic conversation writes use
an append/upsert transaction and audit event without full graph snapshots.

### 4.4 One knowledge-document commit API

Replace the `save_file_record` + `ingest_text` pair with one backend command:

1. validate and assign a stable document ID;
2. extract facts/entities/relations;
3. build and validate the Graph-v2 batch;
4. generate document vectors;
5. stage Graph-v2, vector, document metadata, and text changes;
6. commit all durable stores with compensation on failure;
7. emit one KB refresh event and return one receipt.

Deletion uses the same document ID to remove Graph-v2 provenance, Qdrant
vectors, document metadata, and text together. It never deletes the original
source file.

### 4.5 Knowledge snapshot service

Introduce `KnowledgeSnapshotService` as the only user-facing backup/restore
system. Existing per-commit graph backups remain short-lived internal rollback
checkpoints and are not presented as safe knowledge snapshots.

Snapshot scopes:

| Scope | Included state |
| :--- | :--- |
| Graph | Graph-v2 nodes, edges, import events, schema metadata, and cleanup checkpoints |
| Knowledge | Graph scope plus document registry/text, `documents` and `code_analysis` vectors, analyzer/OCR run metadata, and referenced artifacts |
| Mind | Knowledge scope plus facts, episodes, episode vectors, conversations, and chat sessions/messages |

Knowledge is the default scope. Graph-only snapshots are useful for cheap
diagnostics but must display a warning that they cannot independently restore
document or vector consistency. Mind snapshots require an additional privacy
confirmation because they contain conversation and personal-memory data.

Each completed snapshot has an immutable manifest containing:

- snapshot ID, optional user name/description, scope, state, creation time,
  Ayla version, Graph-v2 schema version, and parent snapshot ID;
- SurrealDB table generations and record counts;
- Qdrant collection snapshots, dimensions, point counts, and embedding model;
- referenced filesystem artifact paths relative to Ayla's data directory;
- SHA-256 hashes for every exported database file, vector snapshot, artifact,
  and the manifest itself;
- graph validation results, dangling-reference counts, and cross-store parity
  checks;
- retention state (`automatic`, `manual`, or `pinned`) and total bytes.

Snapshot creation protocol:

1. acquire a global knowledge-mutation barrier shared by ingestion, deletion,
   analyzer/OCR approval, cleanup, reset, and restore;
2. wait for active mutations to finish, while reads remain available;
3. allocate a private staging directory and write a `creating` manifest;
4. export the selected SurrealDB tables from one consistent generation;
5. create Qdrant snapshots for the selected collections;
6. copy only referenced Ayla-owned artifacts—never arbitrary original source
   files outside the application data directory;
7. hash all components and run graph, provenance, vector, and artifact parity
   validation;
8. atomically rename staging to the final snapshot directory and mark it
   `verified`; otherwise remove staging and leave live data unchanged.

Restore protocol:

1. verify hashes, manifest/schema compatibility, available disk space, graph
   referential integrity, vector dimensions/counts, and required artifacts;
2. show a dry-run diff with added/removed/changed nodes, edges, documents,
   vectors, facts, episodes, conversations, and disk usage;
3. automatically create and verify a pre-restore safety snapshot unless the
   user explicitly selects secure irreversible restore;
4. restore SurrealDB tables and Qdrant collections into staging names;
5. validate the fully staged knowledge state;
6. atomically swap database tables and Qdrant aliases/generations, then restore
   artifacts through an atomic directory swap;
7. roll back every swapped store if any final validation fails;
8. invalidate all panel caches and record a restore audit event that contains
   counts and snapshot IDs but no document content.

Integrity monitoring runs on startup, after every knowledge mutation, before
snapshot creation, and on an optional schedule. It checks:

- Graph-v2 schema and dangling edge endpoints;
- source provenance and import-run consistency;
- document registry/text/vector parity;
- Qdrant collection dimensions, point counts, and required payload keys;
- analyzer/OCR run references and artifact hashes;
- fact/episode record-to-vector consistency for Mind scope;
- incomplete snapshot staging directories and interrupted restores.

Corruption never triggers an automatic destructive restore. Ayla enters a
read-only degraded knowledge mode, identifies the failing component, and offers
the newest verified compatible snapshot. Writes resume only after successful
repair, restore, or explicit user acknowledgement.

Snapshot storage and retention:

- store under `AYLA_DATA_DIR/snapshots/<snapshot-id>` with directory mode
  `0700` and files mode `0600`;
- never store service credentials, environment secrets, Ollama tokens, or
  arbitrary files outside Ayla's data directory;
- support an optional passphrase-encrypted portable export; never retain the
  passphrase;
- default automatic retention: last 5 hourly, 7 daily, and 4 weekly verified
  snapshots, bounded by a configurable disk quota;
- pinned snapshots are never automatically removed;
- deletion removes all components and reports whether space reclamation was
  successful;
- reset offers `create safety snapshot`, `use existing verified snapshot`, or
  `secure purge without snapshot`. Secure purge also removes snapshots that
  contain the selected data and requires typed confirmation.

## 5. Implementation phases

### Phase A — domain decoupling and adapters

- Add `graph_types.rs` and migrate extractor/cleanup type imports.
- Add `GraphImportBatch::from_knowledge_extract` and source/UID helpers.
- Add Graph-v2 retrieval DTOs and source-aware search tests.
- Add Graph-v2 incremental mutation and cursor paging contracts.

Exit gate: Graph-v2 compiles without importing anything from `graph.rs`.

### Phase B — memory runtime cutover

- Replace `MemoryManager.graph` with the Graph-v2 store.
- Remove startup legacy loading and `legacy-mirror:v1` backfill.
- Port `MemoryPipeline` and `IngestionPipeline` to Graph-v2 batches.
- Port `HybridRetrieval` to Graph-v2 and include approved
  `code_analysis` vectors.
- Remove the graph sync step from consolidation; keep fact/episode maintenance.
- Make offline ingestion report that durable KB storage is unavailable instead
  of claiming an ephemeral graph write succeeded.

Exit gate: no runtime path reads or writes `graph_node` or `graph_edge`.

### Phase C — unified knowledge-document lifecycle

- Introduce `commit_knowledge_document`, `delete_knowledge_document`, and
  `reset_knowledge_base` commands.
- Make OCR approval commit its structured Graph-v2 package and document vectors
  in one workflow.
- Move Quick Extract, Search documents, Search knowledge reports, and Chat-to-KB
  to the unified command.
- Preserve research history and downloaded files while resetting KB flags when
  the corresponding KB document is deleted.
- Make code-analysis deletion erase its Graph-v2 audit/backup records as well
  as graph provenance, vectors, artifacts, and analyzer history.

Exit gate: forced failures at each commit stage leave no partial records.

### Phase D — snapshot and integrity foundation

- Add the global mutation barrier and snapshot manifest/state machine.
- Add consistent SurrealDB export/import and Qdrant snapshot adapters.
- Add artifact hashing, cross-store parity checks, staged restore, automatic
  rollback, and startup corruption detection.
- Add commands for snapshot dry-run, create, list, inspect, verify, diff,
  restore, pin/unpin, export/import, delete, and retention cleanup.
- Add failure-injection tests for interruption at every creation/restore stage.

Exit gate: a verified Knowledge snapshot restores into empty staging stores and
produces the same graph/document/vector counts and hashes without modifying live
data until the final swap.

### Phase E — Graph-v2-only KB panel

- Stop calling `get_kb_data`.
- Remove the schema selector, Legacy option, and backfill button.
- Render Graph, Nodes, Edges, Structured, Runs, and Files from Graph-v2 and the
  unified document registry.
- Surface Graph-v2 read errors instead of silently falling back.
- Add source-group shortcuts so an OCR/code/document import can be opened
  directly from its producing panel.
- Port AI Analyze & Improve to UID-keyed Graph-v2 proposals and staged approval.
- Add explicit Refresh and Reset controls with typed destructive confirmation.
- Add a Snapshots tab showing scope, verification state, age, size, counts,
  parent, compatibility, and retention status, with Create, Verify, Compare,
  Restore, Pin, Export, and Delete actions.
- Show integrity status and the newest verified recovery point in the KB header
  and Status panel. A corruption alert links directly to snapshot recovery.

Exit gate: every KB tab works with legacy tables absent.

### Phase F — transport and code removal

- Remove `get_kb_data`, `migrate_legacy_graph_v2`, and legacy-only graph
  commands from core, sidecar, Electron policy, Tauri wrappers, and tests.
- Delete `graph.rs` after its shared types and all behavior have moved.
- Remove legacy re-exports, migration documentation, CSS labels, and test
  fixtures.
- Add a repository guard test that rejects `graph_node`, `graph_edge`,
  `legacy-mirror:v1`, `KnowledgeGraph`, or a Legacy graph selector in runtime
  code.

Exit gate: `rg` finds legacy names only in historical migration documentation,
not compiled/runtime sources.

### Phase G — authorized destructive cleanup

Run only while Ayla is stopped and after Phases A-E pass:

1. create and verify a sealed pre-cutover Knowledge snapshot, unless the user
   selects secure purge without snapshot;
2. dry-run `reset_knowledge_base` and compare counts with this inventory;
3. delete all `kb_file` and `text_store` records;
4. delete all points from Qdrant `documents` and recreate the empty collection;
5. remove `graph_node`, `graph_edge`, and `graph_cleanup_checkpoint` tables;
6. remove all `legacy:` nodes and incident edges from Graph-v2;
7. delete `legacy-mirror:v1` audit events;
8. remove every pre-cutover Graph-v2 backup table and mark/remove its history;
9. reset Search KB-ingested flags without deleting search history/downloads;
10. create and verify a clean-baseline Knowledge snapshot;
11. restart Ayla and verify the old tables are not recreated.

Expected post-cleanup baseline, provided no new imports occur before execution:

- generic saved documents: 0;
- generic document vectors: 0;
- legacy nodes/edges: 0/0;
- Graph-v2 native code-analysis graph: 169 nodes/283 edges;
- code-analysis vectors: 195;
- native code-analysis run: preserved.

With the default sealed safety snapshot, the cleanup is recoverable through the
new snapshot manager even after obsolete graph backup tables are removed. A
secure purge without snapshot is irreversible and requires explicit typed
confirmation. Original external files remain untouched in either mode.

## 6. Validation matrix

| Workflow | Required assertion |
| :--- | :--- |
| Startup with empty DB | Creates only Graph-v2 graph tables; no legacy tables |
| Quick Extract approval | One document record, vectors, v2 source, and audit event |
| Search knowledge/document approval | Same atomic commit; history status reconciles |
| OCR semantic approval | Page evidence and concepts appear once; vectors searchable |
| Code Analyzer approval | Graph/source/vector counts match; Chat retrieves code context |
| Chat-to-KB | Durable Graph-v2 source and document vectors; visible immediately |
| Automatic conversation memory | Incremental v2 mutation without full backup churn |
| Delete document | Graph provenance, vectors, metadata, and text disappear together |
| Delete analyzer/OCR run | Graph, vectors, artifacts, audit, and backups disappear together |
| Graph cleanup | UID-safe staged changes preserve provenance and rollback |
| DB/Qdrant failure injection | No false success and no orphaned partial commit |
| KB read failure | Visible error; never silent legacy fallback |
| Reset | Exact dry-run counts, typed confirmation, zero legacy residue |
| Snapshot create | Mutation barrier, hashes, counts, and cross-store parity all verified |
| Snapshot interruption | Live state unchanged; incomplete staging detected and removable |
| Snapshot restore dry-run | Exact graph/document/vector/mind diff without mutation |
| Snapshot restore | Staged validation and atomic swap; automatic rollback on injected failure |
| Corruption detection | Knowledge writes blocked; newest compatible verified snapshot offered |
| Retention | Pinned snapshots preserved; quota and hourly/daily/weekly rules deterministic |

Required checks:

```bash
cargo test --workspace
cargo check --manifest-path src-tauri/Cargo.toml
npm test
npm run build
npm run sidecar:diagnostic:live
npm run sidecar:diagnostic:ocr
npm run sidecar:diagnostic:code
```

Add live diagnostics for generic document commit/delete, Graph-v2 retrieval,
cross-panel refresh, snapshot create/verify/restore/rollback, reset dry-run, and
proof that legacy tables are not recreated after restart.

## 7. Execution rule

Do not execute the destructive cleanup against the current binary. Complete
the runtime, snapshot, and UI cutover first; otherwise current Ayla code can
recreate the legacy graph and split new imports across both schemas. The
snapshot service must pass its failure-injection and restore-to-staging gates
before any legacy or saved-document cleanup is authorized.
