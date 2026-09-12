// ── Knowledge graph ────────────────────────────────────────
// Layer 5: In-memory Petgraph + SurrealDB persistence.

use crate::db::surreal::SurrealDB;
use crate::memory::graph_v2::{GraphImportBatch, GraphV2Store};
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use super::graph_types::NodeType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub name: String,
    pub node_type: NodeType,
    pub sources: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub meaning: String,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub relation: String,
    pub confidence: f32,
    pub sources: Vec<String>,
    pub created: String,
}

#[derive(Clone)]
pub struct KnowledgeGraph {
    graph: StableDiGraph<GraphNode, GraphEdge>,
    name_to_idx: HashMap<String, NodeIndex>,
    dirty_nodes: HashSet<NodeIndex>,
    db: Option<Arc<SurrealDB>>,
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self {
            graph: StableDiGraph::new(),
            name_to_idx: HashMap::new(),
            dirty_nodes: HashSet::new(),
            db: None,
        }
    }

    pub fn with_db(db: Arc<SurrealDB>) -> Self {
        Self {
            db: Some(db),
            ..Self::new()
        }
    }

    /// Graph-v2 is the only live storage. The in-memory workspace projection
    /// supports deterministic name-based concept cleanup without touching native code/OCR UIDs.
    pub async fn load(&mut self) -> Result<(), String> {
        let Some(db) = self.db.clone() else {
            return Ok(());
        };
        let store = GraphV2Store::new(db.clone());
        store.initialize().await?;
        db.ensure_table("knowledge_schema").await?;
        if db
            .query("SELECT * FROM knowledge_schema WHERE version = 2 LIMIT 1;")
            .await?
            .is_empty()
        {
            // One-time import; original tables remain read-only migration backups.
            let rows = db.query("SELECT * FROM graph_node;").await?;
            for mut row in rows {
                let sources = row_sources(&row);
                row["sources"] = serde_json::json!(sources);
                if row.get("created").is_none() {
                    row["created"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
                }
                if row.get("updated").is_none() {
                    row["updated"] = row["created"].clone();
                }
                let node = serde_json::from_value::<GraphNode>(row).map_err(|error| {
                    format!("Legacy graph migration stopped before changing live data: {error}")
                })?;
                let idx = self.upsert_node_sources(&node.name, node.node_type, &sources);
                self.graph[idx].aliases = node.aliases;
                self.graph[idx].meaning = node.meaning;
                self.graph[idx].created = node.created;
            }
            for row in db.query("SELECT * FROM graph_edge;").await? {
                let from = row
                    .get("from_node")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let to = row
                    .get("to_node")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let relation = row
                    .get("relation")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if from.is_empty() || to.is_empty() || relation.is_empty() {
                    continue;
                }
                let from = self.upsert_node(from, NodeType::Concept);
                let to = self.upsert_node(to, NodeType::Concept);
                self.upsert_edge_sources(
                    from,
                    to,
                    relation,
                    row.get("confidence")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.8) as f32,
                    &row_sources(&row),
                );
            }
            if self.node_count() > 0 {
                store
                    .replace_legacy(GraphImportBatch::from_legacy(
                        "workspace",
                        &self.all_nodes().into_iter().cloned().collect::<Vec<_>>(),
                        &self.all_edges(),
                    ))
                    .await?;
            }
            db.insert("knowledge_schema", serde_json::json!({"version": 2, "migrated_at": chrono::Utc::now().to_rfc3339(), "legacy_tables_retained_as_backup": true})).await?;
        }
        let snapshot = store.load().await?;
        self.graph.clear();
        self.name_to_idx.clear();
        self.dirty_nodes.clear();
        let mut identities = HashMap::new();
        for node in snapshot
            .nodes
            .values()
            .filter(|node| node.uid.starts_with("legacy:"))
        {
            let idx = self.upsert_node_sources(&node.name, node.node_type.clone(), &node.sources);
            self.graph[idx].aliases = node.aliases.clone();
            self.graph[idx].meaning = node.meaning.clone();
            self.graph[idx].created = node.created.clone();
            self.graph[idx].updated = node.updated.clone();
            identities.insert(node.uid.clone(), idx);
        }
        for edge in snapshot.edges.values() {
            if let (Some(from), Some(to)) =
                (identities.get(&edge.from_uid), identities.get(&edge.to_uid))
            {
                self.upsert_edge_sources(
                    *from,
                    *to,
                    &edge.relation,
                    edge.confidence,
                    &edge.sources,
                );
            }
        }
        self.dirty_nodes.clear();
        Ok(())
    }

    pub async fn sync(&mut self) -> Result<(), String> {
        let Some(db) = &self.db else {
            return Ok(());
        };
        let nodes = self.all_nodes().into_iter().cloned().collect::<Vec<_>>();
        let batch = GraphImportBatch::from_legacy("workspace", &nodes, &self.all_edges());
        GraphV2Store::new(db.clone()).replace_legacy(batch).await?;
        self.dirty_nodes.clear();
        Ok(())
    }

    pub fn upsert_node(&mut self, name: &str, node_type: NodeType) -> NodeIndex {
        self.upsert_node_sourced(name, node_type, "")
    }

    pub fn upsert_node_sourced(
        &mut self,
        name: &str,
        node_type: NodeType,
        source: &str,
    ) -> NodeIndex {
        let sources = if source.is_empty() {
            Vec::new()
        } else {
            vec![source.to_string()]
        };
        self.upsert_node_sources(name, node_type, &sources)
    }

    pub fn upsert_node_sources(
        &mut self,
        name: &str,
        node_type: NodeType,
        sources: &[String],
    ) -> NodeIndex {
        let name = name.to_lowercase();
        if let Some(&idx) = self.name_to_idx.get(&name) {
            let node = &mut self.graph[idx];
            node.updated = chrono::Utc::now().to_rfc3339();
            if matches!(node.node_type, NodeType::Concept | NodeType::Other)
                && !matches!(node_type, NodeType::Concept | NodeType::Other)
            {
                node.node_type = node_type;
            }
            for source in sources.iter().filter(|source| !source.is_empty()) {
                if !node.sources.contains(source) {
                    node.sources.push(source.clone());
                }
            }
            self.dirty_nodes.insert(idx);
            return idx;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let node = GraphNode {
            name: name.clone(),
            node_type,
            sources: unique_sources(sources.iter().cloned()),
            aliases: Vec::new(),
            meaning: String::new(),
            created: now.clone(),
            updated: now,
        };
        let idx = self.graph.add_node(node);
        self.name_to_idx.insert(name, idx);
        self.dirty_nodes.insert(idx);
        idx
    }

    pub fn upsert_edge(&mut self, from: NodeIndex, to: NodeIndex, relation: &str, confidence: f32) {
        self.upsert_edge_sources(from, to, relation, confidence, &[]);
    }

    pub fn upsert_edge_sourced(
        &mut self,
        from: NodeIndex,
        to: NodeIndex,
        relation: &str,
        confidence: f32,
        source: &str,
    ) {
        let sources = if source.is_empty() {
            Vec::new()
        } else {
            vec![source.to_string()]
        };
        self.upsert_edge_sources(from, to, relation, confidence, &sources);
    }

    fn upsert_edge_sources(
        &mut self,
        from: NodeIndex,
        to: NodeIndex,
        relation: &str,
        confidence: f32,
        sources: &[String],
    ) {
        if from == to {
            return;
        }
        let Some(relation) = normalize_relation_label(relation) else {
            return;
        };
        let existing = self
            .graph
            .edges_connecting(from, to)
            .find(|edge| {
                normalize_relation_label(&edge.weight().relation).as_ref() == Some(&relation)
            })
            .map(|edge| edge.id());
        if let Some(edge_idx) = existing {
            let edge = &mut self.graph[edge_idx];
            edge.relation = relation;
            edge.confidence = edge.confidence.max(confidence);
            for source in sources.iter().filter(|source| !source.is_empty()) {
                if !edge.sources.contains(source) {
                    edge.sources.push(source.clone());
                }
            }
            return;
        }
        self.graph.add_edge(
            from,
            to,
            GraphEdge {
                relation,
                confidence,
                sources: unique_sources(sources.iter().cloned()),
                created: chrono::Utc::now().to_rfc3339(),
            },
        );
    }

    pub fn neighbors(&self, start: &str, depth: u32) -> Vec<String> {
        let name = start.to_lowercase();
        let start_idx = match self.name_to_idx.get(&name) {
            Some(&idx) => idx,
            None => return vec![],
        };

        let mut visited = HashSet::new();
        let mut queue: Vec<(NodeIndex, u32)> = vec![(start_idx, 0)];
        let mut result = vec![];

        while let Some((current, d)) = queue.pop() {
            if d > depth || visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            let node = &self.graph[current];
            if node.name != name {
                result.push(node.name.clone());
            }

            if d < depth {
                for neighbor in self.graph.neighbors_undirected(current) {
                    if !visited.contains(&neighbor) {
                        queue.push((neighbor, d + 1));
                    }
                }
            }
        }

        result
    }

    /// Return neighborhoods for graph nodes mentioned within a natural-language
    /// query instead of requiring the entire query to equal one node name.
    pub fn neighbors_for_query(&self, query: &str, depth: u32) -> Vec<String> {
        let query = query.to_lowercase();
        let mut result = HashSet::new();
        for node in self.graph.node_weights() {
            let matched = query.contains(&node.name.to_lowercase())
                || node
                    .aliases
                    .iter()
                    .any(|alias| query.contains(&alias.to_lowercase()));
            if matched {
                result.insert(node.name.clone());
                result.extend(self.neighbors(&node.name, depth));
            }
        }
        result.into_iter().collect()
    }

    /// Return retrieval-ready entity descriptions so meanings produced by the
    /// graph refinement pass improve chat context instead of being UI-only.
    pub fn entity_context_for_query(&self, query: &str, depth: u32) -> Vec<String> {
        self.neighbors_for_query(query, depth)
            .into_iter()
            .map(|name| {
                self.name_to_idx
                    .get(&name.to_lowercase())
                    .map(|idx| &self.graph[*idx])
                    .filter(|node| !node.meaning.is_empty())
                    .map(|node| format!("{} — {}", node.name, node.meaning))
                    .unwrap_or(name)
            })
            .collect()
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn all_nodes(&self) -> Vec<&GraphNode> {
        self.graph.node_indices().map(|i| &self.graph[i]).collect()
    }

    pub fn all_edges(&self) -> Vec<(String, String, String, f32, Vec<String>)> {
        self.graph
            .edge_indices()
            .filter_map(|e| {
                let (from, to) = self.graph.edge_endpoints(e)?;
                let edge = &self.graph[e];
                Some((
                    self.graph[from].name.clone(),
                    edge.relation.clone(),
                    self.graph[to].name.clone(),
                    edge.confidence,
                    edge.sources.clone(),
                ))
            })
            .collect()
    }

    pub fn remove_by_source(&mut self, source: &str) -> usize {
        let edge_indices: Vec<_> = self.graph.edge_indices().collect();
        for edge_idx in edge_indices {
            let edge = &mut self.graph[edge_idx];
            if let Some(position) = edge.sources.iter().position(|item| item == source) {
                edge.sources.remove(position);
                if edge.sources.is_empty() {
                    self.graph.remove_edge(edge_idx);
                }
            }
        }

        let names: Vec<String> = self
            .graph
            .node_weights()
            .filter(|node| node.sources.iter().any(|item| item == source))
            .map(|n| n.name.clone())
            .collect();
        let mut count = 0;
        for name in names {
            let Some(&idx) = self.name_to_idx.get(&name) else {
                continue;
            };
            self.graph[idx].sources.retain(|item| item != source);
            if self.graph[idx].sources.is_empty() {
                count += usize::from(self.remove_node(&name));
            } else {
                // Legacy workspace descriptions/aliases have no contributor map.
                // Keep shared identity/edges, but discard ambiguous removed-source text.
                self.graph[idx].meaning.clear();
                self.graph[idx].aliases.clear();
                self.graph[idx].updated = chrono::Utc::now().to_rfc3339();
                self.dirty_nodes.insert(idx);
            }
        }
        count
    }

    /// Merge aliases into one canonical node and rebuild all relations so
    /// incoming/outgoing edges, confidence, and document provenance are kept.
    pub fn merge_node_group(
        &mut self,
        canonical: &str,
        aliases: &[String],
        canonical_type: Option<NodeType>,
    ) -> usize {
        let canonical = canonical.trim().to_lowercase();
        if canonical.is_empty() {
            return 0;
        }
        let mut group_names: HashSet<String> = aliases
            .iter()
            .map(|name| name.trim().to_lowercase())
            .filter(|name| !name.is_empty())
            .collect();
        group_names.insert(canonical.clone());
        let group_indices: Vec<NodeIndex> = group_names
            .iter()
            .filter_map(|name| self.name_to_idx.get(name).copied())
            .collect();
        if group_indices.len() < 2 {
            return 0;
        }

        let edge_snapshots: Vec<_> = self
            .graph
            .edge_indices()
            .filter_map(|edge_idx| {
                let (from, to) = self.graph.edge_endpoints(edge_idx)?;
                Some((
                    self.graph[from].name.clone(),
                    self.graph[to].name.clone(),
                    self.graph[edge_idx].clone(),
                ))
            })
            .collect();
        let all_edges: Vec<_> = self.graph.edge_indices().collect();
        for edge_idx in all_edges {
            self.graph.remove_edge(edge_idx);
        }

        let target_idx = self
            .name_to_idx
            .get(&canonical)
            .copied()
            .unwrap_or(group_indices[0]);
        let old_target_name = self.graph[target_idx].name.clone();
        if old_target_name != canonical {
            self.name_to_idx.remove(&old_target_name);
            self.graph[target_idx].name = canonical.clone();
            self.name_to_idx.insert(canonical.clone(), target_idx);
        }

        let mut merged_sources = self.graph[target_idx].sources.clone();
        let mut merged_aliases = self.graph[target_idx].aliases.clone();
        if !old_target_name.eq_ignore_ascii_case(&canonical) {
            merged_aliases.push(old_target_name);
        }
        let mut merged_meaning = self.graph[target_idx].meaning.clone();
        let mut created = self.graph[target_idx].created.clone();
        for idx in &group_indices {
            merged_sources.extend(self.graph[*idx].sources.iter().cloned());
            merged_aliases.extend(self.graph[*idx].aliases.iter().cloned());
            if *idx != target_idx {
                merged_aliases.push(self.graph[*idx].name.clone());
            }
            if merged_meaning.is_empty() && !self.graph[*idx].meaning.is_empty() {
                merged_meaning = self.graph[*idx].meaning.clone();
            }
            created = created.min(self.graph[*idx].created.clone());
        }
        self.graph[target_idx].sources = unique_sources(merged_sources);
        self.graph[target_idx].aliases = unique_strings(merged_aliases)
            .into_iter()
            .filter(|alias| !alias.eq_ignore_ascii_case(&canonical))
            .collect();
        self.graph[target_idx].meaning = merged_meaning;
        self.graph[target_idx].created = created;
        self.graph[target_idx].updated = chrono::Utc::now().to_rfc3339();
        if let Some(node_type) = canonical_type {
            self.graph[target_idx].node_type = node_type;
        }

        let aliases_to_remove: Vec<String> = group_indices
            .iter()
            .filter(|idx| **idx != target_idx)
            .map(|idx| self.graph[*idx].name.clone())
            .collect();
        for alias in &aliases_to_remove {
            self.remove_node(alias);
        }

        for (from, to, edge) in edge_snapshots {
            let from = if group_names.contains(&from) {
                canonical.as_str()
            } else {
                from.as_str()
            };
            let to = if group_names.contains(&to) {
                canonical.as_str()
            } else {
                to.as_str()
            };
            let (Some(&from_idx), Some(&to_idx)) =
                (self.name_to_idx.get(from), self.name_to_idx.get(to))
            else {
                continue;
            };
            self.upsert_edge_sources(
                from_idx,
                to_idx,
                &edge.relation,
                edge.confidence,
                &edge.sources,
            );
        }
        self.dirty_nodes.insert(target_idx);
        aliases_to_remove.len()
    }

    /// Safely improve a node label/type/meaning without changing its identity or
    /// relations. Renaming to an already-existing node is rejected; duplicate
    /// identities must go through `merge_node_group` instead.
    pub fn refine_node(
        &mut self,
        current_name: &str,
        canonical_name: &str,
        canonical_type: Option<NodeType>,
        meaning: &str,
    ) -> bool {
        let current_key = current_name.trim().to_lowercase();
        let canonical_key = canonical_name.trim().to_lowercase();
        if current_key.is_empty() || canonical_key.is_empty() || canonical_key.len() > 180 {
            return false;
        }
        let Some(&idx) = self.name_to_idx.get(&current_key) else {
            return false;
        };
        if self.graph[idx].node_type == NodeType::Document {
            return false;
        }
        if let Some(&collision) = self.name_to_idx.get(&canonical_key) {
            if collision != idx {
                return false;
            }
        }

        let mut changed = false;
        if canonical_key != current_key {
            let old_name = self.graph[idx].name.clone();
            self.name_to_idx.remove(&current_key);
            self.name_to_idx.insert(canonical_key.clone(), idx);
            self.graph[idx].name = canonical_key.clone();
            if !old_name.eq_ignore_ascii_case(&canonical_key)
                && !self.graph[idx]
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(&old_name))
            {
                self.graph[idx].aliases.push(old_name);
            }
            changed = true;
        }
        if let Some(node_type) = canonical_type {
            if self.graph[idx].node_type != node_type {
                self.graph[idx].node_type = node_type;
                changed = true;
            }
        }
        let meaning = meaning.trim();
        if !meaning.is_empty() && meaning.len() <= 500 && self.graph[idx].meaning != meaning {
            self.graph[idx].meaning = meaning.to_string();
            changed = true;
        }
        if changed {
            self.graph[idx].updated = chrono::Utc::now().to_rfc3339();
            self.dirty_nodes.insert(idx);
        }
        changed
    }

    /// Replace an imprecise relation label with a concise canonical predicate.
    /// If the canonical relation already exists, evidence and confidence are
    /// merged so the operation cannot create duplicate edges.
    pub fn refine_relation(
        &mut self,
        from_name: &str,
        to_name: &str,
        current_relation: &str,
        improved_relation: &str,
    ) -> bool {
        let (Some(&from), Some(&to)) = (
            self.name_to_idx.get(&from_name.trim().to_lowercase()),
            self.name_to_idx.get(&to_name.trim().to_lowercase()),
        ) else {
            return false;
        };
        let Some(improved) = normalize_relation_label(improved_relation) else {
            return false;
        };
        let Some(current_relation) = normalize_relation_label(current_relation) else {
            return false;
        };
        let current = self
            .graph
            .edges_connecting(from, to)
            .find(|edge| {
                normalize_relation_label(&edge.weight().relation).as_ref()
                    == Some(&current_relation)
            })
            .map(|edge| edge.id());
        let Some(current_idx) = current else {
            return false;
        };
        if self.graph[current_idx].relation == improved {
            return false;
        }
        let duplicate = self
            .graph
            .edges_connecting(from, to)
            .find(|edge| {
                edge.id() != current_idx
                    && normalize_relation_label(&edge.weight().relation).as_ref() == Some(&improved)
            })
            .map(|edge| edge.id());
        if let Some(duplicate_idx) = duplicate {
            let confidence = self.graph[current_idx].confidence;
            let sources = self.graph[current_idx].sources.clone();
            self.graph[duplicate_idx].confidence =
                self.graph[duplicate_idx].confidence.max(confidence);
            for source in sources {
                if !self.graph[duplicate_idx].sources.contains(&source) {
                    self.graph[duplicate_idx].sources.push(source);
                }
            }
            self.graph.remove_edge(current_idx);
        } else {
            self.graph[current_idx].relation = improved;
        }
        true
    }

    pub fn remove_node(&mut self, name: &str) -> bool {
        let name = name.to_lowercase();
        if let Some(&idx) = self.name_to_idx.get(&name) {
            self.graph.remove_node(idx);
            self.name_to_idx.remove(&name);
            self.dirty_nodes.remove(&idx);
            true
        } else {
            false
        }
    }
}

