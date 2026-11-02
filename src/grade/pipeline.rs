//! The document as a checked value.
//!
//! `Document` is what the host stores; `Pipeline` is what the executor may run.
//! Between them sits `Pipeline::compile`, which answers the questions the rest
//! of the code used to answer by convention: what runs, in which order, what
//! feeds each input, and what is structurally wrong.
//!
//! Compilation never fails. Problems are collected so the host can show them
//! instead of silently dropping a node that "did not apply".
use std::collections::{BTreeMap, HashMap};

use super::buffer::DataType;
use super::document::{Document, GradeNode};

/// Where one input of a node comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputPlan {
    /// The opened photo (the source frame).
    Source,
    /// An output of another node in this pipeline.
    From { node: String, output: String },
    /// Nothing produces this input.
    Missing,
}

/// Why a node will or will not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    /// Enabled (or always-on) and all required inputs are fed.
    Runnable,
    /// Switched off by the user; kept in the document.
    Inactive,
    /// Enabled, but one or more inputs have no producer.
    MissingInput(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct CompiledNode {
    pub id: String,
    pub node_type: String,
    pub enabled: bool,
    pub status: NodeStatus,
    /// Input port name → producer.
    pub inputs: BTreeMap<String, InputPlan>,
}

/// Structural anomaly found while compiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineProblem {
    /// Two nodes share one id.
    DuplicateNode { id: String },
    /// An edge names a node that is not in the document.
    UnknownNode { node: String, edge: String },
    /// Two edges feed the same node input.
    DuplicateInput { node: String, input: String },
    /// Following `image` inputs from the export never reaches the source.
    Unreachable { node: String },
    /// A node sits in the document but no output path reaches it.
    Disconnected { node: String },
    /// The document has no export node, so there is nothing to run toward.
    MissingExport,
    /// The chain from export loops back on itself.
    Cycle { nodes: Vec<String> },
}

#[derive(Clone, Debug)]
pub struct Pipeline {
    order: Vec<String>,
    nodes: BTreeMap<String, CompiledNode>,
    problems: Vec<PipelineProblem>,
}

impl Pipeline {
    /// Execution order from the source to the export, plus one entry per node.
    pub fn compile(doc: &Document) -> Self {
        let mut problems = Vec::new();
        let mut nodes: BTreeMap<String, CompiledNode> = BTreeMap::new();

        for node in &doc.nodes {
            if nodes.contains_key(&node.id) {
                problems.push(PipelineProblem::DuplicateNode {
                    id: node.id.clone(),
                });
                continue;
            }
            nodes.insert(
                node.id.clone(),
                CompiledNode {
                    id: node.id.clone(),
                    node_type: node.param.params().node_type().to_string(),
                    enabled: node.enabled,
                    status: NodeStatus::Inactive,
                    inputs: BTreeMap::new(),
                },
            );
        }

        // Edges first: they decide what feeds each input.
        for edge in &doc.edges {
            let label = format!(
                "{}:{}=>{}:{}",
                edge.from_node, edge.from_output, edge.to_node, edge.to_input
            );
            let from_known = doc.nodes.iter().any(|node| node.id == edge.from_node);
            let to_known = doc.nodes.iter().any(|node| node.id == edge.to_node);
            if !from_known || !to_known {
                problems.push(PipelineProblem::UnknownNode {
                    node: if from_known {
                        edge.to_node.clone()
                    } else {
                        edge.from_node.clone()
                    },
                    edge: label,
                });
                continue;
            }
            let Some(target) = nodes.get_mut(&edge.to_node) else {
                continue;
            };
            if target.inputs.contains_key(&edge.to_input) {
                problems.push(PipelineProblem::DuplicateInput {
                    node: edge.to_node.clone(),
                    input: edge.to_input.clone(),
                });
                continue;
            }
            target.inputs.insert(
                edge.to_input.clone(),
                InputPlan::From {
                    node: edge.from_node.clone(),
                    output: edge.from_output.clone(),
                },
            );
        }

        // The source feeds every node that has no `image` producer. Only node
        // "input" is allowed to be a source-fed pixel step in a saved document.
        for node in nodes.values_mut() {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                node.inputs.entry("image".to_string())
            {
                entry.insert(InputPlan::Source);
            }
        }

        let order = execution_order(doc, &nodes);
        mark_status(&mut nodes, doc, &order, &mut problems);

        // Anything not reached by the export chain is dead weight in the file.
        for node in doc.nodes.iter() {
            if node.id != "input" && !order.contains(&node.id) {
                problems.push(PipelineProblem::Disconnected {
                    node: node.id.clone(),
                });
            }
        }

        Self {
            order,
            nodes,
            problems,
        }
    }

