//! Validated UID-keyed graph records and migration adapters.
//!
//! This module is deliberately side-effect free. It lets Ayla prepare and
//! validate v2 snapshots before enabling any database migration or structured
//! analyzer write path.

use crate::db::surreal::SurrealDB;
use crate::memory::graph::GraphNode;
use crate::memory::graph_types::NodeType;

static GRAPH_WRITER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
use ayla_code_analysis::GeneratedDocuments;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

const MAX_GRAPH_RECORD_BYTES: usize = 64 * 1024;
const MAX_UID_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 128;
const SNAPSHOT_BATCH_BYTES: usize = 512 * 1024;
const APPLICATION_NODE_LIMIT: usize = 2_000;
const APPLICATION_EDGE_LIMIT: usize = 4_000;
const APPLICATION_RUN_LIMIT: usize = 100;
const APPLICATION_NODE_BYTES: usize = 8 * 1024 * 1024;
const APPLICATION_EDGE_BYTES: usize = 8 * 1024 * 1024;
const APPLICATION_RUN_BYTES: usize = 1024 * 1024;
const APPLICATION_PAGE_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_BACKUP_RETENTION: usize = 25;
const MAX_BACKUP_HISTORY_SCAN: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNodeV2 {
    pub uid: String,
    pub name: String,
    pub label: String,
    pub node_type: NodeType,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub meaning: String,
    #[serde(default)]
    pub origins: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub source_locations: Vec<String>,
    pub confidence: f32,
    #[serde(default)]
    pub properties: Map<String, Value>,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdgeV2 {
    pub uid: String,
    pub from_uid: String,
    pub to_uid: String,
    pub relation: String,
    #[serde(default)]
    pub origins: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub source_locations: Vec<String>,
    pub confidence: f32,
    #[serde(default)]
    pub properties: Map<String, Value>,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphImportBatch {
    pub schema_version: u8,
    pub source_id: String,
    pub nodes: Vec<GraphNodeV2>,
    pub edges: Vec<GraphEdgeV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphValidationReport {
    pub node_count: usize,
    pub edge_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphCommitReceipt {
    pub backup_id: String,
    pub node_count: usize,
    pub edge_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphImportRunV2 {
    #[serde(default)]
    pub event_id: String,
    #[serde(default = "default_import_action")]
    pub action: String,
    #[serde(default)]
    pub source_id: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u8,
    #[serde(default)]
    pub affected_node_count: usize,
    #[serde(default)]
    pub affected_edge_count: usize,
    #[serde(default)]
    pub node_count: usize,
    #[serde(default)]
    pub edge_count: usize,
    #[serde(default)]
    pub backup_id: String,
    #[serde(default = "default_backup_available")]
    pub backup_available: bool,
    #[serde(default)]
    pub target_backup_id: Option<String>,
    #[serde(default)]
    pub committed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphV2PageData {
    pub kind: String,
    pub offset: usize,
    pub next_offset: usize,
    pub has_more: bool,
    pub nodes: Vec<GraphNodeV2>,
    pub edges: Vec<GraphEdgeV2>,
    pub runs: Vec<GraphImportRunV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphBackupPruneResult {
    pub retained: usize,
    pub pruned: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphV2ApplicationData {
    pub available: bool,
    pub nodes: Vec<GraphNodeV2>,
    pub edges: Vec<GraphEdgeV2>,
    pub runs: Vec<GraphImportRunV2>,
    pub node_limit: usize,
    pub edge_limit: usize,
    pub run_limit: usize,
    pub nodes_truncated: bool,
    pub edges_truncated: bool,
    pub runs_truncated: bool,
}

impl GraphV2ApplicationData {
    pub fn unavailable() -> Self {
        Self {
            available: false,
            nodes: Vec::new(),
            edges: Vec::new(),
            runs: Vec::new(),
            node_limit: APPLICATION_NODE_LIMIT,
            edge_limit: APPLICATION_EDGE_LIMIT,
            run_limit: APPLICATION_RUN_LIMIT,
            nodes_truncated: false,
            edges_truncated: false,
            runs_truncated: false,
        }
    }
}

pub struct GraphV2Store {
    db: Arc<SurrealDB>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GraphSnapshotV2 {
    pub schema_version: u8,
    pub nodes: BTreeMap<String, GraphNodeV2>,
    pub edges: BTreeMap<String, GraphEdgeV2>,
}

impl GraphValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

impl GraphImportBatch {
    pub fn from_ocr_knowledge(
        source_id: impl Into<String>,
        knowledge: &ayla_ocr::OcrKnowledgePackage,
        timestamp: &str,
    ) -> Self {
        let source_id = source_id.into();
        let nodes = knowledge
            .entities
            .iter()
            .map(|entity| GraphNodeV2 {
                uid: entity.uid.clone(),
                name: entity.name.clone(),
                label: entity.label.clone(),
                node_type: coarse_node_type(&entity.label),
                aliases: Vec::new(),
                meaning: entity.description.clone(),
                origins: vec!["OCR_DETERMINISTIC".into()],
                sources: vec![source_id.clone()],
                source_locations: nonempty_values([entity.source_location.clone()]),
                confidence: entity.confidence,
                properties: entity.properties.clone(),
                created: timestamp.into(),
                updated: timestamp.into(),
            })
            .collect();
        let edges = knowledge
            .relations
            .iter()
            .map(|relation| GraphEdgeV2 {
                uid: stable_edge_uid(
                    &relation.source,
                    &relation.relation,
                    &relation.target,
                    Some(&relation.source_location),
                ),
                from_uid: relation.source.clone(),
                to_uid: relation.target.clone(),
                relation: normalize_relation(&relation.relation),
                origins: vec!["OCR_DETERMINISTIC".into()],
                sources: vec![source_id.clone()],
                source_locations: nonempty_values([relation.source_location.clone()]),
                confidence: relation.confidence,
                properties: relation.properties.clone(),
                created: timestamp.into(),
                updated: timestamp.into(),
            })
            .collect();
        Self {
            schema_version: 2,
            source_id,
            nodes,
            edges,
        }
    }

    pub fn from_code_analysis(
        source_id: impl Into<String>,
        analyzed: &GeneratedDocuments,
        timestamp: &str,
    ) -> Self {
        let source_id = source_id.into();
        let nodes = analyzed
            .entities
            .iter()
            .map(|entity| GraphNodeV2 {
                uid: entity.uid.clone(),
                name: entity.name.clone(),
                label: entity.label.clone(),
                node_type: coarse_node_type(&entity.label),
                aliases: Vec::new(),
                meaning: entity.description.clone().unwrap_or_default(),
                origins: nonempty_values([entity.origin.clone()]),
                sources: nonempty_values([source_id.clone()]),
                source_locations: nonempty_values(entity.source_location.clone()),
                confidence: entity.confidence,
                properties: entity.properties.clone().into_iter().collect(),
                created: timestamp.to_string(),
                updated: timestamp.to_string(),
            })
            .collect();
        let edges = analyzed
            .relations
            .iter()
            .map(|relation| GraphEdgeV2 {
                uid: stable_edge_uid(
                    &relation.source,
                    &relation.relation,
                    &relation.target,
                    relation.source_location.as_deref(),
                ),
                from_uid: relation.source.clone(),
                to_uid: relation.target.clone(),
                relation: normalize_relation(&relation.relation),
                origins: nonempty_values([relation.origin.clone()]),
                sources: nonempty_values([source_id.clone()]),
                source_locations: nonempty_values(relation.source_location.clone()),
                confidence: relation.confidence,
                properties: relation.properties.clone().into_iter().collect(),
                created: timestamp.to_string(),
                updated: timestamp.to_string(),
            })
            .collect();
        Self {
            schema_version: 2,
            source_id,
            nodes,
            edges,
        }
    }

    pub fn from_legacy(
        source_id: impl Into<String>,
        nodes: &[GraphNode],
        edges: &[(String, String, String, f32, Vec<String>)],
    ) -> Self {
        let source_id = source_id.into();
        let mut uid_by_name = HashMap::new();
        let migrated_nodes = nodes
            .iter()
            .map(|node| {
                let uid = stable_legacy_uid(&node.name, &node.node_type);
                uid_by_name.insert(node.name.to_lowercase(), uid.clone());
                GraphNodeV2 {
                    uid,
                    name: node.name.clone(),
                    label: format!("{:?}", node.node_type),
                    node_type: node.node_type.clone(),
                    aliases: node.aliases.clone(),
                    meaning: node.meaning.clone(),
                    origins: vec![if source_id == "workspace" {
                        "WORKSPACE"
                    } else {
                        "LEGACY_MIGRATION"
                    }
                    .into()],
                    sources: node.sources.clone(),
                    source_locations: Vec::new(),
                    confidence: 0.8,
                    properties: Map::new(),
                    created: node.created.clone(),
                    updated: node.updated.clone(),
                }
            })
            .collect();
        let mut migrated_edges: BTreeMap<String, GraphEdgeV2> = BTreeMap::new();
        for incoming in edges
            .iter()
            .filter_map(|(from, relation, to, confidence, sources)| {
                let from_uid = uid_by_name.get(&from.to_lowercase())?.clone();
                let to_uid = uid_by_name.get(&to.to_lowercase())?.clone();
                Some(GraphEdgeV2 {
                    uid: stable_edge_uid(&from_uid, relation, &to_uid, None),
                    from_uid,
                    to_uid,
                    relation: normalize_relation(relation),
                    origins: vec![if source_id == "workspace" {
                        "WORKSPACE"
                    } else {
                        "LEGACY_MIGRATION"
                    }
                    .into()],
                    sources: sources.clone(),
                    source_locations: Vec::new(),
                    confidence: *confidence,
                    properties: Map::new(),
                    created: String::new(),
                    updated: String::new(),
                })
            })
        {
            if let Some(existing) = migrated_edges.get_mut(&incoming.uid) {
                merge_edge(existing, incoming);
            } else {
                migrated_edges.insert(incoming.uid.clone(), incoming);
            }
        }
        Self {
            schema_version: 2,
            source_id,
            nodes: migrated_nodes,
            edges: migrated_edges.into_values().collect(),
        }
    }

    pub fn validate(&self) -> GraphValidationReport {
        let mut errors = Vec::new();
        if self.schema_version != 2 {
            errors.push("schema_version must be 2".into());
        }
        if self.source_id.trim().is_empty() {
            errors.push("source_id must not be empty".into());
        }
        let mut node_ids = HashSet::new();
        for node in &self.nodes {
            validate_identifier("node uid", &node.uid, MAX_UID_BYTES, &mut errors);
            validate_identifier("node label", &node.label, MAX_LABEL_BYTES, &mut errors);
            if node.name.trim().is_empty() {
                errors.push(format!("node {} has an empty name", node.uid));
            }
            if !node_ids.insert(node.uid.as_str()) {
                errors.push(format!("duplicate node uid: {}", node.uid));
            }
            validate_confidence("node", &node.uid, node.confidence, &mut errors);
            validate_record_size("node", &node.uid, node, &mut errors);
        }
        let mut edge_ids = HashSet::new();
        for edge in &self.edges {
            validate_identifier("edge uid", &edge.uid, MAX_UID_BYTES, &mut errors);
            if !edge_ids.insert(edge.uid.as_str()) {
                errors.push(format!("duplicate edge uid: {}", edge.uid));
            }
            if !node_ids.contains(edge.from_uid.as_str()) {
                errors.push(format!(
                    "edge {} references missing source {}",
                    edge.uid, edge.from_uid
                ));
            }
            if !node_ids.contains(edge.to_uid.as_str()) {
                errors.push(format!(
                    "edge {} references missing target {}",
                    edge.uid, edge.to_uid
                ));
            }
            if edge.from_uid == edge.to_uid {
                errors.push(format!("edge {} is a self-loop", edge.uid));
            }
            if edge.relation.is_empty() || edge.relation.len() > 96 {
                errors.push(format!("edge {} has an invalid relation", edge.uid));
            }
            validate_confidence("edge", &edge.uid, edge.confidence, &mut errors);
            validate_record_size("edge", &edge.uid, edge, &mut errors);
        }
        GraphValidationReport {
            node_count: self.nodes.len(),
            edge_count: self.edges.len(),
            errors,
        }
    }
}

impl GraphSnapshotV2 {
    pub fn new() -> Self {
        Self {
            schema_version: 2,
            nodes: BTreeMap::new(),
            edges: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, batch: GraphImportBatch) -> Result<(), GraphValidationReport> {
        let report = batch.validate();
        if !report.is_valid() {
            return Err(report);
        }
        for mut incoming in batch.nodes {
            if incoming.sources.len() == 1 {
                let evidence = node_evidence(&incoming);
                incoming.properties.insert(
                    "_source_evidence".into(),
                    serde_json::json!({incoming.sources[0].clone(): evidence}),
                );
            }
            match self.nodes.get_mut(&incoming.uid) {
                Some(existing) => merge_node(existing, incoming),
                None => {
                    self.nodes.insert(incoming.uid.clone(), incoming);
                }
            }
        }
        for incoming in batch.edges {
            match self.edges.get_mut(&incoming.uid) {
                Some(existing) => merge_edge(existing, incoming),
                None => {
                    self.edges.insert(incoming.uid.clone(), incoming);
                }
            }
        }
        Ok(())
    }

    pub fn remove_source(&mut self, source_id: &str) -> (usize, usize) {
        let edges_before = self.edges.len();
        self.edges.retain(|_, edge| {
            if !edge.sources.iter().any(|source| source == source_id) {
                return true;
            }
            edge.sources.retain(|source| source != source_id);
            !edge.sources.is_empty()
        });
        let nodes_before = self.nodes.len();
        self.nodes.retain(|_, node| {
            if !node.sources.iter().any(|source| source == source_id) {
                return true;
            }
            node.sources.retain(|source| source != source_id);
            // Aggregated descriptions/properties may belong to the removed source.
            // Rebuild them only from surviving attributed evidence.
            let mut evidence = node
                .properties
                .remove("_source_evidence")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            evidence.retain(|source, _| node.sources.contains(source));
            node.meaning.clear();
            node.aliases.clear();
            node.source_locations.clear();
            node.properties.clear();
            for contribution in evidence.values() {
                if node.meaning.is_empty() {
                    node.meaning = contribution["meaning"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                }
                let strings = |key: &str| {
                    contribution[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                };
                node.aliases = merge_values(&node.aliases, strings("aliases"));
                node.source_locations = merge_values(&node.source_locations, strings("locations"));
                if let Some(properties) = contribution["properties"].as_object() {
                    node.properties.extend(properties.clone());
                }
            }
            if !evidence.is_empty() {
                node.properties
                    .insert("_source_evidence".into(), Value::Object(evidence));
            }
            !node.sources.is_empty()
        });
        self.edges.retain(|_, edge| {
            self.nodes.contains_key(&edge.from_uid) && self.nodes.contains_key(&edge.to_uid)
        });
        (
            nodes_before - self.nodes.len(),
            edges_before - self.edges.len(),
        )
    }

    pub fn remove_uid_namespace(&mut self, namespace: &str) -> (usize, usize) {
        let prefix = format!("{namespace}:");
        let nodes_before = self.nodes.len();
        let edges_before = self.edges.len();
        self.nodes.retain(|uid, _| !uid.starts_with(&prefix));
        self.edges.retain(|_, edge| {
            !edge.from_uid.starts_with(&prefix)
                && !edge.to_uid.starts_with(&prefix)
                && self.nodes.contains_key(&edge.from_uid)
                && self.nodes.contains_key(&edge.to_uid)
        });
        (
            nodes_before - self.nodes.len(),
            edges_before - self.edges.len(),
        )
    }

    pub fn validate(&self) -> GraphValidationReport {
        GraphImportBatch {
            schema_version: self.schema_version,
            source_id: "snapshot-validation".into(),
            nodes: self.nodes.values().cloned().collect(),
            edges: self.edges.values().cloned().collect(),
        }
        .validate()
    }
}

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

fn default_import_action() -> String {
    "import".into()
}

fn default_schema_version() -> u8 {
    2
}

fn default_backup_available() -> bool {
    true
}

fn validate_backup_id(backup_id: &str) -> Result<(), String> {
    if backup_id.len() == 32
        && backup_id
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err("Invalid graph-v2 backup ID".into())
    }
}

fn deserialize_records<T: for<'de> Deserialize<'de>>(
    records: Vec<Value>,
    kind: &str,
) -> Result<Vec<T>, String> {
    records
        .into_iter()
        .map(serde_json::from_value::<T>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Invalid graph-v2 {kind} record: {error}"))
}

fn truncate_projection<T: Serialize>(
    records: &mut Vec<T>,
    limit: usize,
    max_bytes: usize,
    kind: &str,
) -> Result<bool, String> {
    let original_len = records.len();
    let mut retained = 0usize;
    let mut bytes = 2usize; // JSON array brackets; MessagePack is similarly bounded.
    for record in records.iter().take(limit) {
        let record_bytes = serde_json::to_vec(record)
            .map_err(|error| format!("Graph-v2 {kind} projection failed: {error}"))?
            .len();
        let separator = usize::from(retained > 0);
        if bytes + separator + record_bytes > max_bytes {
            break;
        }
        bytes += separator + record_bytes;
        retained += 1;
    }
    records.truncate(retained);
    Ok(retained < original_len)
}

fn snapshot_batches(
    table: &str,
    records: &[Value],
    max_bytes: usize,
) -> Result<Vec<String>, String> {
    if !table
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err("Invalid graph-v2 staging table name".into());
    }
    let prefix = format!("INSERT INTO {table} [");
    let suffix = "] RETURN NONE;";
    let mut batches = Vec::new();
    let mut batch = prefix.clone();
    let mut count = 0usize;
    for record in records {
        let mut record = record.clone();
        crate::db::surreal::sanitize_value_for_storage(&mut record);
        let encoded = serde_json::to_string(&record).map_err(|error| error.to_string())?;
        if prefix.len() + encoded.len() + suffix.len() > max_bytes {
            return Err("One graph-v2 record exceeds the persistence batch limit".into());
        }
        if batch.len() + usize::from(count > 0) + encoded.len() + suffix.len() > max_bytes {
            batch.push_str(suffix);
            batches.push(batch);
            batch = prefix.clone();
            count = 0;
        }
        if count > 0 {
            batch.push(',');
        }
        batch.push_str(&encoded);
        count += 1;
    }
    if count > 0 {
        batch.push_str(suffix);
        batches.push(batch);
    }
    Ok(batches)
}

async fn cleanup_staging(db: &SurrealDB, nodes: &str, edges: &str) {
    if let Err(error) = db
        .query(&format!(
            "REMOVE TABLE IF EXISTS {nodes}; REMOVE TABLE IF EXISTS {edges};"
        ))
        .await
    {
        log::warn!("Graph-v2 staging cleanup failed: {error}");
    }
}

fn node_evidence(node: &GraphNodeV2) -> Value {
    let mut properties = node.properties.clone();
    properties.remove("_source_evidence");
    serde_json::json!({"meaning": node.meaning, "aliases": node.aliases, "locations": node.source_locations, "properties": properties})
}

fn merge_node(existing: &mut GraphNodeV2, incoming: GraphNodeV2) {
    let mut evidence = existing
        .properties
        .get("_source_evidence")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if evidence.is_empty() && existing.sources.len() == 1 {
        evidence.insert(existing.sources[0].clone(), node_evidence(existing));
    }
    if let Some(additions) = incoming
        .properties
        .get("_source_evidence")
        .and_then(Value::as_object)
    {
        evidence.extend(additions.clone());
    }
    if !incoming.name.eq_ignore_ascii_case(&existing.name)
        && !existing
            .aliases
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(&incoming.name))
    {
        existing.aliases.push(incoming.name);
    }
    existing.aliases = merge_values(&existing.aliases, incoming.aliases);
    existing.origins = merge_values(&existing.origins, incoming.origins);
    existing.sources = merge_values(&existing.sources, incoming.sources);
    existing.source_locations = merge_values(&existing.source_locations, incoming.source_locations);
    if existing.meaning.is_empty() && !incoming.meaning.is_empty() {
        existing.meaning = incoming.meaning;
    }
    if matches!(existing.node_type, NodeType::Concept | NodeType::Other)
        && !matches!(incoming.node_type, NodeType::Concept | NodeType::Other)
    {
        existing.node_type = incoming.node_type;
        existing.label = incoming.label;
    }
    existing.confidence = existing.confidence.max(incoming.confidence);
    existing.properties.extend(incoming.properties);
    if !evidence.is_empty() {
        existing
            .properties
            .insert("_source_evidence".into(), Value::Object(evidence));
    }
    existing.updated = incoming.updated.max(existing.updated.clone());
    existing.created = incoming.created.min(existing.created.clone());
}

fn merge_edge(existing: &mut GraphEdgeV2, incoming: GraphEdgeV2) {
    existing.origins = merge_values(&existing.origins, incoming.origins);
    existing.sources = merge_values(&existing.sources, incoming.sources);
    existing.source_locations = merge_values(&existing.source_locations, incoming.source_locations);
    existing.confidence = existing.confidence.max(incoming.confidence);
    existing.properties.extend(incoming.properties);
    existing.updated = incoming.updated.max(existing.updated.clone());
    existing.created = incoming.created.min(existing.created.clone());
}

fn merge_values(existing: &[String], incoming: Vec<String>) -> Vec<String> {
    let mut values = existing.to_vec();
    let mut seen = values.iter().cloned().collect::<HashSet<_>>();
    values.extend(
        incoming
            .into_iter()
            .filter(|value| seen.insert(value.clone())),
    );
    values
}

fn validate_identifier(kind: &str, value: &str, max: usize, errors: &mut Vec<String>) {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        errors.push(format!(
            "{kind} is empty, oversized, or contains control characters"
        ));
    }
}

fn validate_confidence(kind: &str, uid: &str, confidence: f32, errors: &mut Vec<String>) {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        errors.push(format!("{kind} {uid} confidence must be between 0 and 1"));
    }
}

fn validate_record_size<T: Serialize>(kind: &str, uid: &str, record: &T, errors: &mut Vec<String>) {
    match serde_json::to_vec(record) {
        Ok(bytes) if bytes.len() > MAX_GRAPH_RECORD_BYTES => errors.push(format!(
            "{kind} {uid} exceeds the {MAX_GRAPH_RECORD_BYTES}-byte record limit"
        )),
        Err(error) => errors.push(format!("{kind} {uid} cannot be serialized: {error}")),
        _ => {}
    }
}

fn coarse_node_type(label: &str) -> NodeType {
    match label.to_ascii_lowercase().as_str() {
        "person" | "author" | "researcher" => NodeType::Person,
        "repository" | "project" | "package" | "module" => NodeType::Project,
        "dependency" | "tool" | "library" | "framework" | "database" => NodeType::Tool,
        "file" | "document" | "documentsection" | "readme" | "license" => NodeType::Document,
        "other" => NodeType::Other,
        _ => NodeType::Concept,
    }
}

fn stable_legacy_uid(name: &str, node_type: &NodeType) -> String {
    stable_uid(
        "legacy",
        &format!("{:?}:{}", node_type, name.trim().to_lowercase()),
    )
}

fn stable_edge_uid(from: &str, relation: &str, to: &str, location: Option<&str>) -> String {
    stable_uid(
        "edge",
        &format!(
            "{from}|{}|{to}|{}",
            normalize_relation(relation),
            location.unwrap_or_default()
        ),
    )
}

fn stable_uid(namespace: &str, value: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!("{namespace}:{}", &digest[..24])
}

fn normalize_relation(value: &str) -> String {
    value
        .trim()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .take(8)
        .map(str::to_uppercase)
        .collect::<Vec<_>>()
        .join("_")
}

fn nonempty_values(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty() && seen.insert(value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayla_code_analysis::{Diagnostic, Entity, Relation};
    use ayla_ocr::{OcrKnowledgeEntity, OcrKnowledgePackage, OcrKnowledgeRelation};
    use std::collections::BTreeMap;

    fn analyzer_result() -> GeneratedDocuments {
        GeneratedDocuments {
            project_name: "fixture".into(),
            repository_uid: "repo:fixture".into(),
            entities: vec![
                Entity {
                    uid: "repo:fixture:function:a".into(),
                    label: "Function".into(),
                    name: "parse".into(),
                    description: Some("Parses format A".into()),
                    source_location: Some("src/a.rs:4".into()),
                    origin: "TREE_SITTER".into(),
                    confidence: 1.0,
                    embedding_text: None,
                    properties: BTreeMap::new(),
                    source_excerpt: Some("secret source is never imported".into()),
                },
                Entity {
                    uid: "repo:fixture:function:b".into(),
                    label: "Function".into(),
                    name: "parse".into(),
                    description: Some("Parses format B".into()),
                    source_location: Some("src/b.rs:8".into()),
                    origin: "TREE_SITTER".into(),
                    confidence: 1.0,
                    embedding_text: None,
                    properties: BTreeMap::new(),
                    source_excerpt: None,
                },
            ],
            relations: vec![Relation {
                source: "repo:fixture:function:a".into(),
                relation: "calls".into(),
                target: "repo:fixture:function:b".into(),
                origin: "TREE_SITTER".into(),
                confidence: 0.95,
                source_location: Some("src/a.rs:9".into()),
                properties: BTreeMap::new(),
            }],
            human_markdown: String::new(),
            knowledge_base_markdown: String::new(),
            diagnostics: Vec::<Diagnostic>::new(),
        }
    }

    #[test]
    fn analyzer_adapter_preserves_same_named_entities_by_uid() {
        let batch = GraphImportBatch::from_code_analysis(
            "code-run:fixture",
            &analyzer_result(),
            "2026-08-17T00:00:00Z",
        );
        let report = batch.validate();
        assert!(report.is_valid(), "{:?}", report.errors);
        assert_eq!(batch.nodes.len(), 2);
        assert_eq!(batch.nodes[0].name, batch.nodes[1].name);
        assert_ne!(batch.nodes[0].uid, batch.nodes[1].uid);
        assert!(!serde_json::to_string(&batch)
            .unwrap()
            .contains("secret source"));
    }

    #[test]
    fn ocr_adapter_preserves_page_citations_and_valid_endpoints() {
        let knowledge = OcrKnowledgePackage {
            schema_version: 1,
            source_name: "paper.pdf".into(),
            document_uid: "ocr-document:paper".into(),
            knowledge_base_markdown: "# Paper".into(),
            entities: vec![
                OcrKnowledgeEntity {
                    uid: "ocr-document:paper".into(),
                    label: "Document".into(),
                    name: "paper.pdf".into(),
                    description: "A paper".into(),
                    source_location: "paper.pdf".into(),
                    confidence: 0.98,
                    properties: Map::new(),
                },
                OcrKnowledgeEntity {
                    uid: "ocr-document:paper:page:1".into(),
                    label: "DocumentSection".into(),
                    name: "paper.pdf — Page 1".into(),
                    description: "Evidence".into(),
                    source_location: "paper.pdf#page=1".into(),
                    confidence: 0.95,
                    properties: Map::new(),
                },
            ],
            relations: vec![OcrKnowledgeRelation {
                source: "ocr-document:paper:page:1".into(),
                relation: "PART_OF".into(),
                target: "ocr-document:paper".into(),
                source_location: "paper.pdf#page=1".into(),
                confidence: 1.0,
                properties: Map::new(),
            }],
            warnings: vec![],
        };
        let batch =
            GraphImportBatch::from_ocr_knowledge("ocr:fixture", &knowledge, "2026-08-17T00:00:00Z");
        assert!(batch.validate().is_valid());
        assert_eq!(batch.nodes[1].node_type, NodeType::Document);
        assert_eq!(batch.nodes[1].source_locations, ["paper.pdf#page=1"]);
        assert_eq!(batch.edges[0].relation, "PART_OF");
        assert_eq!(batch.edges[0].origins, ["OCR_DETERMINISTIC"]);
    }

    #[test]
    fn validation_rejects_missing_endpoints() {
        let mut batch = GraphImportBatch::from_code_analysis(
            "code-run:fixture",
            &analyzer_result(),
            "2026-08-17T00:00:00Z",
        );
        batch.edges[0].to_uid = "missing".into();
        assert!(batch
            .validate()
            .errors
            .iter()
            .any(|error| error.contains("missing target")));
    }

    #[test]
    fn legacy_equivalent_relations_merge_without_losing_sources() {
        let nodes: Vec<_> = ["Sensor", "Pressure"]
            .into_iter()
            .map(|name| GraphNode {
                name: name.into(),
                node_type: NodeType::Concept,
                sources: vec!["a.md".into(), "b.md".into()],
                aliases: Vec::new(),
                meaning: String::new(),
                created: "old".into(),
                updated: "new".into(),
            })
            .collect();
        let edges = vec![
            (
                "Sensor".into(),
                "has pressure".into(),
                "Pressure".into(),
                0.7,
                vec!["a.md".into()],
            ),
            (
                "Sensor".into(),
                "HAS_PRESSURE".into(),
                "Pressure".into(),
                0.9,
                vec!["b.md".into()],
            ),
        ];
        let batch = GraphImportBatch::from_legacy("migration", &nodes, &edges);
        assert!(batch.validate().is_valid());
        assert_eq!(batch.edges.len(), 1);
        assert_eq!(batch.edges[0].sources, ["a.md", "b.md"]);
        assert_eq!(batch.edges[0].confidence, 0.9);
    }

    #[test]
    fn legacy_uids_are_stable() {
        let node = GraphNode {
            name: "Ayla".into(),
            node_type: NodeType::Project,
            sources: vec!["profile.md".into()],
            aliases: Vec::new(),
            meaning: String::new(),
            created: "old".into(),
            updated: "new".into(),
        };
        let first = GraphImportBatch::from_legacy("migration", &[node.clone()], &[]);
        let second = GraphImportBatch::from_legacy("migration", &[node], &[]);
        assert_eq!(first.nodes[0].uid, second.nodes[0].uid);
        assert!(first.validate().is_valid());
    }

    #[test]
    fn reanalysis_merges_provenance_without_duplicate_uids() {
        let analyzed = analyzer_result();
        let first = GraphImportBatch::from_code_analysis(
            "code-run:first",
            &analyzed,
            "2026-08-17T00:00:00Z",
        );
        let second = GraphImportBatch::from_code_analysis(
            "code-run:second",
            &analyzed,
            "2026-08-17T01:00:00Z",
        );
        let mut snapshot = GraphSnapshotV2::new();
        snapshot.apply(first).unwrap();
        snapshot.apply(second).unwrap();
        assert_eq!(snapshot.nodes.len(), 2);
        assert_eq!(snapshot.edges.len(), 1);
        assert_eq!(snapshot.nodes.values().next().unwrap().sources.len(), 2);
        assert!(snapshot.validate().is_valid());
    }

    #[test]
    fn deleting_one_run_preserves_shared_graph_evidence() {
        let analyzed = analyzer_result();
        let mut snapshot = GraphSnapshotV2::new();
        snapshot
            .apply(GraphImportBatch::from_code_analysis(
                "code-run:first",
                &analyzed,
                "2026-08-17T00:00:00Z",
            ))
            .unwrap();
        snapshot
            .apply(GraphImportBatch::from_code_analysis(
                "code-run:second",
                &analyzed,
                "2026-08-17T01:00:00Z",
            ))
            .unwrap();

        assert_eq!(snapshot.remove_source("code-run:first"), (0, 0));
        assert_eq!(snapshot.nodes.len(), 2);
        assert_eq!(snapshot.edges.len(), 1);
        assert_eq!(snapshot.remove_source("code-run:second"), (2, 1));
        assert!(snapshot.nodes.is_empty());
        assert!(snapshot.edges.is_empty());
    }

    #[test]
    fn deletion_removes_a_sources_description_and_private_properties_from_shared_nodes() {
        let mut analyzed = analyzer_result();
        analyzed.entities[0].description = Some("Deleted source private evidence".into());
        analyzed.entities[0]
            .properties
            .insert("private_property".into(), Value::String("remove-me".into()));
        let mut snapshot = GraphSnapshotV2::new();
        snapshot
            .apply(GraphImportBatch::from_code_analysis(
                "first",
                &analyzed,
                "2026-10-13T00:00:00Z",
            ))
            .unwrap();
        analyzed.entities[0].description = Some("Remaining source evidence".into());
        analyzed.entities[0].properties.clear();
        snapshot
            .apply(GraphImportBatch::from_code_analysis(
                "second",
                &analyzed,
                "2026-10-13T01:00:00Z",
            ))
            .unwrap();
        snapshot.remove_source("first");
        let node = &snapshot.nodes["repo:fixture:function:a"];
        assert_eq!(node.meaning, "Remaining source evidence");
        assert!(!serde_json::to_string(node).unwrap().contains("remove-me"));
        assert!(!serde_json::to_string(node)
            .unwrap()
            .contains("Deleted source private evidence"));
        assert_eq!(node.sources, vec!["second"]);
    }

    #[test]
    fn legacy_namespace_refresh_preserves_native_graph_v2_records() {
        let analyzed = analyzer_result();
        let legacy = GraphNode {
            name: "Legacy Entity".into(),
            node_type: NodeType::Concept,
            sources: vec!["legacy.md".into()],
            aliases: Vec::new(),
            meaning: String::new(),
            created: "old".into(),
            updated: "new".into(),
        };
        let mut snapshot = GraphSnapshotV2::new();
        snapshot
            .apply(GraphImportBatch::from_code_analysis(
                "code-run:first",
                &analyzed,
                "2026-08-17T00:00:00Z",
            ))
            .unwrap();
        snapshot
            .apply(GraphImportBatch::from_legacy(
                "legacy-mirror:v1",
                &[legacy],
                &[],
            ))
            .unwrap();

        assert_eq!(snapshot.nodes.len(), 3);
        assert_eq!(snapshot.remove_uid_namespace("legacy"), (1, 0));
        assert_eq!(snapshot.nodes.len(), 2);
        assert!(snapshot
            .nodes
            .keys()
            .all(|uid| uid.starts_with("repo:fixture")));
        assert_eq!(snapshot.edges.len(), 1);
    }

    #[test]
    fn legacy_import_history_defaults_to_an_import_event() {
        let run: GraphImportRunV2 = serde_json::from_value(serde_json::json!({
            "source_id": "code-analysis:legacy",
            "schema_version": 2,
            "node_count": 4,
            "edge_count": 3,
            "backup_id": "0123456789abcdef0123456789abcdef",
            "committed_at": "2026-08-17T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(run.action, "import");
        assert!(run.backup_available);
        assert_eq!(run.affected_node_count, 0);
        assert_eq!(run.target_backup_id, None);
    }

    #[test]
    fn application_projection_is_bounded_when_unavailable() {
        let data = GraphV2ApplicationData::unavailable();
        assert!(!data.available);
        assert_eq!(data.node_limit, APPLICATION_NODE_LIMIT);
        assert_eq!(data.edge_limit, APPLICATION_EDGE_LIMIT);
        assert_eq!(data.run_limit, APPLICATION_RUN_LIMIT);
        assert!(data.nodes.is_empty());
    }

    #[test]
    fn projection_truncation_reports_omitted_records() {
        let mut records = vec![1, 2, 3];
        assert!(truncate_projection(&mut records, 2, 1024, "fixture").unwrap());
        assert_eq!(records, [1, 2]);
        assert!(!truncate_projection(&mut records, 2, 1024, "fixture").unwrap());

        let mut records = vec!["0123456789", "next"];
        assert!(truncate_projection(&mut records, 10, 8, "fixture").unwrap());
        assert!(records.is_empty());
    }

    #[test]
    fn backup_ids_are_strictly_validated() {
        assert!(validate_backup_id("0123456789abcdef0123456789abcdef").is_ok());
        assert!(validate_backup_id("../graph_node_v2").is_err());
        assert!(validate_backup_id("0123456789abcdef").is_err());
    }
}
