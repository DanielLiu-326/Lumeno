//! Domain AI pipelines owned by the index task (Task → Pipeline → Provider).

mod duplicate;
mod embedding;
mod face;
mod finalize;

pub(crate) use duplicate::run_duplicate_pipeline;
pub(crate) use embedding::run_embedding_pipeline;
pub(crate) use face::run_face_pipeline;
pub(crate) use finalize::run_finalize_pipeline;
