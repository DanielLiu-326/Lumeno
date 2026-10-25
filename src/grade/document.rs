//! Grade document for one open photo. The host writes `{filename}.lumeno.json`; this module only
//! turns the document into that JSON and applies a saved copy back onto a fresh document.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde_json::{Value, json};

use super::error::GradeError;
use super::exec::ExecCtx;
use super::nodes::{self, Node};

#[derive(Clone, Debug)]
pub struct Document {
    pub raw: bool,
    pub nodes: Vec<GradeNode>,
    pub edges: Vec<Edge>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from_node: String,
    pub from_output: String,
    pub to_node: String,
    pub to_input: String,
}

pub struct GradeNode {
    pub id: String,
    pub enabled: bool,
    pub param: Box<dyn Node>,
}

impl Clone for GradeNode {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            enabled: self.enabled,
            param: {
                let mut param = self.param.defaults();
                param
                    .set_param(&self.param.params().to_json())
                    .expect("grade node repeats its parameters");
                param
            },
        }
    }
}

impl fmt::Debug for GradeNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GradeNode")
            .field("id", &self.id)
            .field("enabled", &self.enabled)
            .field("type", &self.param.params().node_type())
            .field("params", &self.param.params().to_json())
            .finish()
    }
}

impl GradeNode {
    pub fn always_on(&self) -> bool {
        self.param.params().always_on()
    }

    pub fn run(
        &mut self,
        inputs: &BTreeMap<String, Arc<super::buffer::Value>>,
        ctx: &ExecCtx,
    ) -> Result<BTreeMap<String, super::buffer::Value>, GradeError> {
        if ctx.cancel.is_cancelled() {
            return Err(GradeError::Cancelled);
        }
        self.param.run(inputs, ctx)
    }
}

#[derive(Clone, Debug)]
pub struct NodePatch {
    pub id: String,
    pub enabled: bool,
    pub params: Value,
}

pub use super::field::{FieldKind, FieldSpec};

pub fn default_document(raw: bool) -> Document {
    document_with_baseline(raw, 0.0)
}

/// Camera matrix for one open RAW. `to_working` is linear camera RGB → linear Rec.2020.
pub struct CameraColor {
    pub name: String,
    pub to_working: [f32; 9],
}

/// `baseline_ev` is the camera exposure offset for a RAW file. Raster files ignore it.
pub fn document_with_baseline(raw: bool, baseline_ev: f32) -> Document {
    document_with_camera(raw, baseline_ev, None)
}

pub fn document_with_camera(raw: bool, baseline_ev: f32, camera: Option<CameraColor>) -> Document {
    let mut list = Vec::new();
    list.push(vertex("input", nodes::input(raw), true));
    if raw {
        list.push(vertex("white-balance", nodes::white_balance(), true));
        list.push(vertex("demosaic", nodes::demosaic(), true));
        // Dark channel prior reads min(R,G,B). The working-space matrix mixes
        // those channels, so dehaze stays on camera RGB.
        list.push(vertex("dehaze", nodes::dehaze(), true));
        let profile = camera.map(|color| (color.to_working, color.name));
        list.push(vertex(
            "input-color-profile",
            nodes::input_color_profile(profile),
            true,
        ));
    }
    // JPEG and other rendered files already carry a look. Tone, color, and
    // detail nodes stay in the document so they can be switched on, but they start off.
    let exposure_ev = if raw { baseline_ev } else { 0.0 };
    list.push(vertex("exposure", nodes::exposure_at(exposure_ev), raw));
    list.push(vertex("local-contrast", nodes::local_contrast(), raw));
    if !raw {
        list.push(vertex("dehaze", nodes::dehaze(), false));
    }
    list.push(vertex("velvia", nodes::velvia(), raw));
    list.push(vertex("denoise", nodes::denoise(), raw));
    list.push(vertex("sharpen", nodes::sharpen(), raw));
    list.push(vertex("rotate", nodes::rotate(), true));
    list.push(vertex("crop", nodes::crop(), true));
    list.push(vertex("sigmoid", nodes::sigmoid(), raw));
    // No cube is selected yet, so the module starts off on both raw and rendered files.
    list.push(vertex("lut", nodes::lut(), false));
    list.push(vertex("export", nodes::export(), true));
    let edges = list
        .windows(2)
        .map(|pair| Edge {
            from_node: pair[0].id.clone(),
            from_output: "image".into(),
            to_node: pair[1].id.clone(),
            to_input: "image".into(),
        })
        .collect();
    Document {
        raw,
        nodes: list,
        edges,
    }
}

