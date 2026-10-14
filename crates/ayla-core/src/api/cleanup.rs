//! Backend cleanup operations.

use super::*;

impl BackendApi<'_> {
    pub async fn analyze_and_merge_kb(
        &self,
        events: &dyn EventSink,
    ) -> Result<GraphCleanupResult, BackendError> {
        let (nodes, edges) = {
            let graph = self.manager.graph.lock().await;
            let edges = graph
                .all_edges()
                .into_iter()
                .map(|(from, relation, to, confidence, sources)| CleanupEdge {
                    from,
                    relation,
                    to,
                    confidence,
                    sources: sources.into_iter().take(12).collect(),
                })
                .collect::<Vec<_>>();
            let mut contexts: HashMap<String, Vec<String>> = HashMap::new();
            for edge in &edges {
                contexts
                    .entry(edge.from.to_lowercase())
                    .or_default()
                    .push(format!("{} {} {}", edge.from, edge.relation, edge.to));
                contexts
                    .entry(edge.to.to_lowercase())
                    .or_default()
                    .push(format!("{} {} {}", edge.from, edge.relation, edge.to));
            }
            let nodes = graph
                .all_nodes()
                .into_iter()
                .map(|node| {
                    let mut context = contexts
                        .remove(&node.name.to_lowercase())
                        .unwrap_or_default();
                    context.sort();
                    context.dedup();
                    context.truncate(12);
                    CleanupNode {
                        name: node.name.clone(),
                        node_type: format!("{:?}", node.node_type),
                        sources: node.sources.iter().take(12).cloned().collect(),
                        aliases: node.aliases.iter().take(12).cloned().collect(),
                        meaning: node.meaning.clone(),
                        context,
                    }
                })
                .collect::<Vec<_>>();
            (nodes, edges)
        };
        if nodes.is_empty() {
            return Ok(GraphCleanupResult {
                proposed_groups: 0,
                proposed_node_refinements: 0,
                proposed_relation_refinements: 0,
                merged_groups: 0,
                merged_nodes: 0,
                renamed_nodes: 0,
                updated_meanings: 0,
                updated_node_types: 0,
                relabeled_relations: 0,
                deduplicated_edges: 0,
                skipped_groups: 0,
                groups: Vec::new(),
                node_refinements: Vec::new(),
                relation_refinements: Vec::new(),
            });
        }

        let _ = events.emit(
            "kb-cleanup-progress",
            serde_json::json!({"step": "analyzing", "pct": 15}),
        );
        let analyzer = GraphCleanupAnalyzer::new(self.manager.config.clone());
        let analysis = analyzer
            .analyze(&nodes, &edges, events)
            .await
            .map_err(BackendError::from)?;
        let checkpoint_signature = analysis.checkpoint_signature.clone();
        let proposed_groups = analysis.merge_groups.len();
        let proposed_node_refinements = analysis.node_refinements.len();
        let proposed_relation_refinements = analysis.relation_refinements.len();
        let _ = events.emit(
            "kb-cleanup-progress",
            serde_json::json!({
                "step": "refining",
                "pct": 75,
                "groups": proposed_groups,
                "nodes": proposed_node_refinements,
                "relations": proposed_relation_refinements
            }),
        );

        let mut graph = self.manager.graph.lock().await;
        let mut working = graph.clone();
        let before_edges = working.edge_count();
        let node_types: HashMap<String, NodeType> = working
            .all_nodes()
            .into_iter()
            .map(|node| (node.name.to_lowercase(), node.node_type.clone()))
            .collect();
        let mut merged_groups = Vec::new();
        let mut merged_nodes = 0;
        let mut renamed_nodes = 0;
        let mut updated_meanings = 0;
        let mut updated_node_types = 0;
        let mut relabeled_relations = 0;
        let mut skipped_groups = 0;
        let mut name_resolution: HashMap<String, String> = HashMap::new();
        let mut applied_node_refinements = Vec::new();
        let mut applied_relation_refinements = Vec::new();

        for proposal in analysis.merge_groups {
            if !proposal.confidence.is_finite() || proposal.confidence < 0.90 {
                skipped_groups += 1;
                continue;
            }
            let canonical_type = match proposal.canonical_type.as_str() {
                "Person" => NodeType::Person,
                "Project" => NodeType::Project,
                "Tool" => NodeType::Tool,
                "Concept" => NodeType::Concept,
                "Other" => NodeType::Other,
                _ => {
                    skipped_groups += 1;
                    continue;
                }
            };
            let canonical = proposal.canonical.trim().to_lowercase();
            let mut names: HashSet<String> = proposal
                .aliases
                .iter()
                .map(|alias| alias.trim().to_lowercase())
                .filter(|alias| !alias.is_empty())
                .collect();
            names.insert(canonical.clone());
            let existing_names: Vec<String> = names
                .into_iter()
                .filter(|name| node_types.contains_key(name))
                .collect();
            let contains_document = existing_names
                .iter()
                .any(|name| node_types.get(name) == Some(&NodeType::Document));
            if canonical.is_empty() || existing_names.len() < 2 || contains_document {
                skipped_groups += 1;
                continue;
            }

            let removed =
                working.merge_node_group(&canonical, &existing_names, Some(canonical_type));
            if removed == 0 {
                skipped_groups += 1;
                continue;
            }
            merged_nodes += removed;
            for alias in &existing_names {
                name_resolution.insert(alias.to_lowercase(), canonical.clone());
            }
            merged_groups.push(GraphCleanupGroup {
                canonical,
                aliases: existing_names,
                confidence: proposal.confidence,
                reason: proposal.reason,
            });
        }

        let mut node_refinement_proposals = analysis.node_refinements;
        node_refinement_proposals.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut refined_targets = HashSet::new();
        for proposal in node_refinement_proposals {
            if !proposal.confidence.is_finite() || proposal.confidence < 0.90 {
                skipped_groups += 1;
                continue;
            }
            let raw_current = proposal.current_name.trim().to_lowercase();
            let current = resolve_graph_name(&raw_current, &name_resolution);
            // An alias that was already merged must not rename the surviving
            // canonical node based on an obsolete alias-specific proposal.
            if current != raw_current || !refined_targets.insert(current.clone()) {
                continue;
            }
            let Some(existing) = working
                .all_nodes()
                .into_iter()
                .find(|node| node.name.eq_ignore_ascii_case(&current))
                .cloned()
            else {
                skipped_groups += 1;
                continue;
            };
            if existing.node_type == NodeType::Document {
                skipped_groups += 1;
                continue;
            }
            let Some(canonical_type) = cleanup_node_type(&proposal.canonical_type) else {
                skipped_groups += 1;
                continue;
            };
            let canonical = proposal.canonical_name.trim().to_lowercase();
            let name_changed = !canonical.is_empty() && canonical != current;
            let meaning_changed =
                !proposal.meaning.trim().is_empty() && existing.meaning != proposal.meaning.trim();
            let type_changed = existing.node_type != canonical_type;
            if !working.refine_node(
                &current,
                &canonical,
                Some(canonical_type),
                &proposal.meaning,
            ) {
                continue;
            }
            if name_changed {
                renamed_nodes += 1;
                name_resolution.insert(current.clone(), canonical.clone());
            }
            if meaning_changed {
                updated_meanings += 1;
            }
            if type_changed {
                updated_node_types += 1;
            }
            applied_node_refinements.push(GraphNodeRefinement {
                previous_name: existing.name,
                canonical_name: if canonical.is_empty() {
                    current
                } else {
                    canonical
                },
                meaning: proposal.meaning.trim().to_string(),
                confidence: proposal.confidence,
                reason: proposal.reason,
            });
        }

        for proposal in analysis.relation_refinements {
            if !proposal.confidence.is_finite() || proposal.confidence < 0.92 {
                skipped_groups += 1;
                continue;
            }
            let from = resolve_graph_name(&proposal.from, &name_resolution);
            let to = resolve_graph_name(&proposal.to, &name_resolution);
            let Some(improved_relation) =
                crate::memory::graph::normalize_relation_label(&proposal.improved_relation)
            else {
                skipped_groups += 1;
                continue;
            };
            if improved_relation == "RELATED_TO"
                && !proposal.current_relation.eq_ignore_ascii_case("RELATED_TO")
            {
                skipped_groups += 1;
                continue;
            }
            if !working.refine_relation(&from, &to, &proposal.current_relation, &improved_relation)
            {
                continue;
            }
            relabeled_relations += 1;
            applied_relation_refinements.push(GraphRelationRefinement {
                from,
                to,
                previous_relation: proposal.current_relation,
                improved_relation,
                confidence: proposal.confidence,
                reason: proposal.reason,
            });
        }

        if merged_nodes > 0
            || renamed_nodes > 0
            || updated_meanings > 0
            || updated_node_types > 0
            || relabeled_relations > 0
        {
            working.sync().await.map_err(BackendError::from)?;
            *graph = working;
        }
        let deduplicated_edges = before_edges.saturating_sub(graph.edge_count());
        let _ = events.emit(
            "kb-cleanup-progress",
            serde_json::json!({
                "step": "complete",
                "pct": 100,
                "merged_nodes": merged_nodes,
                "renamed_nodes": renamed_nodes,
                "updated_meanings": updated_meanings,
                "updated_node_types": updated_node_types,
                "relabeled_relations": relabeled_relations
            }),
        );
        drop(graph);
        analyzer.clear_checkpoint(&checkpoint_signature).await;
        Ok(GraphCleanupResult {
            proposed_groups,
            proposed_node_refinements,
            proposed_relation_refinements,
            merged_groups: merged_groups.len(),
            merged_nodes,
            renamed_nodes,
            updated_meanings,
            updated_node_types,
            relabeled_relations,
            deduplicated_edges,
            skipped_groups,
            groups: merged_groups,
            node_refinements: applied_node_refinements,
            relation_refinements: applied_relation_refinements,
        })
    }
}
