//! Non-destructive grade graph: document, parameters, and preview execution.

mod buffer;
mod color;
pub(crate) mod cube;
mod document;
mod error;
mod exec;
mod field;
mod nodes;
mod pipeline;
mod profile;
mod store;

pub use color::rec2020_from_rgb_cam;
pub use document::{
    CameraColor, Document, FieldKind, FieldSpec, GradeNode, NodePatch, affected, apply_node_params,
    apply_saved_document, default_document, default_params, document_json, document_with_camera,
    fields, params_json, title_key,
};
pub use error::GradeError;
pub use exec::{
    CancelFlag, Frame, Rgba, bake_view, frame_from_camera_rgb, frame_from_image, render_cancellable,
};
pub use pipeline::{CompiledNode, InputPlan, NodeStatus, Pipeline, PipelineProblem};
pub use profile::parse_camera_profile;
pub use store::{
    ResourceKind, delete_resource, ensure_grade_lut, import_lut, import_preset, list_resources,
    rename_resource, user_root_from_photo,
};