fn vertex(id: &str, param: Box<dyn Node>, enabled: bool) -> GradeNode {
    GradeNode {
        id: id.to_string(),
        enabled,
        param,
    }
}

pub fn apply_node_params(doc: &mut Document, patch: &NodePatch) -> Result<(), String> {
    let node = doc
        .nodes
        .iter_mut()
        .find(|node| node.id == patch.id)
        .ok_or_else(|| format!("unknown node {}", patch.id))?;
    let before = node.param.params().to_json();
    if node.always_on() && !patch.enabled && patch.params == before {
        return Err("always-on".into());
    }
    node.param
        .set_param(&patch.params)
        .map_err(|error| error.to_string())?;
    let changed = node.param.params().to_json() != before;
    if !changed && node.always_on() && !patch.enabled {
        return Err("always-on".into());
    }
    node.enabled = if changed { true } else { patch.enabled };
    Ok(())
}

pub fn params_json(node: &GradeNode) -> Value {
    node.param.params().to_json()
}

pub fn document_json(doc: &Document) -> Value {
    json!({
        "version": 1,
        "raw": doc.raw,
        "nodes": doc.nodes.iter().map(|node| json!({
            "id": node.id,
            "type": node.param.params().node_type(),
            "enabled": node.enabled,
            "params": node.param.params().to_json(),
        })).collect::<Vec<_>>()
    })
}

/// Copy saved node switches and parameters onto `doc`. Unknown ids are skipped.
/// Camera matrices and the exposure baseline stay on the nodes already built for this file.
pub fn apply_saved_document(doc: &mut Document, saved: &Value) -> Result<(), String> {
    let nodes = saved
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "grade file".to_string())?;
    for item in nodes {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(node) = doc.nodes.iter_mut().find(|node| node.id == id) else {
            continue;
        };
        if let Some(params) = item.get("params") {
            if node.param.set_param(params).is_err() {
                continue;
            }
        }
        if node.always_on() {
            node.enabled = true;
        } else if let Some(enabled) = item.get("enabled").and_then(Value::as_bool) {
            node.enabled = enabled;
        }
    }
    Ok(())
}

pub fn default_params(node: &GradeNode) -> Value {
    node.param.defaults().params().to_json()
}

pub fn affected(doc: &Document, start: &str) -> HashSet<String> {
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &doc.edges {
        next.entry(edge.from_node.as_str())
            .or_default()
            .push(edge.to_node.as_str());
    }
    let mut out = HashSet::new();
    let mut stack = vec![start];
    while let Some(id) = stack.pop() {
        if !out.insert(id.to_string()) {
            continue;
        }
        if let Some(children) = next.get(id) {
            stack.extend(children.iter().copied());
        }
    }
    out
}

pub fn upstream<'a>(doc: &'a Document, id: &str) -> Option<&'a str> {
    doc.edges
        .iter()
        .find(|edge| edge.to_node == id && edge.to_input == "image")
        .map(|edge| edge.from_node.as_str())
}

