//! Durable graph storage, paging, snapshots, and backup retention.

use super::*;

impl GraphV2Store {
    pub fn new(db: Arc<SurrealDB>) -> Self {
        Self { db }
    }

    pub async fn initialize(&self) -> Result<(), String> {
        self.db.ensure_table("graph_node_v2").await?;
        self.db.ensure_table("graph_edge_v2").await?;
        self.db.ensure_table("graph_import_run_v2").await
    }

    pub async fn load(&self) -> Result<GraphSnapshotV2, String> {
        self.initialize().await?;
        let nodes = self
            .db
            .query("SELECT * FROM graph_node_v2;")
            .await?
            .into_iter()
            .map(serde_json::from_value::<GraphNodeV2>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Invalid graph-v2 node record: {error}"))?;
        let edges = self
            .db
            .query("SELECT * FROM graph_edge_v2;")
            .await?
            .into_iter()
            .map(serde_json::from_value::<GraphEdgeV2>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Invalid graph-v2 edge record: {error}"))?;
        let snapshot = GraphSnapshotV2 {
            schema_version: 2,
            nodes: nodes
                .into_iter()
                .map(|node| (node.uid.clone(), node))
                .collect(),
            edges: edges
                .into_iter()
                .map(|edge| (edge.uid.clone(), edge))
                .collect(),
        };
        let report = snapshot.validate();
        if report.is_valid() {
            Ok(snapshot)
        } else {
            Err(format!(
                "Stored graph-v2 snapshot failed validation: {}",
                report.errors.join("; ")
            ))
        }
    }

    /// Return a bounded, read-only application projection. The persistence
    /// snapshot may be much larger; IPC callers never receive an unbounded
    /// graph or audit history response.
    pub async fn application_data(&self) -> Result<GraphV2ApplicationData, String> {
        self.initialize().await?;
        let mut nodes = deserialize_records::<GraphNodeV2>(
            self.db
                .query(&format!(
                    "SELECT * FROM graph_node_v2 ORDER BY updated DESC LIMIT {};",
                    APPLICATION_NODE_LIMIT + 1
                ))
                .await?,
            "node",
        )?;
        let mut edges = deserialize_records::<GraphEdgeV2>(
            self.db
                .query(&format!(
                    "SELECT * FROM graph_edge_v2 ORDER BY updated DESC LIMIT {};",
                    APPLICATION_EDGE_LIMIT + 1
                ))
                .await?,
            "edge",
        )?;
        let mut runs = deserialize_records::<GraphImportRunV2>(
            self.db
                .query(&format!(
                    "SELECT * FROM graph_import_run_v2 ORDER BY committed_at DESC LIMIT {};",
                    APPLICATION_RUN_LIMIT + 1
                ))
                .await?,
            "run",
        )?;
        let nodes_truncated = truncate_projection(
            &mut nodes,
            APPLICATION_NODE_LIMIT,
            APPLICATION_NODE_BYTES,
            "node",
        )?;
        let edges_truncated = truncate_projection(
            &mut edges,
            APPLICATION_EDGE_LIMIT,
            APPLICATION_EDGE_BYTES,
            "edge",
        )?;
        let runs_truncated = truncate_projection(
            &mut runs,
            APPLICATION_RUN_LIMIT,
            APPLICATION_RUN_BYTES,
            "run",
        )?;
        Ok(GraphV2ApplicationData {
            available: true,
            nodes,
            edges,
            runs,
            node_limit: APPLICATION_NODE_LIMIT,
            edge_limit: APPLICATION_EDGE_LIMIT,
            run_limit: APPLICATION_RUN_LIMIT,
            nodes_truncated,
            edges_truncated,
            runs_truncated,
        })
    }

    pub async fn context_for_query(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>, String> {
        let terms: Vec<_> = query
            .split_whitespace()
            .filter(|t| t.chars().count() >= 3)
            .take(12)
            .map(str::to_lowercase)
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut vars = Map::new();
        vars.insert("terms".into(), serde_json::json!(terms));
        let rows = self.db.query_with_vars(
            "SELECT * FROM graph_node_v2 WHERE array::any($terms, |$term| string::contains(string::lowercase(name), $term) OR string::contains(string::lowercase(meaning), $term)) ORDER BY confidence DESC LIMIT 20;", vars).await?;
        let nodes: Vec<GraphNodeV2> = deserialize_records(rows, "retrieval node")?;
        let mut blocked = HashSet::new();
        for query in [
            "SELECT name, path FROM kb_file WHERE status IN ['failed', 'deleting'];",
            "SELECT name, path FROM knowledge_operation WHERE status != 'committed';",
        ] {
            for row in self.db.query(query).await? {
                if let Some(name) = row.get("name").and_then(Value::as_str) {
                    blocked.insert(name.to_string());
                }
                if let Some(id) = row
                    .get("path")
                    .and_then(Value::as_str)
                    .and_then(|path| path.strip_prefix("ocr://"))
                    .and_then(|path| path.split('/').next())
                {
                    blocked.insert(format!("ocr:{id}"));
                }
            }
        }
        let active_ocr: HashSet<String> = self
            .db
            .query("SELECT path FROM kb_file WHERE status = 'committed';")
            .await?
            .into_iter()
            .filter_map(|row| {
                row.get("path")
                    .and_then(Value::as_str)
                    .and_then(|path| path.strip_prefix("ocr://"))
                    .and_then(|path| path.split('/').next())
                    .map(|id| format!("ocr:{id}"))
            })
            .collect();
        let approved = |source: &str| {
            if blocked.contains(source) {
                return false;
            }
            if source.starts_with("ocr:") {
                return active_ocr.contains(source);
            }
            if let Some(operation) = source.strip_prefix("code-analysis:") {
                return crate::artifacts::load_code_analysis_run(operation).is_ok_and(|run| {
                    run.status == "approved" && run.graph_approved && run.vectors_indexed
                });
            }
            true
        };
        Ok(nodes
            .into_iter()
            .filter_map(|mut node| {
                let allowed: Vec<_> = node
                    .sources
                    .iter()
                    .filter(|source| approved(source))
                    .cloned()
                    .collect();
                if !node.sources.is_empty() && allowed.is_empty() {
                    return None;
                }
                if allowed.len() != node.sources.len() {
                    node.meaning = node
                        .properties
                        .get("_source_evidence")
                        .and_then(Value::as_object)
                        .and_then(|evidence| {
                            allowed
                                .iter()
                                .find_map(|source| evidence.get(source)?.get("meaning")?.as_str())
                        })
                        .unwrap_or_default()
                        .to_string();
                }
                Some(format!(
                    "[Knowledge source: {}] {}: {}",
                    allowed.join(", "),
                    node.name,
                    crate::memory::safe_truncate(&node.meaning, 1200)
                ))
            })
            .take(limit.min(20))
            .collect())
    }

    pub async fn has_source_run(&self, source_id: &str) -> Result<bool, String> {
        self.initialize().await?;
        let mut vars = Map::new();
        vars.insert("source_id".into(), Value::String(source_id.into()));
        self.db
            .query_with_vars(
                "SELECT source_id FROM graph_import_run_v2 WHERE source_id = $source_id LIMIT 1;",
                vars,
            )
            .await
            .map(|rows| !rows.is_empty())
    }

    pub async fn application_page(
        &self,
        kind: &str,
        offset: usize,
        requested_limit: usize,
    ) -> Result<GraphV2PageData, String> {
        self.initialize().await?;
        if offset > 1_000_000 {
            return Err("Graph-v2 page offset exceeds the application limit".into());
        }
        let (table, order, maximum_limit) = match kind {
            "nodes" => ("graph_node_v2", "updated", 1_000usize),
            "edges" => ("graph_edge_v2", "updated", 2_000usize),
            "runs" => ("graph_import_run_v2", "committed_at", 100usize),
            _ => return Err("Graph-v2 page kind must be nodes, edges, or runs".into()),
        };
        let limit = requested_limit.clamp(1, maximum_limit);
        let records = self
            .db
            .query(&format!(
                "SELECT * FROM {table} ORDER BY {order} DESC LIMIT {} START {offset};",
                limit + 1
            ))
            .await?;
        let mut page = GraphV2PageData {
            kind: kind.into(),
            offset,
            next_offset: offset,
            has_more: false,
            nodes: Vec::new(),
            edges: Vec::new(),
            runs: Vec::new(),
        };
        match kind {
            "nodes" => {
                page.nodes = deserialize_records(records, "node")?;
                page.has_more = truncate_projection(
                    &mut page.nodes,
                    limit,
                    APPLICATION_PAGE_BYTES,
                    "node page",
                )?;
                page.next_offset += page.nodes.len();
            }
            "edges" => {
                page.edges = deserialize_records(records, "edge")?;
                page.has_more = truncate_projection(
                    &mut page.edges,
                    limit,
                    APPLICATION_PAGE_BYTES,
                    "edge page",
                )?;
                page.next_offset += page.edges.len();
            }
            "runs" => {
                page.runs = deserialize_records(records, "run")?;
                page.has_more =
                    truncate_projection(&mut page.runs, limit, APPLICATION_PAGE_BYTES, "run page")?;
                page.next_offset += page.runs.len();
            }
            _ => unreachable!(),
        }
        Ok(page)
    }

    pub async fn import(&self, batch: GraphImportBatch) -> Result<GraphCommitReceipt, String> {
        let _writer = GRAPH_WRITER.lock().await;
        let source_id = batch.source_id.clone();
        let affected_node_count = batch.nodes.len();
        let affected_edge_count = batch.edges.len();
        let mut snapshot = self.load().await?;
        snapshot
            .apply(batch)
            .map_err(|report| format!("Graph import rejected: {}", report.errors.join("; ")))?;
        let receipt = self.commit_unlocked(&snapshot).await?;
        self.record_run(GraphImportRunV2 {
            event_id: uuid::Uuid::new_v4().to_string(),
            action: "import".into(),
            source_id,
            schema_version: 2,
            affected_node_count,
            affected_edge_count,
            node_count: receipt.node_count,
            edge_count: receipt.edge_count,
            backup_id: receipt.backup_id.clone(),
            backup_available: true,
            target_backup_id: None,
            committed_at: chrono::Utc::now().to_rfc3339(),
        })
        .await?;
        Ok(receipt)
    }

    /// Replace one analysis revision with another in a single staged graph
    /// commit. Shared facts retain any provenance contributed by other runs.
    pub async fn replace_source(
        &self,
        previous_source_id: &str,
        batch: GraphImportBatch,
    ) -> Result<GraphCommitReceipt, String> {
        if previous_source_id.trim().is_empty() {
            return Err("previous_source_id must not be empty".into());
        }
        let source_id = batch.source_id.clone();
        if source_id == previous_source_id {
            return self.import(batch).await;
        }
        let _writer = GRAPH_WRITER.lock().await;
        let affected_node_count = batch.nodes.len();
        let affected_edge_count = batch.edges.len();
        let mut snapshot = self.load().await?;
        snapshot.remove_source(previous_source_id);
        snapshot.apply(batch).map_err(|report| {
            format!(
                "Graph source replacement rejected: {}",
                report.errors.join("; ")
            )
        })?;
        let receipt = self.commit_unlocked(&snapshot).await?;
        self.record_run(GraphImportRunV2 {
            event_id: uuid::Uuid::new_v4().to_string(),
            action: "replace".into(),
            source_id,
            schema_version: 2,
            affected_node_count,
            affected_edge_count,
            node_count: receipt.node_count,
            edge_count: receipt.edge_count,
            backup_id: receipt.backup_id.clone(),
            backup_available: true,
            target_backup_id: None,
            committed_at: chrono::Utc::now().to_rfc3339(),
        })
        .await?;
        Ok(receipt)
    }

    /// Replace the deterministic legacy namespace while preserving native
    /// Graph-v2 OCR/code records. This makes repeated backfills idempotent and
    /// propagates deletions from the legacy graph.
    pub async fn replace_legacy(
        &self,
        batch: GraphImportBatch,
    ) -> Result<GraphCommitReceipt, String> {
        let _writer = GRAPH_WRITER.lock().await;
        let source_id = batch.source_id.clone();
        let affected_node_count = batch.nodes.len();
        let affected_edge_count = batch.edges.len();
        let mut snapshot = self.load().await?;
        snapshot.remove_uid_namespace("legacy");
        snapshot.apply(batch).map_err(|report| {
            format!(
                "Legacy graph migration rejected: {}",
                report.errors.join("; ")
            )
        })?;
        let receipt = self.commit_unlocked(&snapshot).await?;
        self.record_run(GraphImportRunV2 {
            event_id: uuid::Uuid::new_v4().to_string(),
            action: "legacy_sync".into(),
            source_id,
            schema_version: 2,
            affected_node_count,
            affected_edge_count,
            node_count: receipt.node_count,
            edge_count: receipt.edge_count,
            backup_id: receipt.backup_id.clone(),
            backup_available: true,
            target_backup_id: None,
            committed_at: chrono::Utc::now().to_rfc3339(),
        })
        .await?;
        Ok(receipt)
    }

    pub async fn remove_source(&self, source_id: &str) -> Result<GraphCommitReceipt, String> {
        if source_id.trim().is_empty() {
            return Err("source_id must not be empty".into());
        }
        let _writer = GRAPH_WRITER.lock().await;
        let mut snapshot = self.load().await?;
        let (affected_node_count, affected_edge_count) = snapshot.remove_source(source_id);
        let receipt = self.commit_unlocked(&snapshot).await?;
        self.record_run(GraphImportRunV2 {
            event_id: uuid::Uuid::new_v4().to_string(),
            action: "remove".into(),
            source_id: source_id.into(),
            schema_version: 2,
            affected_node_count,
            affected_edge_count,
            node_count: receipt.node_count,
            edge_count: receipt.edge_count,
            backup_id: receipt.backup_id.clone(),
            backup_available: true,
            target_backup_id: None,
            committed_at: chrono::Utc::now().to_rfc3339(),
        })
        .await?;
        Ok(receipt)
    }

    pub async fn commit(&self, snapshot: &GraphSnapshotV2) -> Result<GraphCommitReceipt, String> {
        let _writer = GRAPH_WRITER.lock().await;
        self.commit_unlocked(snapshot).await
    }

    async fn commit_unlocked(
        &self,
        snapshot: &GraphSnapshotV2,
    ) -> Result<GraphCommitReceipt, String> {
        let report = snapshot.validate();
        if !report.is_valid() {
            return Err(format!(
                "Graph-v2 snapshot rejected: {}",
                report.errors.join("; ")
            ));
        }
        self.initialize().await?;
        let snapshot_id = uuid::Uuid::new_v4().simple().to_string();
        let node_stage = format!("graph_node_v2_stage_{snapshot_id}");
        let edge_stage = format!("graph_edge_v2_stage_{snapshot_id}");
        let node_backup = format!("graph_node_v2_backup_{snapshot_id}");
        let edge_backup = format!("graph_edge_v2_backup_{snapshot_id}");
        let node_records = snapshot
            .nodes
            .values()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Graph-v2 node serialization failed: {error}"))?;
        let edge_records = snapshot
            .edges
            .values()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Graph-v2 edge serialization failed: {error}"))?;
        let node_batches = snapshot_batches(&node_stage, &node_records, SNAPSHOT_BATCH_BYTES)?;
        let edge_batches = snapshot_batches(&edge_stage, &edge_records, SNAPSHOT_BATCH_BYTES)?;
        for query in node_batches.iter().chain(edge_batches.iter()) {
            if let Err(error) = self.db.query(query).await {
                cleanup_staging(&self.db, &node_stage, &edge_stage).await;
                return Err(format!("Graph-v2 staging failed: {error}"));
            }
        }

        let mut commit = format!(
            "BEGIN TRANSACTION; DEFINE TABLE IF NOT EXISTS {node_backup}; \
             DEFINE TABLE IF NOT EXISTS {edge_backup}; \
             INSERT INTO {node_backup} (SELECT * FROM graph_node_v2) RETURN NONE; \
             INSERT INTO {edge_backup} (SELECT * FROM graph_edge_v2) RETURN NONE; \
             DELETE graph_edge_v2; DELETE graph_node_v2;"
        );
        if !node_records.is_empty() {
            commit.push_str(&format!(
                " INSERT INTO graph_node_v2 (SELECT * FROM {node_stage}) RETURN NONE;"
            ));
        }
        if !edge_records.is_empty() {
            commit.push_str(&format!(
                " INSERT INTO graph_edge_v2 (SELECT * FROM {edge_stage}) RETURN NONE;"
            ));
        }
        commit.push_str(" COMMIT TRANSACTION;");
        if let Err(error) = self.db.query(&commit).await {
            cleanup_staging(&self.db, &node_stage, &edge_stage).await;
            return Err(format!("Graph-v2 commit failed: {error}"));
        }
        cleanup_staging(&self.db, &node_stage, &edge_stage).await;
        Ok(GraphCommitReceipt {
            backup_id: snapshot_id,
            node_count: node_records.len(),
            edge_count: edge_records.len(),
        })
    }

    pub async fn rollback(&self, backup_id: &str) -> Result<GraphCommitReceipt, String> {
        let _writer = GRAPH_WRITER.lock().await;
        validate_backup_id(backup_id)?;
        self.initialize().await?;
        let mut vars = Map::new();
        vars.insert("backup_id".into(), Value::String(backup_id.into()));
        let known_backup = self
            .db
            .query_with_vars(
                "SELECT backup_id, backup_available FROM graph_import_run_v2 \
                 WHERE backup_id = $backup_id LIMIT 1;",
                vars,
            )
            .await?;
        if known_backup.is_empty() {
            return Err("Unknown graph-v2 backup ID".into());
        }
        if known_backup[0]
            .get("backup_available")
            .and_then(Value::as_bool)
            == Some(false)
        {
            return Err("Graph-v2 backup has expired under the retention policy".into());
        }
        let node_backup = format!("graph_node_v2_backup_{backup_id}");
        let edge_backup = format!("graph_edge_v2_backup_{backup_id}");
        let nodes = deserialize_records::<GraphNodeV2>(
            self.db
                .query(&format!("SELECT * FROM {node_backup};"))
                .await?,
            "backup node",
        )?;
        let edges = deserialize_records::<GraphEdgeV2>(
            self.db
                .query(&format!("SELECT * FROM {edge_backup};"))
                .await?,
            "backup edge",
        )?;
        let snapshot = GraphSnapshotV2 {
            schema_version: 2,
            nodes: nodes
                .into_iter()
                .map(|node| (node.uid.clone(), node))
                .collect(),
            edges: edges
                .into_iter()
                .map(|edge| (edge.uid.clone(), edge))
                .collect(),
        };
        let report = snapshot.validate();
        if !report.is_valid() {
            return Err(format!(
                "Graph-v2 backup failed validation: {}",
                report.errors.join("; ")
            ));
        }
        // `commit` first backs up the current graph, making an application
        // rollback itself recoverable.
        let receipt = self.commit_unlocked(&snapshot).await?;
        self.record_run(GraphImportRunV2 {
            event_id: uuid::Uuid::new_v4().to_string(),
            action: "rollback".into(),
            source_id: format!("backup:{backup_id}"),
            schema_version: 2,
            affected_node_count: receipt.node_count,
            affected_edge_count: receipt.edge_count,
            node_count: receipt.node_count,
            edge_count: receipt.edge_count,
            backup_id: receipt.backup_id.clone(),
            backup_available: true,
            target_backup_id: Some(backup_id.into()),
            committed_at: chrono::Utc::now().to_rfc3339(),
        })
        .await?;
        Ok(receipt)
    }

    async fn record_run(&self, run: GraphImportRunV2) -> Result<(), String> {
        self.db.insert("graph_import_run_v2", run).await?;
        if let Err(error) = self.prune_backups(DEFAULT_BACKUP_RETENTION).await {
            log::warn!("Graph-v2 automatic backup pruning failed: {error}");
        }
        Ok(())
    }

    pub async fn prune_backups(
        &self,
        requested_retention: usize,
    ) -> Result<GraphBackupPruneResult, String> {
        self.initialize().await?;
        let retention = requested_retention.clamp(1, 100);
        let rows = self
            .db
            .query(&format!(
                "SELECT backup_id, backup_available, committed_at FROM graph_import_run_v2 \
                 ORDER BY committed_at DESC LIMIT {MAX_BACKUP_HISTORY_SCAN};"
            ))
            .await
            .map_err(|error| format!("Graph-v2 backup history scan failed: {error}"))?;
        let mut seen = HashSet::new();
        let available = rows
            .iter()
            .filter(|row| row.get("backup_available").and_then(Value::as_bool) != Some(false))
            .filter_map(|row| row.get("backup_id").and_then(Value::as_str))
            .filter(|backup_id| validate_backup_id(backup_id).is_ok())
            .filter(|backup_id| seen.insert((*backup_id).to_string()))
            .map(str::to_string)
            .collect::<Vec<_>>();
        let expired = available
            .iter()
            .skip(retention)
            .cloned()
            .collect::<Vec<_>>();
        if !expired.is_empty() {
            let mut maintenance = String::new();
            for backup_id in &expired {
                let node_backup = format!("graph_node_v2_backup_{backup_id}");
                let edge_backup = format!("graph_edge_v2_backup_{backup_id}");
                maintenance.push_str(&format!(
                    "REMOVE TABLE IF EXISTS {node_backup}; REMOVE TABLE IF EXISTS {edge_backup}; "
                ));
            }
            maintenance.push_str(
                "UPDATE graph_import_run_v2 SET backup_available = false \
                 WHERE backup_id IN $backup_ids;",
            );
            let mut vars = Map::new();
            vars.insert(
                "backup_ids".into(),
                Value::Array(expired.iter().cloned().map(Value::String).collect()),
            );
            self.db
                .exec_with_vars(&maintenance, vars)
                .await
                .map_err(|error| format!("Graph-v2 backup maintenance failed: {error}"))?;
        }
        Ok(GraphBackupPruneResult {
            retained: available.len().min(retention),
            pruned: expired.len(),
        })
    }
}