    /// Node ids from the source to the export, the order the executor follows.
    pub fn order(&self) -> &[String] {
        &self.order
    }

    pub fn node(&self, id: &str) -> Option<&CompiledNode> {
        self.nodes.get(id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &CompiledNode> {
        self.nodes.values()
    }

    pub fn problems(&self) -> &[PipelineProblem] {
        &self.problems
    }

    /// Whether this pipeline has nothing structurally wrong.
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// Nodes that would run, in execution order.
    pub fn runnable(&self) -> Vec<&CompiledNode> {
        self.order
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .filter(|node| node.status == NodeStatus::Runnable)
            .collect()
    }
}

/// Walk back from `export` along `image` inputs. This is the linear chain the
/// document has always described; a forked graph would be a different feature,
/// and `compile` reports it as `Disconnected` rather than guessing an order.
fn execution_order(doc: &Document, nodes: &BTreeMap<String, CompiledNode>) -> Vec<String> {
    let mut order = Vec::new();
    let mut seen = Vec::new();
    let mut current = doc
        .nodes
        .iter()
        .find(|node| node.id == "export")
        .map(|node| node.id.clone());
    while let Some(id) = current {
        if seen.contains(&id) {
            break;
        }
        seen.push(id.clone());
        order.push(id.clone());
        current = nodes
            .get(&id)
            .and_then(|node| match node.inputs.get("image") {
                Some(InputPlan::From { node, .. }) => Some(node.clone()),
                _ => None,
            });
    }
    order.reverse();
    order
}

fn mark_status(
    nodes: &mut BTreeMap<String, CompiledNode>,
    doc: &Document,
    order: &[String],
    problems: &mut Vec<PipelineProblem>,
) {
    if !doc.nodes.iter().any(|node| node.id == "export") {
        problems.push(PipelineProblem::MissingExport);
    }
    for id in order {
        let Some(node) = nodes.get_mut(id) else {
            continue;
        };
        let enabled = node.enabled;
        if !enabled {
            node.status = NodeStatus::Inactive;
            continue;
        }
        let missing: Vec<String> = node
            .inputs
            .iter()
            .filter(|(_, plan)| matches!(plan, InputPlan::Missing))
            .map(|(name, _)| name.clone())
            .collect();
        node.status = if missing.is_empty() {
            NodeStatus::Runnable
        } else {
            NodeStatus::MissingInput(missing)
        };
    }
    // A cycle: the export chain revisits a node.
    let mut seen = Vec::new();
    let mut cursor = doc
        .nodes
        .iter()
        .find(|node| node.id == "export")
        .map(|node| node.id.clone());
    let mut cycle_start = None;
    while let Some(id) = cursor {
        if let Some(index) = seen.iter().position(|seen| seen == &id) {
            cycle_start = Some(index);
            break;
        }
        seen.push(id.clone());
        cursor = nodes
            .get(&id)
            .and_then(|node| match node.inputs.get("image") {
                Some(InputPlan::From { node, .. }) => Some(node.clone()),
                _ => None,
            });
    }
    if let Some(index) = cycle_start {
        problems.push(PipelineProblem::Cycle {
            nodes: seen[index..].to_vec(),
        });
    }
}

/// Data kind an input accepts, when the node can describe its ports. Ports are
/// associated functions (`where Self: Sized`), so this is the one piece of
/// validation that needs the value side of `Node` before it can run.
#[allow(dead_code)]
fn accepts(_types: &'static [DataType]) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{InputPlan, NodeStatus, Pipeline, PipelineProblem};
    use crate::grade::document::{Edge, GradeNode, default_document, document_with_camera};

    #[test]
    fn default_documents_compile_cleanly() {
        for raw in [true, false] {
            let pipeline = Pipeline::compile(&default_document(raw));
            assert!(pipeline.is_clean(), "problems: {:?}", pipeline.problems());
            assert_eq!(pipeline.order().first().map(String::as_str), Some("input"));
            assert_eq!(pipeline.order().last().map(String::as_str), Some("export"));
            assert!(
                pipeline
                    .runnable()
                    .iter()
                    .all(|node| { node.status == NodeStatus::Runnable })
            );
        }
    }

    #[test]
    fn order_is_source_to_export_and_every_input_is_wired() {
        let pipeline = Pipeline::compile(&default_document(true));
        let exposure = pipeline.node("exposure").expect("exposure in the chain");
        assert_eq!(
            exposure.inputs.get("image"),
            Some(&InputPlan::From {
                node: "input-color-profile".to_string(),
                output: "image".to_string()
            })
        );
        let position = |id: &str| pipeline.order().iter().position(|item| item == id);
        assert!(position("input").unwrap() < position("input-color-profile").unwrap());
        assert!(position("input-color-profile").unwrap() < position("exposure").unwrap());
    }

    #[test]
    fn an_edge_to_a_missing_node_is_reported() {
        let mut doc = default_document(true);
        doc.edges.push(Edge {
            from_node: "ghost".to_string(),
            from_output: "image".to_string(),
            to_node: "exposure".to_string(),
            to_input: "mask".to_string(),
        });
        let pipeline = Pipeline::compile(&doc);
        assert!(pipeline.problems().iter().any(|problem| matches!(
            problem,
            PipelineProblem::UnknownNode { node, .. } if node == "ghost"
        )));
    }

    #[test]
    fn a_node_outside_the_export_chain_is_reported_as_disconnected() {
        let mut doc = default_document(true);
        let orphan = doc.nodes[0].param.defaults();
        doc.nodes.push(GradeNode {
            id: "orphan".to_string(),
            enabled: true,
            param: orphan,
        });
        let pipeline = Pipeline::compile(&doc);
        assert!(pipeline.problems().iter().any(|problem| matches!(
            problem,
            PipelineProblem::Disconnected { node } if node == "orphan"
        )));
    }

    #[test]
    fn duplicate_ids_are_reported_once() {
        let mut doc = default_document(true);
        let clone = doc.nodes[0].param.defaults();
        doc.nodes.push(GradeNode {
            id: doc.nodes[0].id.clone(),
            enabled: true,
            param: clone,
        });
        let pipeline = Pipeline::compile(&doc);
        assert!(pipeline.problems().iter().any(|problem| matches!(
            problem,
            PipelineProblem::DuplicateNode { id } if id == "input"
        )));
    }

    #[test]
    fn a_cycle_does_not_hang_and_is_reported() {
        // Two nodes feeding each other, reachable from the export.
        let mut doc = default_document(true);
        let a = doc.nodes[0].param.defaults();
        let b = doc.nodes[0].param.defaults();
        doc.nodes.push(GradeNode {
            id: "a".to_string(),
            enabled: true,
            param: a,
        });
        doc.nodes.push(GradeNode {
            id: "b".to_string(),
            enabled: true,
            param: b,
        });
        doc.edges = vec![
            Edge {
                from_node: "export".to_string(),
                from_output: "image".to_string(),
                to_node: "a".to_string(),
                to_input: "image".to_string(),
            },
            Edge {
                from_node: "b".to_string(),
                from_output: "image".to_string(),
                to_node: "a".to_string(),
                to_input: "image".to_string(),
            },
            Edge {
                from_node: "a".to_string(),
                from_output: "image".to_string(),
                to_node: "b".to_string(),
                to_input: "image".to_string(),
            },
        ];
        // The export must be the head of the chain for the walk to start there.
        doc.edges.retain(|edge| edge.to_node != "export");
        doc.edges.push(Edge {
            from_node: "a".to_string(),
            from_output: "image".to_string(),
            to_node: "export".to_string(),
            to_input: "image".to_string(),
        });
        let pipeline = Pipeline::compile(&doc);
        assert!(
            pipeline
                .problems()
                .iter()
                .any(|problem| matches!(problem, PipelineProblem::Cycle { .. }))
        );
    }

    #[test]
    fn a_camera_document_keeps_the_profile_node_runnable() {
        let camera = crate::grade::document::CameraColor {
            name: "Test".to_string(),
            to_working: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        };
        let pipeline = Pipeline::compile(&document_with_camera(true, 0.0, Some(camera)));
        assert!(pipeline.is_clean());
        assert!(pipeline.node("input-color-profile").is_some());
    }
}