pub fn title(locale: &str, id: &str) -> &'static str {
    let zh = locale.starts_with("zh");
    match (zh, id) {
        (true, "input") => "输入",
        (false, "input") => "Input",
        (true, "white-balance") => "白平衡",
        (false, "white-balance") => "White Balance",
        (true, "demosaic") => "去马赛克",
        (false, "demosaic") => "Demosaic",
        (true, "input-color-profile") => "输入色彩配置",
        (false, "input-color-profile") => "Input Color Profile",
        (true, "exposure") => "曝光",
        (false, "exposure") => "Exposure",
        (true, "local-contrast") => "局部对比",
        (false, "local-contrast") => "Local Contrast",
        (true, "dehaze") => "去雾",
        (false, "dehaze") => "Dehaze",
        (true, "velvia") => "增色",
        (false, "velvia") => "Velvia",
        (true, "denoise") => "降噪",
        (false, "denoise") => "Denoise",
        (true, "sharpen") => "锐化",
        (false, "sharpen") => "Sharpen",
        (true, "rotate") => "旋转",
        (false, "rotate") => "Rotate",
        (true, "crop") => "裁切",
        (false, "crop") => "Crop",
        (true, "sigmoid") => "影调",
        (false, "sigmoid") => "Sigmoid",
        (true, "lut") => "LUT",
        (false, "lut") => "LUT",
        (true, "export") => "导出",
        (false, "export") => "Export",
        _ => "Node",
    }
}

