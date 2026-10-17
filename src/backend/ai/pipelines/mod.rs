//! AI index pipeline hierarchy.
//!
//! Start at [`AiIndexPipeline`]:
//!
//! `scan → duplicates → embeddings → faces → finalize`
//!
//! Embedding and face inference use the shared `ImagePipeline`
//! (`read → decode → infer → save`) from the parent `pipeline` module.

mod duplicate;
mod embedding;
mod face;
mod finalize;
mod index;
pub(crate) mod scan;

pub(crate) use face::rebuild_album_face_clusters;
pub(crate) use index::{AiIndexPipeline, IndexPipelineError};