fn unique_sources(sources: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    sources
        .into_iter()
        .filter(|source| !source.is_empty() && seen.insert(source.clone()))
        .collect()
}

fn unique_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.is_empty() && seen.insert(value.to_lowercase()))
        .collect()
}

pub(crate) fn normalize_relation_label(value: &str) -> Option<String> {
    let tokens = value
        .trim()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .take(8)
        .map(|token| token.to_uppercase())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        None
    } else {
        let relation = tokens.join("_");
        (relation.len() <= 96).then_some(relation)
    }
}

fn row_sources(row: &serde_json::Value) -> Vec<String> {
    let sources = row_string_array(row, "sources");
    if sources.is_empty() {
        row.get("source")
            .and_then(|value| value.as_str())
            .filter(|source| !source.is_empty())
            .map(|source| vec![source.to_string()])
            .unwrap_or_default()
    } else {
        unique_sources(sources)
    }
}

fn row_string_array(row: &serde_json::Value, field: &str) -> Vec<String> {
    row.get(field)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_relation_separators_merge_evidence() {
        let mut graph = KnowledgeGraph::new();
        let sensor = graph.upsert_node("sensor", NodeType::Concept);
        let pressure = graph.upsert_node("pressure", NodeType::Concept);
        graph.upsert_edge_sourced(sensor, pressure, "has pressure", 0.7, "a.md");
        graph.upsert_edge_sourced(sensor, pressure, "HAS_PRESSURE", 0.9, "b.md");
        assert_eq!(graph.edge_count(), 1);
        let edges = graph.all_edges();
        assert_eq!(edges[0].1, "HAS_PRESSURE");
        assert_eq!(edges[0].3, 0.9);
        assert_eq!(edges[0].4, ["a.md", "b.md"]);
        assert!(graph.refine_relation("sensor", "pressure", "has pressure", "MEASURES"));
        assert_eq!(graph.all_edges()[0].1, "MEASURES");
    }

    #[test]
    fn deleting_a_node_keeps_other_indices_valid() {
        let mut graph = KnowledgeGraph::new();
        let a = graph.upsert_node("a", NodeType::Concept);
        let b = graph.upsert_node("b", NodeType::Concept);
        let c = graph.upsert_node("c", NodeType::Concept);
        graph.upsert_edge(a, b, "LINKS", 1.0);
        graph.upsert_edge(b, c, "LINKS", 1.0);

        assert!(graph.remove_node("a"));
        assert_eq!(graph.upsert_node("c", NodeType::Concept), c);
        assert!(graph.remove_node("b"));
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.all_nodes()[0].name, "c");
    }

    #[test]
    fn merging_nodes_rewires_and_deduplicates_relations() {
        let mut graph = KnowledgeGraph::new();
        let raft = graph.upsert_node_sourced("raft", NodeType::Concept, "a.md");
        let alias = graph.upsert_node_sourced("raft consensus", NodeType::Concept, "b.md");
        let quorum = graph.upsert_node_sourced("quorum", NodeType::Concept, "a.md");
        graph.upsert_edge_sourced(raft, quorum, "USES", 0.8, "a.md");
        graph.upsert_edge_sourced(alias, quorum, "USES", 0.95, "b.md");

        assert_eq!(
            graph.merge_node_group(
                "raft",
                &["raft consensus".to_string()],
                Some(NodeType::Concept)
            ),
            1
        );
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        let edge = &graph.all_edges()[0];
        assert_eq!(edge.0, "raft");
        assert_eq!(edge.3, 0.95);
        assert_eq!(edge.4.len(), 2);
    }

    #[test]
    fn deleting_one_source_preserves_shared_nodes_and_edges() {
        let mut graph = KnowledgeGraph::new();
        let shared = graph.upsert_node_sourced("raft", NodeType::Concept, "a.md");
        graph.upsert_node_sourced("raft", NodeType::Concept, "b.md");
        let quorum = graph.upsert_node_sourced("quorum", NodeType::Concept, "a.md");
        graph.upsert_node_sourced("quorum", NodeType::Concept, "b.md");
        graph.upsert_edge_sourced(shared, quorum, "USES", 0.9, "a.md");
        graph.upsert_edge_sourced(shared, quorum, "USES", 0.9, "b.md");

        assert_eq!(graph.remove_by_source("a.md"), 0);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.all_edges()[0].4, vec!["b.md".to_string()]);
    }

    #[test]
    fn refining_a_node_preserves_edges_and_records_the_old_name() {
        let mut graph = KnowledgeGraph::new();
        let old = graph.upsert_node_sourced("rl", NodeType::Other, "a.md");
        let policy = graph.upsert_node_sourced("policy", NodeType::Concept, "a.md");
        graph.upsert_edge_sourced(old, policy, "HAS", 0.9, "a.md");

        assert!(graph.refine_node(
            "rl",
            "reinforcement learning",
            Some(NodeType::Concept),
            "A learning paradigm in which an agent improves behavior from rewards."
        ));
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        let refined = graph
            .all_nodes()
            .into_iter()
            .find(|node| node.name == "reinforcement learning")
            .expect("renamed node");
        assert_eq!(refined.aliases, vec!["rl".to_string()]);
        assert!(!refined.meaning.is_empty());
        assert_eq!(graph.all_edges()[0].0, "reinforcement learning");
    }

    #[test]
    fn refining_a_relation_deduplicates_existing_canonical_edge() {
        let mut graph = KnowledgeGraph::new();
        let model = graph.upsert_node_sourced("model", NodeType::Concept, "a.md");
        let data = graph.upsert_node_sourced("data", NodeType::Concept, "a.md");
        graph.upsert_edge_sourced(model, data, "is trained using", 0.8, "a.md");
        graph.upsert_edge_sourced(model, data, "TRAINED_ON", 0.95, "b.md");

        assert!(graph.refine_relation("model", "data", "is trained using", "trained on"));
        assert_eq!(graph.edge_count(), 1);
        let edge = &graph.all_edges()[0];
        assert_eq!(edge.1, "TRAINED_ON");
        assert_eq!(edge.3, 0.95);
        assert_eq!(edge.4.len(), 2);
    }

    #[test]
    fn refined_meanings_and_aliases_are_available_to_retrieval() {
        let mut graph = KnowledgeGraph::new();
        graph.upsert_node("reinforcement learning", NodeType::Concept);
        assert!(graph.refine_node(
            "reinforcement learning",
            "reinforcement learning",
            None,
            "A learning paradigm driven by reward feedback."
        ));
        let idx = graph.name_to_idx["reinforcement learning"];
        graph.graph[idx].aliases.push("rl".into());

        let context = graph.entity_context_for_query("explain rl", 1);
        assert_eq!(context.len(), 1);
        assert!(context[0].contains("reward feedback"));
    }
}