pub fn fields(locale: &str, node: &GradeNode) -> Vec<FieldSpec> {
    node.param.params().fields(locale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn raster_starts_without_tone_or_color() {
        let doc = default_document(false);
        assert!(
            doc.nodes
                .iter()
                .all(|node| node.id != "input-color-profile")
        );
        for id in [
            "exposure",
            "local-contrast",
            "dehaze",
            "velvia",
            "denoise",
            "sharpen",
            "sigmoid",
        ] {
            let node = doc.nodes.iter().find(|node| node.id == id).unwrap();
            assert!(
                !node.enabled,
                "{id} should start disabled on rendered files"
            );
        }
        let raw = default_document(true);
        for id in [
            "exposure",
            "local-contrast",
            "dehaze",
            "velvia",
            "denoise",
            "sharpen",
            "sigmoid",
            "white-balance",
            "demosaic",
            "input-color-profile",
        ] {
            let node = raw.nodes.iter().find(|node| node.id == id).unwrap();
            assert!(node.enabled, "{id} should start enabled on raw");
        }
        assert_eq!(upstream(&raw, "dehaze"), Some("demosaic"));
        assert_eq!(upstream(&raw, "input-color-profile"), Some("dehaze"));
        assert_eq!(upstream(&doc, "dehaze"), Some("local-contrast"));
    }

    #[test]
    fn raw_exposure_starts_at_camera_baseline() {
        let mut doc = document_with_baseline(true, 0.75);
        let exposure = doc.nodes.iter().find(|node| node.id == "exposure").unwrap();
        assert!((exposure.param.params().to_json()["ev"].as_f64().unwrap() - 0.75).abs() < 1e-5);
        assert!((default_params(exposure)["ev"].as_f64().unwrap() - 0.75).abs() < 1e-5);
        apply_node_params(
            &mut doc,
            &NodePatch {
                id: "exposure".into(),
                enabled: true,
                params: json!({ "ev": 1.5, "black": 0.0 }),
            },
        )
        .unwrap();
        let exposure = doc.nodes.iter().find(|node| node.id == "exposure").unwrap();
        assert!((exposure.param.params().to_json()["ev"].as_f64().unwrap() - 1.5).abs() < 1e-5);
        assert!((default_params(exposure)["ev"].as_f64().unwrap() - 0.75).abs() < 1e-5);
        let raster = document_with_baseline(false, 0.75);
        let exposure = raster
            .nodes
            .iter()
            .find(|node| node.id == "exposure")
            .unwrap();
        assert!(!exposure.enabled);
        assert!(
            exposure.param.params().to_json()["ev"]
                .as_f64()
                .unwrap()
                .abs()
                < 1e-5
        );
    }

    #[test]
    fn always_on_nodes_reject_being_switched_off() {
        let mut doc = default_document(true);
        for id in ["demosaic", "input-color-profile", "export"] {
            let params = doc
                .nodes
                .iter()
                .find(|node| node.id == id)
                .unwrap()
                .param
                .params()
                .to_json();
            let err = apply_node_params(
                &mut doc,
                &NodePatch {
                    id: id.into(),
                    enabled: false,
                    params,
                },
            )
            .unwrap_err();
            assert_eq!(err, "always-on");
            let node = doc.nodes.iter().find(|node| node.id == id).unwrap();
            assert!(node.enabled, "{id}");
        }
    }

    #[test]
    fn a_saved_file_restores_exposure_without_touching_the_baseline() {
        let mut doc = default_document(true);
        apply_node_params(
            &mut doc,
            &NodePatch {
                id: "exposure".into(),
                enabled: true,
                params: json!({ "ev": 1.25, "black": 0.0 }),
            },
        )
        .unwrap();
        let saved = document_json(&doc);
        let mut fresh = default_document(true);
        apply_saved_document(&mut fresh, &saved).unwrap();
        let exposure = fresh.nodes.iter().find(|node| node.id == "exposure").unwrap();
        assert!((exposure.param.params().to_json()["ev"].as_f64().unwrap() - 1.25).abs() < 1e-4);
        assert!(exposure.enabled);
    }

    #[test]
    fn changing_params_turns_the_node_on() {
        let mut doc = default_document(false);
        let exposure = doc.nodes.iter().find(|node| node.id == "exposure").unwrap();
        assert!(!exposure.enabled);
        let mut params = exposure.param.params().to_json();
        apply_node_params(
            &mut doc,
            &NodePatch {
                id: "exposure".into(),
                enabled: false,
                params: params.clone(),
            },
        )
        .unwrap();
        let exposure = doc.nodes.iter().find(|node| node.id == "exposure").unwrap();
        assert!(!exposure.enabled);
        params["ev"] = json!(0.4);
        apply_node_params(
            &mut doc,
            &NodePatch {
                id: "exposure".into(),
                enabled: false,
                params,
            },
        )
        .unwrap();
        let exposure = doc.nodes.iter().find(|node| node.id == "exposure").unwrap();
        assert!(exposure.enabled);
        assert!((exposure.param.params().to_json()["ev"].as_f64().unwrap() - 0.4).abs() < 1e-5);
    }

    #[test]
    fn crop_edit_dirties_only_downstream() {
        let doc = default_document(false);
        let dirty = affected(&doc, "crop");
        assert!(dirty.contains("crop"));
        assert!(dirty.contains("sigmoid"));
        assert!(dirty.contains("export"));
        assert!(!dirty.contains("exposure"));
        assert!(!dirty.contains("input"));
    }

    #[test]
    fn exposure_patch_replaces_the_whole_node() {
        let mut doc = default_document(false);
        apply_node_params(
            &mut doc,
            &NodePatch {
                id: "exposure".into(),
                enabled: true,
                params: json!({ "ev": 1.5, "black": 0.02 }),
            },
        )
        .unwrap();
        let params = doc
            .nodes
            .iter()
            .find(|node| node.id == "exposure")
            .unwrap()
            .param
            .params()
            .to_json();
        assert!((params["ev"].as_f64().unwrap() - 1.5).abs() < 1e-5);
        assert!((params["black"].as_f64().unwrap() - 0.02).abs() < 1e-5);
    }

    #[test]
    fn param_json_rebuilds_exposure() {
        let node = nodes::param_from_json("exposure", &json!({ "ev": 0.5, "black": 0.0 })).unwrap();
        assert_eq!(node.params().node_type(), "exposure");
        let again =
            nodes::param_from_json("exposure", &json!({ "ev": -1.0, "black": 0.01 })).unwrap();
        let params = again.params().to_json();
        assert!((params["ev"].as_f64().unwrap() + 1.0).abs() < 1e-5);
    }
}
