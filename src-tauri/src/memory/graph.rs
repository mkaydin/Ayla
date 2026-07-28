// ── Knowledge graph ────────────────────────────────────────
// Layer 5: In-memory Petgraph + SurrealDB persistence.

use crate::db::surreal::SurrealDB;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeType {
    Person,
    Project,
    Tool,
    Concept,
    Document,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub name: String,
    pub node_type: NodeType,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub relation: String,
    pub confidence: f32,
    pub created: String,
}

pub struct KnowledgeGraph {
    graph: DiGraph<GraphNode, GraphEdge>,
    name_to_idx: HashMap<String, NodeIndex>,
    dirty_nodes: HashSet<NodeIndex>,
    db: Option<Arc<SurrealDB>>,
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            name_to_idx: HashMap::new(),
            dirty_nodes: HashSet::new(),
            db: None,
        }
    }

    pub fn with_db(db: Arc<SurrealDB>) -> Self {
        Self { db: Some(db), ..Self::new() }
    }

    pub async fn load(&mut self) -> Result<(), String> {
        let db = match self.db.clone() {
            Some(d) => d,
            None => return Ok(()),
        };
        db.ensure_table("graph_node").await?;
        db.ensure_table("graph_edge").await?;

        // Load nodes
        if let Ok(rows) = db.query("SELECT * FROM graph_node;").await {
            for row in rows {
                let name = row.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let type_str = row.get("node_type").and_then(|v| v.as_str()).unwrap_or("Other");
                let nt = match type_str {
                    "Person" => NodeType::Person, "Project" => NodeType::Project,
                    "Tool" => NodeType::Tool, "Concept" => NodeType::Concept,
                    "Document" => NodeType::Document, _ => NodeType::Other,
                };
                if !name.is_empty() {
                    self.upsert_node(name, nt);
                }
            }
        }

        // Load edges
        if let Ok(rows) = db.query("SELECT * FROM graph_edge;").await {
            for row in rows {
                let from = row.get("from_node").and_then(|v| v.as_str()).unwrap_or("");
                let to = row.get("to_node").and_then(|v| v.as_str()).unwrap_or("");
                let rel = row.get("relation").and_then(|v| v.as_str()).unwrap_or("");
                let conf = row.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.8) as f32;
                if !from.is_empty() && !to.is_empty() {
                    let fi = self.upsert_node(from, NodeType::Concept);
                    let ti = self.upsert_node(to, NodeType::Concept);
                    self.upsert_edge(fi, ti, rel, conf);
                }
            }
        }

        log::info!("Graph loaded: {} nodes, {} edges", self.node_count(), self.edge_count());
        Ok(())
    }

    pub async fn sync(&mut self) -> Result<(), String> {
        let db = match &self.db {
            Some(d) => d,
            None => return Ok(()),
        };

        // Delete old data and re-insert (simple full sync)
        let _ = db.query("DELETE graph_node;").await;
        let _ = db.query("DELETE graph_edge;").await;

        // Save all nodes
        for idx in self.graph.node_indices() {
            let node = &self.graph[idx];
            let record = serde_json::json!({
                "name": node.name,
                "node_type": format!("{:?}", node.node_type),
                "updated": node.updated,
            });
            let _ = db.insert("graph_node", record).await;
        }

        // Save all edges
        for edge_idx in self.graph.edge_indices() {
            if let Some((from, to)) = self.graph.edge_endpoints(edge_idx) {
                let edge = &self.graph[edge_idx];
                let record = serde_json::json!({
                    "from_node": self.graph[from].name,
                    "to_node": self.graph[to].name,
                    "relation": edge.relation,
                    "confidence": edge.confidence,
                });
                let _ = db.insert("graph_edge", record).await;
            }
        }

        self.dirty_nodes.clear();
        log::info!("Graph synced: {} nodes, {} edges", self.node_count(), self.edge_count());
        Ok(())
    }

    pub fn upsert_node(&mut self, name: &str, node_type: NodeType) -> NodeIndex {
        let name = name.to_lowercase();
        if let Some(&idx) = self.name_to_idx.get(&name) {
            let node = &mut self.graph[idx];
            node.updated = chrono::Utc::now().to_rfc3339();
            self.dirty_nodes.insert(idx);
            return idx;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let node = GraphNode {
            name: name.clone(),
            node_type,
            created: now.clone(),
            updated: now,
        };
        let idx = self.graph.add_node(node);
        self.name_to_idx.insert(name, idx);
        self.dirty_nodes.insert(idx);
        idx
    }

    pub fn upsert_edge(&mut self, from: NodeIndex, to: NodeIndex, relation: &str, confidence: f32) {
        // Check if edge already exists
        for edge in self.graph.edges_connecting(from, to) {
            if edge.weight().relation == relation {
                return;
            }
        }
        self.graph.add_edge(from, to, GraphEdge {
            relation: relation.to_string(),
            confidence,
            created: chrono::Utc::now().to_rfc3339(),
        });
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
            if d > depth || visited.contains(&current) { continue; }
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

    pub fn node_count(&self) -> usize { self.graph.node_count() }
    pub fn edge_count(&self) -> usize { self.graph.edge_count() }

    pub fn all_nodes(&self) -> Vec<&GraphNode> {
        self.graph.node_indices().map(|i| &self.graph[i]).collect()
    }

    pub fn all_edges(&self) -> Vec<(String, String, String, f32)> {
        self.graph.edge_indices().filter_map(|e| {
            let (from, to) = self.graph.edge_endpoints(e)?;
            let edge = &self.graph[e];
            Some((
                self.graph[from].name.clone(),
                edge.relation.clone(),
                self.graph[to].name.clone(),
                edge.confidence,
            ))
        }).collect()
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

    pub fn remove_edge(&mut self, from_name: &str, to_name: &str, relation: &str) -> bool {
        let from = self.name_to_idx.get(&from_name.to_lowercase());
        let to = self.name_to_idx.get(&to_name.to_lowercase());
        if let (Some(&fi), Some(&ti)) = (from, to) {
            let edges: Vec<_> = self.graph.edges_connecting(fi, ti)
                .map(|e| (e.id(), e.weight().relation.clone()))
                .collect();
            for (eid, rel) in edges {
                if rel == relation {
                    self.graph.remove_edge(eid);
                    return true;
                }
            }
        }
        false
    }
}
