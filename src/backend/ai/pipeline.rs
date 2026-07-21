//! Read → decode → GPU infer pipeline for local AI image jobs.
//!
//! All inter-stage queues use **`tokio::sync::mpsc`** (async). Producers call
//! `.send().await` / consumers `.recv().await` so a full queue yields the task rather
//! than blocking the runtime thread. **`std::sync::mpsc` is never used here.**
//!
//! 1. **Read** (single async task): serial `tokio::fs::read`, push to decode queue.
//! 2. **Decode** (N parallel workers): decode bytes off the CPU thread pool, push to infer queue.
//! 3. **Infer** (batch consumer): drain up to `gpu_batch_size` decoded images (no wait to fill
//!    the batch when the queue is short), run GPU/CPU inference for the batch.

use image::DynamicImage;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::backend::album::decoder::{DecodeOptions, ImageDecoder};

use super::AiItemError;

#[derive(Debug, Clone, Copy)]
pub struct PipelineConfig {
    /// Concurrent decode workers (`ai_cpu_threads`).
    pub decode_parallelism: usize,
    /// Max images per GPU infer batch (`ai_batch_size`). If fewer are queued, all are taken.
    pub gpu_batch_size: usize,
}

impl PipelineConfig {
    /// Bounded decode-stage queue: read → decode. Backpressure when full.
    pub fn decode_queue_capacity(&self) -> usize {
        self.decode_parallelism.saturating_mul(2).max(1)
    }

    /// Bounded infer-stage queue: decode → GPU. Backpressure when full.
    pub fn infer_queue_capacity(&self) -> usize {
        self.gpu_batch_size.saturating_mul(2).max(1)
    }

    /// Bounded progress notifications: infer → task UI.
    pub fn progress_queue_capacity(&self) -> usize {
        self.gpu_batch_size.saturating_mul(4).max(8)
    }
}

/// Bounded async progress channel (see [`progress_channel`]).
pub type PipelineProgressTx = mpsc::Sender<(usize, PathBuf)>;
pub type PipelineProgressRx = mpsc::Receiver<(usize, PathBuf)>;

/// Create a bounded async progress queue (`tokio::sync::mpsc`, not `std::sync::mpsc`).
pub fn progress_channel(capacity: usize) -> (PipelineProgressTx, PipelineProgressRx) {
    mpsc::channel(capacity.max(1))
}

struct ReadJob {
    index: usize,
    path: PathBuf,
    bytes: Result<Vec<u8>, AiItemError>,
}

struct InferJob {
    index: usize,
    path: PathBuf,
    image: Result<DynamicImage, AiItemError>,
}

fn decode_bytes_with_decoder(
    decoder: &dyn ImageDecoder,
    bytes: &[u8],
    options: DecodeOptions,
) -> Result<DynamicImage, AiItemError> {
    decoder
        .decode_bytes(bytes, options)
        .map_err(|error| AiItemError {
            code: error.code.to_string(),
            message: error.message,
        })
}

/// Run the staged pipeline; output order matches `paths`.
pub async fn run_image_pipeline<R, F>(
    paths: &[PathBuf],
    config: PipelineConfig,
    decoder: Arc<dyn ImageDecoder>,
    decode_apply_exif: bool,
    infer_fn: F,
    progress: Option<PipelineProgressTx>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<R>, String>
where
    R: Send + 'static,
    F: Fn(Result<DynamicImage, AiItemError>) -> R + Send + Sync + 'static,
{
    let total = paths.len();
    if total == 0 {
        return Ok(Vec::new());
    }

    let decode_parallelism = config.decode_parallelism.max(1).min(total);
    let gpu_batch_size = config.gpu_batch_size.max(1);
    let decode_cap = config.decode_queue_capacity();
    let infer_cap = config.infer_queue_capacity();

    let (decode_tx, mut decode_rx) = mpsc::channel::<ReadJob>(decode_cap);
    let (infer_tx, mut infer_rx) = mpsc::channel::<InferJob>(infer_cap);

    let paths_for_reader = paths.to_vec();
    let reader = tokio::spawn(async move {
        for (index, path) in paths_for_reader.into_iter().enumerate() {
            let bytes = tokio::fs::read(&path).await.map_err(|error| AiItemError {
                code: "read_failed".to_string(),
                message: error.to_string(),
            });
            // Async backpressure: when decode queue is full, yield until a slot opens.
            if decode_tx
                .send(ReadJob {
                    index,
                    path,
                    bytes,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let infer_tx_decode = infer_tx.clone();
    let decode_workers = tokio::spawn(async move {
        let mut join_set = JoinSet::new();
        while let Some(job) = decode_rx.recv().await {
            while join_set.len() >= decode_parallelism {
                if join_set.join_next().await.is_none() {
                    break;
                }
            }
            let infer_tx = infer_tx_decode.clone();
            let decoder = Arc::clone(&decoder);
            join_set.spawn(async move {
                let decode_options = if decode_apply_exif {
                    DecodeOptions::new()
                } else {
                    DecodeOptions::raw_pixels()
                };
                let image = match job.bytes {
                    Ok(bytes) => tokio::task::spawn_blocking(move || {
                        decode_bytes_with_decoder(decoder.as_ref(), &bytes, decode_options)
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(AiItemError {
                            code: "decode_failed".to_string(),
                            message: format!("decode join failed: {error}"),
                        })
                    }),
                    Err(error) => Err(error),
                };
                // Async backpressure on infer queue when GPU stage falls behind.
                let _ = infer_tx
                    .send(InferJob {
                        index: job.index,
                        path: job.path,
                        image,
                    })
                    .await;
            });
        }
        while join_set.join_next().await.is_some() {}
        drop(infer_tx_decode);
    });

    let infer_fn = Arc::new(infer_fn);
    let mut slots: Vec<Option<R>> = (0..total).map(|_| None).collect();
    let mut completed = 0usize;

    while completed < total {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            reader.abort();
            decode_workers.abort();
            return Err("pipeline cancelled".to_string());
        }

        let Some(first) = infer_rx.recv().await else {
            break;
        };
        let mut batch = Vec::with_capacity(gpu_batch_size.min(total.saturating_sub(completed)));
        batch.push(first);
        while batch.len() < gpu_batch_size {
            match infer_rx.try_recv() {
                Ok(job) => batch.push(job),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        let infer_fn = Arc::clone(&infer_fn);
        let batch_results = tokio::task::spawn_blocking(move || {
            batch
                .into_iter()
                .map(|job| {
                    let result = infer_fn(job.image);
                    (job.index, job.path, result)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| format!("infer batch join failed: {error}"))?;

        for (index, path, result) in batch_results {
            if index >= total {
                return Err(format!("pipeline index out of range: {index} >= {total}"));
            }
            slots[index] = Some(result);
            completed += 1;
            if let Some(tx) = progress.as_ref() {
                let _ = tx.send((index, path)).await;
            }
        }
    }

    let _ = reader.await;
    let _ = decode_workers.await;

    if completed < total {
        return Err(format!(
            "pipeline incomplete: processed {completed} of {total} images"
        ));
    }
    if slots.iter().any(|slot| slot.is_none()) {
        return Err("pipeline missing result slots".to_string());
    }
    Ok(slots.into_iter().map(|slot| slot.expect("checked")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_capacities_are_at_least_one() {
        let config = PipelineConfig {
            decode_parallelism: 0,
            gpu_batch_size: 0,
        };
        assert_eq!(config.decode_queue_capacity(), 1);
        assert_eq!(config.infer_queue_capacity(), 1);
        assert_eq!(config.progress_queue_capacity(), 8);
    }
}
