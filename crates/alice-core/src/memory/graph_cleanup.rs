use crate::memory::config::MemoryConfig;
use crate::{db::surreal::SurrealDB, EventSink};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

const CLEANUP_MAX_OUTPUT_TOKENS: usize = 8_192;
const BATCH_NODE_COUNT: usize = 80;
const BATCH_EDGE_COUNT: usize = 100;
const CHECKPOINT_VERSION: &str = "graph-refinement-v2";

#[derive(Debug, Clone, Serialize)]
pub struct CleanupNode {
    pub name: String,
    pub node_type: String,
    pub sources: Vec<String>,
    pub aliases: Vec<String>,
    pub meaning: String,
    pub context: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupEdge {
    pub from: String,
    pub relation: String,
    pub to: String,
    pub confidence: f32,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MergeProposal {
    pub canonical: String,
    pub aliases: Vec<String>,
    pub canonical_type: String,
    pub confidence: f32,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeRefinementProposal {
    pub current_name: String,
    pub canonical_name: String,
    pub canonical_type: String,
    pub meaning: String,
    pub confidence: f32,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RelationRefinementProposal {
    pub from: String,
    pub to: String,
    pub current_relation: String,
    pub improved_relation: String,
    pub confidence: f32,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct GraphRefinementAnalysis {
    pub merge_groups: Vec<MergeProposal>,
    pub node_refinements: Vec<NodeRefinementProposal>,
    pub relation_refinements: Vec<RelationRefinementProposal>,
    pub checkpoint_signature: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CleanupResponse {
    #[serde(default)]
    merge_groups: Vec<MergeProposal>,
    #[serde(default)]
    node_refinements: Vec<NodeRefinementProposal>,
    #[serde(default)]
    relation_refinements: Vec<RelationRefinementProposal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum AnalysisTask {
    Duplicates,
    Nodes,
    Relations,
}

#[derive(Debug, Clone, Serialize)]
struct AnalysisBatch {
    key: String,
    task: AnalysisTask,
    nodes: Vec<CleanupNode>,
    edges: Vec<CleanupEdge>,
    candidate_groups: Vec<Vec<String>>,
}

#[derive(Clone)]
pub struct GraphCleanupAnalyzer {
    config: MemoryConfig,
    client: reqwest::Client,
}

impl GraphCleanupAnalyzer {
    pub fn new(config: MemoryConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    pub async fn analyze(
        &self,
        nodes: &[CleanupNode],
        edges: &[CleanupEdge],
        events: &dyn EventSink,
    ) -> Result<GraphRefinementAnalysis, String> {
        let signature = graph_signature(nodes, edges);
        let (batches, local_relations) = build_analysis_plan(nodes, edges);
        let checkpoint_db = self.checkpoint_db().await;
        let mut checkpoints = if let Some(db) = checkpoint_db.as_deref() {
            load_checkpoints(db, &signature).await
        } else {
            HashMap::new()
        };
        let resumed_batches = batches
            .iter()
            .filter(|batch| checkpoints.contains_key(&batch.key))
            .count();
        let concurrency = if self.config.llm_model.to_lowercase().contains(":cloud") {
            2
        } else {
            1
        };
        let _ = events.emit(
            "kb-cleanup-progress",
            json!({
                "step": "planned",
                "pct": 10,
                "total_batches": batches.len(),
                "resumed_batches": resumed_batches,
                "local_relations": local_relations.len(),
                "concurrency": concurrency
            }),
        );

        let mut responses = Vec::with_capacity(batches.len());
        let mut pending = Vec::new();
        for batch in batches {
            if let Some(response) = checkpoints.remove(&batch.key) {
                responses.push(response);
            } else {
                pending.push(batch);
            }
        }

        let started = std::time::Instant::now();
        let mut network_completed = 0usize;
        let total_batches = responses.len() + pending.len();
        for group in pending.chunks(concurrency) {
            let group_started = std::time::Instant::now();
            let results = if group.len() == 2 {
                let (first, second) =
                    tokio::join!(self.analyze_batch(&group[0]), self.analyze_batch(&group[1]));
                vec![first, second]
            } else {
                vec![self.analyze_batch(&group[0]).await]
            };
            let mut group_error = None;
            for (batch, result) in group.iter().zip(results) {
                match result {
                    Ok(response) => {
                        if let Some(db) = checkpoint_db.as_deref() {
                            save_checkpoint(db, &signature, &batch.key, &response).await;
                        }
                        responses.push(response);
                        network_completed += 1;
                    }
                    Err(error) => {
                        if group_error.is_none() {
                            group_error = Some(error);
                        }
                    }
                }
            }
            if let Some(error) = group_error {
                return Err(error);
            }
            let completed = resumed_batches + network_completed;
            let remaining = total_batches.saturating_sub(completed);
            let elapsed_seconds = started.elapsed().as_secs_f64();
            let throughput = elapsed_seconds / network_completed.max(1) as f64;
            let eta_seconds = (throughput * remaining as f64).round() as u64;
            let pct = 10 + ((completed as f64 / total_batches.max(1) as f64) * 62.0) as u64;
            let _ = events.emit(
                "kb-cleanup-progress",
                json!({
                    "step": "batch",
                    "pct": pct.min(72),
                    "current_batch": completed,
                    "total_batches": total_batches,
                    "remaining_batches": remaining,
                    "eta_seconds": eta_seconds,
                    "resumed_batches": resumed_batches,
                    "last_batch_ms": group_started.elapsed().as_millis(),
                    "concurrency": concurrency
                }),
            );
        }
        let mut analysis = deduplicate_analysis(responses);
        analysis.relation_refinements.extend(local_relations);
        analysis.checkpoint_signature = signature;
        Ok(analysis)
    }

    async fn analyze_batch(&self, batch: &AnalysisBatch) -> Result<CleanupResponse, String> {
        let inventory = serde_json::to_string(&json!({
            "nodes": batch.nodes,
            "edges": batch.edges,
            "duplicate_candidate_groups": batch.candidate_groups,
        }))
        .unwrap_or_default();
        let schema = cleanup_schema();
        let task_constraint = match batch.task {
            AnalysisTask::Duplicates => "DUPLICATES ONLY. Compare nodes only within each duplicate_candidate_group. Return empty node_refinements and relation_refinements arrays.",
            AnalysisTask::Nodes => "NODES ONLY. Improve the supplied nodes. Return empty merge_groups and relation_refinements arrays.",
            AnalysisTask::Relations => "RELATIONS ONLY. Improve the supplied edges. Return empty merge_groups and node_refinements arrays.",
        };
        let prompt = format!(
            "Analyze and conservatively improve this knowledge graph inventory. Perform three\
             independent tasks: exact-duplicate merging, node refinement, and relation-label\
             refinement. Every proposal must preserve the factual identity and direction of the\
             existing graph.\n\n\
             MERGE RULES:\n\
             - Merge spelling, capitalization, acronym/full-name, or redundant qualified-name\
               variants only when their relation context confirms the same identity.\n\
             - Never merge merely related, parent/child, component/system, cause/effect, or\
               broad/narrow concepts.\n\
             - Never merge Document nodes.\n\
             - Prefer the shortest precise existing name as canonical. A clearer new canonical\
               name is allowed only when all aliases unambiguously share that identity.\n\
             - `aliases` must contain every existing node name to replace; include the canonical\
               name too when it already exists.\n\
             - Emit only groups with confidence >= 0.90. When uncertain, do not merge.\n\
             - Return an empty merge_groups array when no safe duplicates exist.\n\n\
             NODE REFINEMENT RULES:\n\
             - Inspect every non-Document node. Rename only labels that are unclear, malformed,\
               overly verbose, sentence-like, or an unexplained abbreviation.\n\
             - canonical_name must be a concise, specific noun phrase for the exact same entity;\
               never broaden, narrow, or reinterpret it. Keep the current name when it is already\
               good.\n\
             - meaning must be one concise, standalone sentence grounded only in the supplied\
               node and relation context. Do not invent facts.\n\
             - Emit a node_refinement when the name should change OR the existing meaning is empty\
               or materially unclear. Never refine Document nodes.\n\
             - Emit only node refinements with confidence >= 0.90.\n\n\
             RELATION REFINEMENT RULES:\n\
             - Replace vague, sentence-like, malformed, or inconsistent relation text with a\
               precise predicate of one to five words.\n\
             - Use UPPER_SNAKE_CASE, for example USES, PART_OF, CAUSES, TRAINED_ON, or IMPROVES.\n\
             - Preserve edge direction and factual meaning. Do not add or delete relations.\n\
             - Do not replace a specific relation with RELATED_TO.\n\
             - Emit only relation refinements with confidence >= 0.92.\n\n\
             BATCH CONSTRAINT:\n{}\n\n\
             Output must match this JSON schema exactly:\n{}\n\n\
             GRAPH INVENTORY:\n{}",
            task_constraint,
            serde_json::to_string(&schema).unwrap_or_default(),
            inventory
        );
        let output_tokens = match batch.task {
            AnalysisTask::Nodes => CLEANUP_MAX_OUTPUT_TOKENS,
            AnalysisTask::Duplicates | AnalysisTask::Relations => CLEANUP_MAX_OUTPUT_TOKENS / 2,
        };
        let input_bytes = prompt
            .len()
            .saturating_add(serde_json::to_vec(&schema).map_or(0, |encoded| encoded.len()));
        let num_ctx = self.config.ollama_context_for(input_bytes, output_tokens);
        log::info!(
            "Graph cleanup analysis: {} nodes, {} edges, num_ctx={num_ctx}",
            batch.nodes.len(),
            batch.edges.len()
        );
        let body = json!({
            "model": self.config.llm_model,
            "messages": [
                {
                    "role": "system",
                    "content": "You are a conservative knowledge-graph refinement engine. Return only schema-valid JSON grounded in the supplied inventory."
                },
                { "role": "user", "content": prompt }
            ],
            "stream": false,
            "think": false,
            "format": schema,
            "options": {
                "temperature": 0.0,
                "seed": 42,
                "num_ctx": num_ctx,
                "num_predict": output_tokens
            }
        });
        let response = self
            .client
            .post(format!("{}/api/chat", self.config.ollama_url))
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("Graph cleanup LLM request: {error}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("Graph cleanup LLM response: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "Graph cleanup LLM HTTP {}: {}",
                status,
                crate::memory::safe_truncate(&text, 500)
            ));
        }
        let envelope: Value =
            serde_json::from_str(&text).map_err(|error| format!("Ollama JSON: {error}"))?;
        let content = envelope["message"]["content"]
            .as_str()
            .ok_or("Ollama graph cleanup response did not contain message.content")?;
        let parsed = parse_cleanup_response(content)?;
        Ok(parsed)
    }

    async fn checkpoint_db(&self) -> Option<Arc<SurrealDB>> {
        match SurrealDB::connect(
            &self.config.surrealdb_url,
            &self.config.surrealdb_ns,
            &self.config.surrealdb_db,
            &self.config.surrealdb_user,
            &self.config.surrealdb_pass,
        )
        .await
        {
            Ok(db) => Some(Arc::new(db)),
            Err(error) => {
                log::warn!("Graph cleanup checkpoints unavailable: {error}");
                None
            }
        }
    }

    pub async fn clear_checkpoint(&self, signature: &str) {
        let Some(db) = self.checkpoint_db().await else {
            return;
        };
        let mut vars = serde_json::Map::new();
        vars.insert("signature".into(), json!(signature));
        if let Err(error) = db
            .exec_with_vars(
                "DELETE graph_cleanup_checkpoint WHERE signature = $signature RETURN NONE;",
                vars,
            )
            .await
        {
            log::warn!("Graph cleanup checkpoint removal failed: {error}");
        }
    }
}

fn build_analysis_plan(
    nodes: &[CleanupNode],
    edges: &[CleanupEdge],
) -> (Vec<AnalysisBatch>, Vec<RelationRefinementProposal>) {
    let by_name: HashMap<String, CleanupNode> = nodes
        .iter()
        .cloned()
        .map(|node| (node.name.to_lowercase(), node))
        .collect();
    let mut batches = Vec::new();

    // Duplicate detection is candidate-based: only locally plausible aliases
    // reach the LLM, grouped so unrelated nodes cannot be merged together.
    let candidate_groups = duplicate_candidate_groups(nodes);
    let mut group_batch = Vec::<Vec<String>>::new();
    let mut group_node_count = 0usize;
    for group in candidate_groups {
        let additional = group
            .iter()
            .filter(|name| {
                !group_batch
                    .iter()
                    .flatten()
                    .any(|existing| existing.eq_ignore_ascii_case(name))
            })
            .count();
        if !group_batch.is_empty() && group_node_count + additional > BATCH_NODE_COUNT {
            batches.push(duplicate_batch(&group_batch, &by_name));
            group_batch.clear();
            group_node_count = 0;
        }
        group_node_count += additional;
        group_batch.push(group);
    }
    if !group_batch.is_empty() {
        batches.push(duplicate_batch(&group_batch, &by_name));
    }

    // Meanings and labels are incremental. Nodes with a useful saved meaning
    // and a clean label do not consume another model call on later runs.
    let mut incremental_nodes = nodes
        .iter()
        .filter(|node| node_needs_refinement(node))
        .cloned()
        .collect::<Vec<_>>();
    incremental_nodes.sort_by(|a, b| a.name.cmp(&b.name));
    for chunk in incremental_nodes.chunks(BATCH_NODE_COUNT) {
        batches.push(finalize_batch(AnalysisBatch {
            key: String::new(),
            task: AnalysisTask::Nodes,
            nodes: chunk.to_vec(),
            edges: Vec::new(),
            candidate_groups: Vec::new(),
        }));
    }

    // Safe formatting changes happen locally. Only vague or genuinely
    // sentence-like predicates need semantic interpretation by the LLM, and
    // every such edge belongs to exactly one relation batch.
    let mut unresolved_edges = Vec::new();
    let mut local_relations = Vec::new();
    for edge in edges {
        let normalized = deterministic_relation_label(&edge.relation);
        if relation_needs_llm(&edge.relation, &normalized) {
            unresolved_edges.push(edge.clone());
        } else if !edge.relation.eq(&normalized) {
            local_relations.push(RelationRefinementProposal {
                from: edge.from.clone(),
                to: edge.to.clone(),
                current_relation: edge.relation.clone(),
                improved_relation: normalized,
                confidence: 1.0,
                reason: "Deterministic canonical relation formatting".into(),
            });
        }
    }
    unresolved_edges
        .sort_by(|a, b| (&a.from, &a.to, &a.relation).cmp(&(&b.from, &b.to, &b.relation)));
    for chunk in unresolved_edges.chunks(BATCH_EDGE_COUNT) {
        let endpoint_names = chunk
            .iter()
            .flat_map(|edge| [edge.from.to_lowercase(), edge.to.to_lowercase()])
            .collect::<HashSet<_>>();
        let mut endpoint_nodes = endpoint_names
            .into_iter()
            .filter_map(|name| by_name.get(&name).cloned())
            .map(|mut node| {
                node.sources.clear();
                node.aliases.truncate(4);
                node.context.clear();
                node
            })
            .collect::<Vec<_>>();
        endpoint_nodes.sort_by(|a, b| a.name.cmp(&b.name));
        batches.push(finalize_batch(AnalysisBatch {
            key: String::new(),
            task: AnalysisTask::Relations,
            nodes: endpoint_nodes,
            edges: chunk.to_vec(),
            candidate_groups: Vec::new(),
        }));
    }
    (batches, local_relations)
}

fn duplicate_batch(
    groups: &[Vec<String>],
    by_name: &HashMap<String, CleanupNode>,
) -> AnalysisBatch {
    let names = groups
        .iter()
        .flatten()
        .map(|name| name.to_lowercase())
        .collect::<HashSet<_>>();
    let mut nodes = names
        .into_iter()
        .filter_map(|name| by_name.get(&name).cloned())
        .map(|mut node| {
            node.context.truncate(6);
            node.sources.truncate(4);
            node
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    finalize_batch(AnalysisBatch {
        key: String::new(),
        task: AnalysisTask::Duplicates,
        nodes,
        edges: Vec::new(),
        candidate_groups: groups.to_vec(),
    })
}

fn finalize_batch(mut batch: AnalysisBatch) -> AnalysisBatch {
    batch.key = format!("{:016x}", stable_hash_json(&batch));
    batch
}

fn node_needs_refinement(node: &CleanupNode) -> bool {
    if node.node_type == "Document" {
        return false;
    }
    let name = node.name.trim();
    node.meaning.trim().is_empty()
        || name.len() > 80
        || name.split_whitespace().count() > 10
        || name.chars().any(char::is_control)
        || name.ends_with(['.', ',', ';', ':'])
}

fn duplicate_candidate_groups(nodes: &[CleanupNode]) -> Vec<Vec<String>> {
    let eligible = nodes
        .iter()
        .filter(|node| node.node_type != "Document")
        .collect::<Vec<_>>();
    let mut keyed: HashMap<String, HashSet<String>> = HashMap::new();
    let mut compact_names: HashMap<String, Vec<String>> = HashMap::new();
    for node in &eligible {
        let sort_key = candidate_sort_key(&node.name);
        let compact = compact_name(&node.name);
        if !sort_key.is_empty() {
            keyed
                .entry(format!("ordered:{sort_key}"))
                .or_default()
                .insert(node.name.clone());
        }
        if !compact.is_empty() {
            keyed
                .entry(format!("compact:{compact}"))
                .or_default()
                .insert(node.name.clone());
            compact_names
                .entry(compact.clone())
                .or_default()
                .push(node.name.clone());
            let characters = compact.chars().collect::<Vec<_>>();
            if (4..=64).contains(&characters.len()) {
                for removed in 0..characters.len() {
                    let deletion_key = characters
                        .iter()
                        .enumerate()
                        .filter_map(|(index, character)| (index != removed).then_some(*character))
                        .collect::<String>();
                    keyed
                        .entry(format!("fuzzy:{deletion_key}"))
                        .or_default()
                        .insert(node.name.clone());
                }
            }
        }
        let mut tokens = name_tokens(&node.name);
        if tokens.len() > 1 {
            tokens.sort();
            keyed
                .entry(format!("tokens:{}", tokens.join(" ")))
                .or_default()
                .insert(node.name.clone());
        }
    }
    for node in &eligible {
        let tokens = name_tokens(&node.name);
        if tokens.len() < 2 {
            continue;
        }
        let acronym = tokens
            .iter()
            .filter_map(|token| token.chars().next())
            .collect::<String>();
        if acronym.len() < 2 || acronym.len() > 10 {
            continue;
        }
        if let Some(short_names) = compact_names.get(&acronym) {
            let group = keyed.entry(format!("acronym:{}", node.name)).or_default();
            group.insert(node.name.clone());
            group.extend(short_names.iter().cloned());
        }
    }
    // A concise existing name is also a candidate for a qualified form such
    // as "raft" / "raft consensus algorithm". This only proposes comparison;
    // the LLM still has to confirm identical identity at high confidence.
    let single_token_nodes = eligible
        .iter()
        .filter_map(|node| {
            let tokens = name_tokens(&node.name);
            (tokens.len() == 1 && tokens[0].len() >= 3)
                .then_some((tokens[0].clone(), node.name.clone()))
        })
        .collect::<Vec<_>>();
    for node in &eligible {
        let tokens = name_tokens(&node.name);
        if tokens.len() < 2 {
            continue;
        }
        for (token, short_name) in &single_token_nodes {
            if tokens.contains(token) {
                let group = keyed
                    .entry(format!("qualified:{short_name}:{}", node.name))
                    .or_default();
                group.insert(short_name.clone());
                group.insert(node.name.clone());
            }
        }
    }

    let groups = keyed
        .into_values()
        .filter(|group| group.len() >= 2)
        .collect::<Vec<_>>();
    merge_overlapping_groups(groups)
}

fn merge_overlapping_groups(groups: Vec<HashSet<String>>) -> Vec<Vec<String>> {
    let mut merged = Vec::<HashSet<String>>::new();
    for mut group in groups {
        let mut index = 0;
        while index < merged.len() {
            if !group.is_disjoint(&merged[index]) {
                group.extend(merged.swap_remove(index));
            } else {
                index += 1;
            }
        }
        merged.push(group);
    }
    let mut result = merged
        .into_iter()
        .map(|group| {
            let mut names = group.into_iter().collect::<Vec<_>>();
            names.sort();
            names
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

fn name_tokens(value: &str) -> Vec<String> {
    value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.strip_suffix('s').unwrap_or(token).to_string())
        .collect()
}

fn compact_name(value: &str) -> String {
    name_tokens(value).join("")
}

fn deterministic_relation_label(value: &str) -> String {
    let phrase = value
        .trim()
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mapped = match phrase.as_str() {
        "is a" | "are a" | "type of" => "IS_A",
        "part of" | "is part of" => "PART_OF",
        "use" | "uses" | "utilize" | "utilizes" => "USES",
        "depend on" | "depends on" => "DEPENDS_ON",
        "cause" | "causes" => "CAUSES",
        "enable" | "enables" => "ENABLES",
        "improve" | "improves" => "IMPROVES",
        "trained on" | "is trained on" | "is trained using" => "TRAINED_ON",
        "implement" | "implements" => "IMPLEMENTS",
        "contain" | "contains" => "CONTAINS",
        "belong to" | "belongs to" => "BELONGS_TO",
        "created by" => "CREATED_BY",
        "developed by" => "DEVELOPED_BY",
        "based on" | "is based on" => "BASED_ON",
        "evaluated on" | "is evaluated on" => "EVALUATED_ON",
        "applied to" | "is applied to" => "APPLIED_TO",
        "produce" | "produces" => "PRODUCES",
        "support" | "supports" => "SUPPORTS",
        "contrast with" | "contrasts with" => "CONTRASTS_WITH",
        "extend" | "extends" => "EXTENDS",
        _ => return crate::memory::graph::normalize_relation_label(value).unwrap_or_default(),
    };
    mapped.to_string()
}

fn relation_needs_llm(current: &str, normalized: &str) -> bool {
    const GENERIC: &[&str] = &[
        "HAS",
        "RELATED_TO",
        "RELATES_TO",
        "ASSOCIATED_WITH",
        "CONNECTED_TO",
        "LINKS",
        "LINKED_TO",
        "INVOLVES",
        "AFFECTS",
    ];
    normalized.is_empty()
        || GENERIC.contains(&normalized)
        || current.split_whitespace().count() > 5
        || normalized.split('_').count() > 5
}

fn graph_signature(nodes: &[CleanupNode], edges: &[CleanupEdge]) -> String {
    let mut sorted_nodes = nodes.to_vec();
    sorted_nodes.sort_by(|a, b| a.name.cmp(&b.name));
    let mut sorted_edges = edges.to_vec();
    sorted_edges.sort_by(|a, b| (&a.from, &a.to, &a.relation).cmp(&(&b.from, &b.to, &b.relation)));
    format!(
        "{}-{:016x}",
        CHECKPOINT_VERSION,
        stable_hash_json(&(sorted_nodes, sorted_edges))
    )
}

fn stable_hash_json(value: &impl Serialize) -> u64 {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    encoded.into_iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
    })
}

async fn load_checkpoints(db: &SurrealDB, signature: &str) -> HashMap<String, CleanupResponse> {
    let mut vars = serde_json::Map::new();
    vars.insert("signature".into(), json!(signature));
    match db
        .query_with_vars(
            "SELECT batch_key, response FROM graph_cleanup_checkpoint WHERE signature = $signature;",
            vars,
        )
        .await
    {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| {
                let key = row.get("batch_key")?.as_str()?.to_string();
                let response = serde_json::from_value(row.get("response")?.clone()).ok()?;
                Some((key, response))
            })
            .collect(),
        Err(error) => {
            log::warn!("Graph cleanup checkpoint load failed: {error}");
            HashMap::new()
        }
    }
}

async fn save_checkpoint(
    db: &SurrealDB,
    signature: &str,
    batch_key: &str,
    response: &CleanupResponse,
) {
    let record = json!({
        "signature": signature,
        "batch_key": batch_key,
        "response": response,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Err(error) = db.insert("graph_cleanup_checkpoint", record).await {
        log::warn!("Graph cleanup checkpoint save failed: {error}");
    }
}

/// Models such as Nemotron may append a short explanation or a second JSON
/// fragment after the schema-constrained object. Parse the first object and
/// deliberately ignore trailing commentary.
fn parse_cleanup_response(content: &str) -> Result<CleanupResponse, String> {
    let trimmed = content.trim().trim_matches('`').trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| "Graph cleanup JSON: no JSON object found".to_string())?;
    let parsed = serde_json::Deserializer::from_str(&trimmed[start..])
        .into_iter::<CleanupResponse>()
        .next()
        .ok_or_else(|| "Graph cleanup JSON: empty response".to_string())?
        .map_err(|error| format!("Graph cleanup JSON: {error}"));
    match parsed {
        Ok(value) => Ok(value),
        Err(error) => {
            // Never apply speculative merges from malformed model output. A
            // cleanup batch can safely be skipped and retried later.
            log::warn!("Graph cleanup returned malformed JSON; skipping batch: {error}");
            Ok(CleanupResponse::default())
        }
    }
}

fn cleanup_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "merge_groups": {
                "type": "array",
                "maxItems": 100,
                "items": {
                    "type": "object",
                    "properties": {
                        "canonical": { "type": "string", "maxLength": 180 },
                        "aliases": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": 20,
                            "items": { "type": "string", "maxLength": 180 }
                        },
                        "canonical_type": {
                            "type": "string",
                            "enum": ["Person", "Project", "Tool", "Concept", "Other"]
                        },
                        "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "reason": { "type": "string", "maxLength": 300 }
                    },
                    "required": ["canonical", "aliases", "canonical_type", "confidence", "reason"],
                    "additionalProperties": false
                }
            },
            "node_refinements": {
                "type": "array",
                "maxItems": 80,
                "items": {
                    "type": "object",
                    "properties": {
                        "current_name": { "type": "string", "maxLength": 180 },
                        "canonical_name": { "type": "string", "maxLength": 180 },
                        "canonical_type": {
                            "type": "string",
                            "enum": ["Person", "Project", "Tool", "Concept", "Other"]
                        },
                        "meaning": { "type": "string", "maxLength": 500 },
                        "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "reason": { "type": "string", "maxLength": 300 }
                    },
                    "required": ["current_name", "canonical_name", "canonical_type", "meaning", "confidence", "reason"],
                    "additionalProperties": false
                }
            },
            "relation_refinements": {
                "type": "array",
                "maxItems": 100,
                "items": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "string", "maxLength": 180 },
                        "to": { "type": "string", "maxLength": 180 },
                        "current_relation": { "type": "string", "maxLength": 300 },
                        "improved_relation": { "type": "string", "maxLength": 96 },
                        "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "reason": { "type": "string", "maxLength": 300 }
                    },
                    "required": ["from", "to", "current_relation", "improved_relation", "confidence", "reason"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["merge_groups", "node_refinements", "relation_refinements"],
        "additionalProperties": false
    })
}

fn candidate_sort_key(name: &str) -> String {
    name.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.strip_suffix('s').unwrap_or(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn deduplicate_proposals(proposals: Vec<MergeProposal>) -> Vec<MergeProposal> {
    let mut seen = HashSet::new();
    proposals
        .into_iter()
        .filter(|proposal| {
            let mut aliases: Vec<String> = proposal
                .aliases
                .iter()
                .map(|alias| alias.to_lowercase())
                .collect();
            aliases.sort();
            aliases.dedup();
            seen.insert(aliases.join("\u{1f}"))
        })
        .collect()
}

fn deduplicate_analysis(responses: Vec<CleanupResponse>) -> GraphRefinementAnalysis {
    let mut merges = Vec::new();
    let mut nodes: HashMap<String, NodeRefinementProposal> = HashMap::new();
    let mut relations: HashMap<String, RelationRefinementProposal> = HashMap::new();
    for response in responses {
        merges.extend(response.merge_groups);
        for proposal in response.node_refinements {
            let key = proposal.current_name.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let replace = nodes
                .get(&key)
                .map(|existing| proposal.confidence > existing.confidence)
                .unwrap_or(true);
            if replace {
                nodes.insert(key, proposal);
            }
        }
        for proposal in response.relation_refinements {
            let key = format!(
                "{}\u{1f}{}\u{1f}{}",
                proposal.from.trim().to_lowercase(),
                proposal.to.trim().to_lowercase(),
                proposal.current_relation.trim().to_lowercase()
            );
            let replace = relations
                .get(&key)
                .map(|existing| proposal.confidence > existing.confidence)
                .unwrap_or(true);
            if replace {
                relations.insert(key, proposal);
            }
        }
    }
    let mut node_refinements = nodes.into_values().collect::<Vec<_>>();
    node_refinements.sort_by(|a, b| a.current_name.cmp(&b.current_name));
    let mut relation_refinements = relations.into_values().collect::<Vec<_>>();
    relation_refinements.sort_by(|a, b| {
        (&a.from, &a.to, &a.current_relation).cmp(&(&b.from, &b.to, &b.current_relation))
    });
    GraphRefinementAnalysis {
        merge_groups: deduplicate_proposals(merges),
        node_refinements,
        relation_refinements,
        checkpoint_signature: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_keys_group_simple_name_variants() {
        assert_eq!(
            candidate_sort_key("Distributed Systems"),
            candidate_sort_key("distributed system")
        );
    }

    #[test]
    fn duplicate_llm_groups_are_collapsed() {
        let proposal = MergeProposal {
            canonical: "raft".into(),
            aliases: vec!["raft".into(), "raft consensus".into()],
            canonical_type: "Concept".into(),
            confidence: 0.96,
            reason: String::new(),
        };
        assert_eq!(
            deduplicate_proposals(vec![proposal.clone(), proposal]).len(),
            1
        );
    }

    #[test]
    fn cleanup_parser_accepts_trailing_model_commentary() {
        let response = r#"{"merge_groups": []}
Here is the analysis."#;
        let parsed = parse_cleanup_response(response).expect("first JSON object should parse");
        assert!(parsed.merge_groups.is_empty());
    }

    #[test]
    fn cleanup_parser_skips_malformed_model_output() {
        let parsed = parse_cleanup_response(r#"{"merge_groups":[{"canonical":"x"}"#)
            .expect("malformed cleanup should degrade safely");
        assert!(parsed.merge_groups.is_empty());
    }

    #[test]
    fn cleanup_parser_accepts_node_and_relation_refinements() {
        let response = r#"{
          "merge_groups": [],
          "node_refinements": [{
            "current_name": "rl",
            "canonical_name": "reinforcement learning",
            "canonical_type": "Concept",
            "meaning": "A learning paradigm driven by reward feedback.",
            "confidence": 0.97,
            "reason": "Expanded an unclear abbreviation."
          }],
          "relation_refinements": [{
            "from": "agent",
            "to": "environment",
            "current_relation": "interacts with the",
            "improved_relation": "INTERACTS_WITH",
            "confidence": 0.98,
            "reason": "Canonical predicate."
          }]
        }"#;
        let parsed = parse_cleanup_response(response).expect("valid refinement response");
        assert_eq!(parsed.node_refinements.len(), 1);
        assert_eq!(parsed.relation_refinements.len(), 1);
    }

    #[test]
    fn duplicate_refinements_keep_the_highest_confidence_proposal() {
        let low = NodeRefinementProposal {
            current_name: "rl".into(),
            canonical_name: "reinforcement learning".into(),
            canonical_type: "Concept".into(),
            meaning: "Low confidence meaning.".into(),
            confidence: 0.91,
            reason: String::new(),
        };
        let mut high = low.clone();
        high.meaning = "Higher confidence meaning.".into();
        high.confidence = 0.98;
        let analysis = deduplicate_analysis(vec![
            CleanupResponse {
                node_refinements: vec![low],
                ..CleanupResponse::default()
            },
            CleanupResponse {
                node_refinements: vec![high],
                ..CleanupResponse::default()
            },
        ]);
        assert_eq!(analysis.node_refinements.len(), 1);
        assert_eq!(
            analysis.node_refinements[0].meaning,
            "Higher confidence meaning."
        );
    }

    fn cleanup_node(name: &str, meaning: &str) -> CleanupNode {
        CleanupNode {
            name: name.into(),
            node_type: "Concept".into(),
            sources: vec!["test.md".into()],
            aliases: Vec::new(),
            meaning: meaning.into(),
            context: Vec::new(),
        }
    }

    #[test]
    fn incremental_plan_skips_nodes_with_meanings_and_assigns_each_edge_once() {
        let nodes = vec![
            cleanup_node("alpha", "An already analyzed concept."),
            cleanup_node("beta", ""),
            cleanup_node("gamma", "Another analyzed concept."),
        ];
        let edges = vec![
            CleanupEdge {
                from: "alpha".into(),
                relation: "uses".into(),
                to: "beta".into(),
                confidence: 0.9,
                sources: vec!["test.md".into()],
            },
            CleanupEdge {
                from: "beta".into(),
                relation: "RELATED_TO".into(),
                to: "gamma".into(),
                confidence: 0.8,
                sources: vec!["test.md".into()],
            },
        ];
        let (batches, local) = build_analysis_plan(&nodes, &edges);

        let node_work = batches
            .iter()
            .filter(|batch| batch.task == AnalysisTask::Nodes)
            .flat_map(|batch| batch.nodes.iter().map(|node| node.name.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(node_work, vec!["beta"]);
        assert_eq!(
            batches.iter().map(|batch| batch.edges.len()).sum::<usize>(),
            1
        );
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].improved_relation, "USES");
    }

    #[test]
    fn duplicate_plan_only_sends_locally_plausible_candidates() {
        let nodes = vec![
            cleanup_node("raft", "A consensus algorithm."),
            cleanup_node("raft consensus algorithm", "The Raft algorithm."),
            cleanup_node("reinforcemant learning", "A misspelled concept."),
            cleanup_node("reinforcement learning", "A learning concept."),
            cleanup_node("unrelated database", "A database."),
        ];
        let groups = duplicate_candidate_groups(&nodes);

        assert!(groups.iter().any(|group| {
            group.contains(&"raft".to_string())
                && group.contains(&"raft consensus algorithm".to_string())
        }));
        assert!(groups
            .iter()
            .all(|group| !group.contains(&"unrelated database".to_string())));
        assert!(groups.iter().any(|group| {
            group.contains(&"reinforcemant learning".to_string())
                && group.contains(&"reinforcement learning".to_string())
        }));
    }

    #[test]
    fn deterministic_relation_mapping_preserves_semantics() {
        assert_eq!(
            deterministic_relation_label("is trained using"),
            "TRAINED_ON"
        );
        assert_eq!(deterministic_relation_label("part-of"), "PART_OF");
        assert!(relation_needs_llm("related to", "RELATED_TO"));
        assert!(!relation_needs_llm("uses", "USES"));
    }

    #[tokio::test]
    #[ignore = "requires a running Ollama instance"]
    async fn live_cleanup_analyzer_finds_aliases() {
        let nodes = vec![
            CleanupNode {
                name: "raft".into(),
                node_type: "Concept".into(),
                sources: vec!["a.md".into()],
                aliases: vec![],
                meaning: String::new(),
                context: vec![],
            },
            CleanupNode {
                name: "raft consensus algorithm".into(),
                node_type: "Concept".into(),
                sources: vec!["b.md".into()],
                aliases: vec![],
                meaning: String::new(),
                context: vec![],
            },
            CleanupNode {
                name: "etcd".into(),
                node_type: "Tool".into(),
                sources: vec!["a.md".into(), "b.md".into()],
                aliases: vec![],
                meaning: String::new(),
                context: vec![],
            },
            CleanupNode {
                name: "quorum".into(),
                node_type: "Concept".into(),
                sources: vec!["a.md".into(), "b.md".into()],
                aliases: vec![],
                meaning: String::new(),
                context: vec![],
            },
        ];
        let edges = vec![
            CleanupEdge {
                from: "etcd".into(),
                relation: "IMPLEMENTS".into(),
                to: "raft".into(),
                confidence: 0.98,
                sources: vec!["a.md".into()],
            },
            CleanupEdge {
                from: "etcd".into(),
                relation: "IMPLEMENTS".into(),
                to: "raft consensus algorithm".into(),
                confidence: 0.98,
                sources: vec!["b.md".into()],
            },
            CleanupEdge {
                from: "raft".into(),
                relation: "USES".into(),
                to: "quorum".into(),
                confidence: 0.95,
                sources: vec!["a.md".into()],
            },
        ];
        let proposals = GraphCleanupAnalyzer::new(MemoryConfig::default())
            .analyze(&nodes, &edges, &crate::events::NoopEventSink)
            .await
            .expect("cleanup analysis");
        assert!(proposals.merge_groups.iter().any(|proposal| {
            let aliases: HashSet<_> = proposal
                .aliases
                .iter()
                .map(|alias| alias.to_lowercase())
                .collect();
            aliases.contains("raft") && aliases.contains("raft consensus algorithm")
        }));
    }
}
