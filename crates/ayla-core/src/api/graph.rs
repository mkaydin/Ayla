//! Backend graph operations.

use super::*;

impl BackendApi<'_> {
    pub async fn get_kb_data(&self) -> KnowledgeBaseData {
        let graph = self.manager.graph.lock().await;
        let nodes = graph
            .all_nodes()
            .into_iter()
            .map(|node| KnowledgeNode {
                name: node.name.clone(),
                node_type: format!("{:?}", node.node_type),
                source: node.sources.first().cloned().unwrap_or_default(),
                sources: node.sources.clone(),
                aliases: node.aliases.clone(),
                meaning: node.meaning.clone(),
                created: node.created.clone(),
            })
            .collect();
        let edges = graph
            .all_edges()
            .into_iter()
            .map(|(from, relation, to, confidence, sources)| KnowledgeEdge {
                from,
                relation,
                to,
                confidence,
                sources,
            })
            .collect();
        KnowledgeBaseData {
            nodes,
            edges,
            // The UI retrieves file records through `get_files`; preserve the
            // historical `get_kb_data` response shape with an empty list here.
            files: Vec::new(),
        }
    }

    pub async fn get_graph_v2_data(&self) -> Result<GraphV2ApplicationData, BackendError> {
        let Some(store) = self.manager.graph_v2.as_ref() else {
            return Ok(GraphV2ApplicationData::unavailable());
        };
        store
            .lock()
            .await
            .application_data()
            .await
            .map_err(|error| BackendError::new("graph_v2_read", error))
    }

    pub async fn get_graph_v2_page(
        &self,
        kind: &str,
        offset: usize,
        limit: usize,
    ) -> Result<GraphV2PageData, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        store
            .lock()
            .await
            .application_page(kind, offset, limit)
            .await
            .map_err(|error| BackendError::new("graph_v2_read", error))
    }

    pub async fn migrate_legacy_graph_v2(&self) -> Result<LegacyGraphV2Migration, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let (nodes, edges) = {
            let graph = self.manager.graph.lock().await;
            (
                graph.all_nodes().into_iter().cloned().collect::<Vec<_>>(),
                graph.all_edges(),
            )
        };
        let source_id = "legacy-mirror:v1".to_string();
        let batch = GraphImportBatch::from_legacy(source_id.clone(), &nodes, &edges);
        let migrated_nodes = batch.nodes.len();
        let migrated_edges = batch.edges.len();
        let receipt = store
            .lock()
            .await
            .replace_legacy(batch)
            .await
            .map_err(|error| BackendError::new("graph_v2_migration", error))?;
        Ok(LegacyGraphV2Migration {
            source_id,
            migrated_nodes,
            migrated_edges,
            receipt,
        })
    }

    pub async fn prune_graph_v2_backups(
        &self,
        retention: usize,
    ) -> Result<GraphBackupPruneResult, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        store
            .lock()
            .await
            .prune_backups(retention)
            .await
            .map_err(|error| BackendError::new("graph_v2_maintenance", error))
    }

    pub async fn rollback_graph_v2(
        &self,
        backup_id: &str,
    ) -> Result<GraphV2Rollback, BackendError> {
        let store = self.manager.graph_v2.as_ref().ok_or_else(|| {
            BackendError::new(
                "graph_v2_offline",
                "Graph-v2 storage is unavailable while the database is offline",
            )
        })?;
        let receipt = store
            .lock()
            .await
            .rollback(backup_id)
            .await
            .map_err(|error| BackendError::new("graph_v2_rollback", error))?;
        self.manager
            .graph
            .lock()
            .await
            .load()
            .await
            .map_err(BackendError::from)?;
        Ok(GraphV2Rollback {
            target_backup_id: backup_id.into(),
            receipt,
        })
    }
}
