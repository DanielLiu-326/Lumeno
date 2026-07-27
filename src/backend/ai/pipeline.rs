//! Shared bounded image pipeline: read → decode → infer → save.
//!
//! Domain pipelines own cache keys and index rules; this module only moves
//! bytes/images/results through stages with backpressure and bottleneck logs.

use image::DynamicImage;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::backend::album::decoder::{DecodeOptions, ImageDecoder, decode_bytes_with_path};

use super::provider::AiItemError;

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Pipeline name for logs (e.g. `embedding`, `face`).
    pub name: String,
    /// Concurrent decode workers (`ai_cpu_threads`).
    pub decode_parallelism: usize,
    /// Max images per infer batch (`ai_batch_size`).
    pub gpu_batch_size: usize,
    /// Save-stage queue capacity (defaults to `gpu_batch_size * 2`).
    pub save_queue_capacity: Option<usize>,
    pub decode_apply_exif: bool,
    /// Log when `send` waits longer than this.
    pub congestion_wait_threshold: Duration,
    /// Per-edge throttle for congestion warnings.
    pub congestion_log_interval: Duration,
}

impl PipelineConfig {
    pub fn new(name: impl Into<String>, decode_parallelism: usize, gpu_batch_size: usize) -> Self {
        Self {
            name: name.into(),
            decode_parallelism,
            gpu_batch_size,
            save_queue_capacity: None,
            decode_apply_exif: true,
            congestion_wait_threshold: Duration::from_millis(200),
            congestion_log_interval: Duration::from_secs(3),
        }
    }

    pub fn decode_queue_capacity(&self) -> usize {
        self.decode_parallelism.saturating_mul(2).max(1)
    }

    pub fn infer_queue_capacity(&self) -> usize {
        self.gpu_batch_size.saturating_mul(2).max(1)
    }

    pub fn save_queue_capacity(&self) -> usize {
        self.save_queue_capacity
            .unwrap_or_else(|| self.gpu_batch_size.saturating_mul(2).max(1))
    }

    pub fn progress_queue_capacity(&self) -> usize {
        self.gpu_batch_size.saturating_mul(4).max(8)
    }
}

/// Bounded async progress channel: `(job_index, display_path)`.
pub type PipelineProgressTx = mpsc::Sender<(usize, PathBuf)>;
pub type PipelineProgressRx = mpsc::Receiver<(usize, PathBuf)>;

pub fn progress_channel(capacity: usize) -> (PipelineProgressTx, PipelineProgressRx) {
    mpsc::channel(capacity.max(1))
}

#[derive(Debug)]
pub struct ImageJob<M> {
    pub index: usize,
    pub path: PathBuf,
    pub meta: M,
}

struct ReadJob<M> {
    index: usize,
    path: PathBuf,
    meta: M,
    bytes: Result<Vec<u8>, AiItemError>,
}

struct InferJob<M> {
    index: usize,
    path: PathBuf,
    meta: M,
    image: Result<DynamicImage, AiItemError>,
}

struct SaveJob<M, Out> {
    index: usize,
    path: PathBuf,
    meta: M,
    result: Result<Out, AiItemError>,
    /// Decoded image when decode succeeded (available after infer for crop/side writes).
    image: Option<DynamicImage>,
}

struct EdgeDiag {
    from_to: &'static str,
    capacity: usize,
    max_depth: AtomicUsize,
    max_wait_ms: AtomicU64,
    wait_over_threshold: AtomicUsize,
    last_log: Mutex<Option<Instant>>,
}

struct StageDiag {
    items: AtomicUsize,
    total_ms: AtomicU64,
}

struct PipelineDiag {
    name: String,
    threshold: Duration,
    log_interval: Duration,
    read_to_decode: EdgeDiag,
    decode_to_infer: EdgeDiag,
    infer_to_save: EdgeDiag,
    read: StageDiag,
    decode: StageDiag,
    infer: StageDiag,
    save: StageDiag,
}

impl PipelineDiag {
    fn new(config: &PipelineConfig) -> Arc<Self> {
        Arc::new(Self {
            name: config.name.clone(),
            threshold: config.congestion_wait_threshold,
            log_interval: config.congestion_log_interval,
            read_to_decode: EdgeDiag::new("read→decode", config.decode_queue_capacity()),
            decode_to_infer: EdgeDiag::new("decode→infer", config.infer_queue_capacity()),
            infer_to_save: EdgeDiag::new("infer→save", config.save_queue_capacity()),
            read: StageDiag::default(),
            decode: StageDiag::default(),
            infer: StageDiag::default(),
            save: StageDiag::default(),
        })
    }

    fn record_stage(stage: &StageDiag, elapsed: Duration) {
        stage.items.fetch_add(1, Ordering::Relaxed);
        stage
            .total_ms
            .fetch_add(elapsed.as_millis() as u64, Ordering::Relaxed);
    }

    fn note_send(&self, edge: &EdgeDiag, wait: Duration, remaining_capacity: usize) {
        let depth = edge.capacity.saturating_sub(remaining_capacity);
        edge.max_depth.fetch_max(depth, Ordering::Relaxed);
        let wait_ms = wait.as_millis() as u64;
        edge.max_wait_ms.fetch_max(wait_ms, Ordering::Relaxed);
        if wait < self.threshold {
            return;
        }
        edge.wait_over_threshold.fetch_add(1, Ordering::Relaxed);
        let mut last = edge.last_log.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if last.is_some_and(|t| now.duration_since(t) < self.log_interval) {
            return;
        }
        *last = Some(now);
        eprintln!(
            "pipeline={} stage={} wait_ms={} depth={}/{}",
            self.name, edge.from_to, wait_ms, depth, edge.capacity
        );
    }

    fn summary_line(&self) -> String {
        let fmt_stage = |label: &str, stage: &StageDiag| {
            let items = stage.items.load(Ordering::Relaxed);
            let total = stage.total_ms.load(Ordering::Relaxed);
            let avg = if items == 0 { 0 } else { total / items as u64 };
            format!("{label}:items={items},avg_ms={avg}")
        };
        let fmt_edge = |edge: &EdgeDiag| {
            format!(
                "{}:max_wait_ms={},max_depth={}/{},congested={}",
                edge.from_to,
                edge.max_wait_ms.load(Ordering::Relaxed),
                edge.max_depth.load(Ordering::Relaxed),
                edge.capacity,
                edge.wait_over_threshold.load(Ordering::Relaxed) > 0
            )
        };
        format!(
            "pipeline={} summary {} | {} | {} | {} || {} | {} | {}",
            self.name,
            fmt_stage("read", &self.read),
            fmt_stage("decode", &self.decode),
            fmt_stage("infer", &self.infer),
            fmt_stage("save", &self.save),
            fmt_edge(&self.read_to_decode),
            fmt_edge(&self.decode_to_infer),
            fmt_edge(&self.infer_to_save),
        )
    }
}

impl EdgeDiag {
    fn new(from_to: &'static str, capacity: usize) -> Self {
        Self {
            from_to,
            capacity,
            max_depth: AtomicUsize::new(0),
            max_wait_ms: AtomicU64::new(0),
            wait_over_threshold: AtomicUsize::new(0),
            last_log: Mutex::new(None),
        }
    }
}

impl Default for StageDiag {
    fn default() -> Self {
        Self {
            items: AtomicUsize::new(0),
            total_ms: AtomicU64::new(0),
        }
    }
}

async fn send_diagnosed<T>(
    tx: &mpsc::Sender<T>,
    value: T,
    diag: &PipelineDiag,
    edge: &EdgeDiag,
) -> Result<(), mpsc::error::SendError<T>> {
    let started = Instant::now();
    let remaining = tx.capacity();
    let result = tx.send(value).await;
    diag.note_send(edge, started.elapsed(), remaining);
    result
}

fn decode_job_with_decoder(
    decoder: &dyn ImageDecoder,
    path: &std::path::Path,
    bytes: &[u8],
    options: DecodeOptions,
) -> Result<DynamicImage, AiItemError> {
    decode_bytes_with_path(decoder, path, bytes, options).map_err(|error| AiItemError {
        code: error.code.to_string(),
        message: error.message,
    })
}

/// Run read → decode → batch infer → per-item save.
///
/// On cancel: stop reading new jobs, drain already-decoded/inferred work through save,
/// then return `Err("pipeline cancelled")`.
///
/// `save` receives the decoded image when decode succeeded (even if infer failed), so
/// domain pipelines can write side artifacts (e.g. face crops) without re-decoding.
pub async fn run_image_pipeline<M, Out, Infer, InferFut, Save, SaveFut>(
    jobs: Vec<ImageJob<M>>,
    config: PipelineConfig,
    decoder: Arc<dyn ImageDecoder>,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<PipelineProgressTx>,
    infer: Infer,
    save: Save,
) -> Result<(), String>
where
    M: Send + 'static,
    Out: Send + 'static,
    Infer: Fn(Vec<DynamicImage>) -> InferFut + Send + Sync + 'static,
    InferFut: Future<Output = Result<(Vec<DynamicImage>, Vec<Result<Out, AiItemError>>), String>>
        + Send
        + 'static,
    Save: Fn(M, Result<Out, AiItemError>, Option<DynamicImage>) -> SaveFut + Send + Sync + 'static,
    SaveFut: Future<Output = ()> + Send + 'static,
{
    let total = jobs.len();
    if total == 0 {
        return Ok(());
    }

    let diag = PipelineDiag::new(&config);
    let decode_parallelism = config.decode_parallelism.max(1).min(total);
    let gpu_batch_size = config.gpu_batch_size.max(1);
    let decode_cap = config.decode_queue_capacity();
    let infer_cap = config.infer_queue_capacity();
    let save_cap = config.save_queue_capacity();
    let decode_apply_exif = config.decode_apply_exif;

    let (decode_tx, mut decode_rx) = mpsc::channel::<ReadJob<M>>(decode_cap);
    let (infer_tx, mut infer_rx) = mpsc::channel::<InferJob<M>>(infer_cap);
    let (save_tx, mut save_rx) = mpsc::channel::<SaveJob<M, Out>>(save_cap);

    let cancel = cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));

    let diag_read = Arc::clone(&diag);
    let reader_cancel = Arc::clone(&cancel);
    let reader = tokio::spawn(async move {
        for job in jobs {
            if reader_cancel.load(Ordering::Relaxed) {
                break;
            }
            let started = Instant::now();
            let bytes = tokio::fs::read(&job.path)
                .await
                .map_err(|error| AiItemError {
                    code: "read_failed".to_string(),
                    message: error.to_string(),
                });
            PipelineDiag::record_stage(&diag_read.read, started.elapsed());
            if send_diagnosed(
                &decode_tx,
                ReadJob {
                    index: job.index,
                    path: job.path,
                    meta: job.meta,
                    bytes,
                },
                &diag_read,
                &diag_read.read_to_decode,
            )
            .await
            .is_err()
            {
                break;
            }
        }
    });

    let infer_tx_decode = infer_tx.clone();
    let diag_decode = Arc::clone(&diag);
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
            let diag = Arc::clone(&diag_decode);
            join_set.spawn(async move {
                let decode_options = if decode_apply_exif {
                    DecodeOptions::new()
                } else {
                    DecodeOptions::raw_pixels()
                };
                let started = Instant::now();
                let image = match job.bytes {
                    Ok(bytes) => {
                        let path = job.path.clone();
                        tokio::task::spawn_blocking(move || {
                            decode_job_with_decoder(decoder.as_ref(), &path, &bytes, decode_options)
                        })
                        .await
                        .unwrap_or_else(|error| {
                            Err(AiItemError {
                                code: "decode_failed".to_string(),
                                message: format!("decode join failed: {error}"),
                            })
                        })
                    }
                    Err(error) => Err(error),
                };
                PipelineDiag::record_stage(&diag.decode, started.elapsed());
                let _ = send_diagnosed(
                    &infer_tx,
                    InferJob {
                        index: job.index,
                        path: job.path,
                        meta: job.meta,
                        image,
                    },
                    &diag,
                    &diag.decode_to_infer,
                )
                .await;
            });
        }
        while join_set.join_next().await.is_some() {}
        drop(infer_tx_decode);
    });

    let infer = Arc::new(infer);
    let save = Arc::new(save);
    let diag_infer = Arc::clone(&diag);
    let save_tx_infer = save_tx.clone();
    let progress_infer = progress;
    let infer_worker = tokio::spawn(async move {
        loop {
            let first = match infer_rx.recv().await {
                Some(job) => job,
                None => break,
            };
            let mut batch = vec![first];
            while batch.len() < gpu_batch_size {
                match infer_rx.try_recv() {
                    Ok(job) => batch.push(job),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            // Split decode failures from images that need inference.
            let mut ready_images = Vec::new();
            let mut ready_meta = Vec::new();
            let mut passthrough = Vec::new();
            for job in batch {
                match job.image {
                    Ok(image) => {
                        ready_images.push(image);
                        ready_meta.push((job.index, job.path, job.meta));
                    }
                    Err(error) => passthrough.push(SaveJob {
                        index: job.index,
                        path: job.path,
                        meta: job.meta,
                        result: Err(error),
                        image: None,
                    }),
                }
            }

            let mut inferred = Vec::new();
            if !ready_images.is_empty() {
                let started = Instant::now();
                let batch_result = infer(ready_images).await;
                PipelineDiag::record_stage(&diag_infer.infer, started.elapsed());
                match batch_result {
                    Ok((images, results)) if results.len() == ready_meta.len() => {
                        for (((index, path, meta), result), image) in
                            ready_meta.into_iter().zip(results).zip(images.into_iter())
                        {
                            inferred.push(SaveJob {
                                index,
                                path,
                                meta,
                                result,
                                image: Some(image),
                            });
                        }
                    }
                    Ok((images, results)) => {
                        let message = format!(
                            "infer result count mismatch: got {} expected {}",
                            results.len(),
                            ready_meta.len()
                        );
                        // Prefer pairing leftover images with meta when lengths diverge.
                        let mut images = images.into_iter();
                        for (index, path, meta) in ready_meta {
                            inferred.push(SaveJob {
                                index,
                                path,
                                meta,
                                result: Err(AiItemError {
                                    code: "invalid_output".to_string(),
                                    message: message.clone(),
                                }),
                                image: images.next(),
                            });
                        }
                    }
                    Err(error) => {
                        for (index, path, meta) in ready_meta {
                            inferred.push(SaveJob {
                                index,
                                path,
                                meta,
                                result: Err(AiItemError {
                                    code: "provider_rejected".to_string(),
                                    message: error.clone(),
                                }),
                                image: None,
                            });
                        }
                    }
                }
            }

            for job in passthrough.into_iter().chain(inferred) {
                if let Some(tx) = progress_infer.as_ref() {
                    let _ = tx.send((job.index, job.path.clone())).await;
                }
                if send_diagnosed(&save_tx_infer, job, &diag_infer, &diag_infer.infer_to_save)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    });

    let diag_save = Arc::clone(&diag);
    let save_worker = tokio::spawn(async move {
        while let Some(job) = save_rx.recv().await {
            let started = Instant::now();
            save(job.meta, job.result, job.image).await;
            PipelineDiag::record_stage(&diag_save.save, started.elapsed());
        }
    });

    let _ = reader.await;
    let _ = decode_workers.await;
    drop(infer_tx);
    let _ = infer_worker.await;
    drop(save_tx);
    let _ = save_worker.await;

    eprintln!("{}", diag.summary_line());

    if cancel.load(Ordering::Relaxed) {
        return Err("pipeline cancelled".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::album::decoder::{DecoderCreateOptions, create_decoder};
    use image::{ImageBuffer, Rgb};
    use std::sync::Mutex;
    use uuid::Uuid;

    #[test]
    fn queue_capacities_are_at_least_one() {
        let config = PipelineConfig::new("test", 0, 0);
        assert_eq!(config.decode_queue_capacity(), 1);
        assert_eq!(config.infer_queue_capacity(), 1);
        assert_eq!(config.save_queue_capacity(), 1);
        assert_eq!(config.progress_queue_capacity(), 8);
    }

    #[tokio::test]
    async fn pipeline_infers_and_saves_each_item() {
        let dir = std::env::temp_dir().join(format!("album-pipe-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let path = dir.join(format!("{i}.png"));
            let img: ImageBuffer<Rgb<u8>, _> =
                ImageBuffer::from_pixel(4, 4, Rgb([i as u8, 10, 20]));
            img.save(&path).unwrap();
            paths.push(path);
        }
        let jobs = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| ImageJob {
                index,
                path,
                meta: index,
            })
            .collect::<Vec<_>>();
        let saved = Arc::new(Mutex::new(Vec::new()));
        let saved_c = Arc::clone(&saved);
        let decoder: Arc<dyn ImageDecoder> =
            Arc::from(create_decoder(DecoderCreateOptions::auto()).unwrap());
        let config = PipelineConfig::new("test", 2, 2);
        run_image_pipeline(
            jobs,
            config,
            decoder,
            None,
            None,
            |images| async move {
                let widths = images
                    .iter()
                    .map(|img| Ok(img.width() as usize))
                    .collect::<Vec<_>>();
                Ok((images, widths))
            },
            move |meta, result, _image| {
                let saved_c = Arc::clone(&saved_c);
                async move {
                    saved_c.lock().unwrap().push((meta, result.unwrap()));
                }
            },
        )
        .await
        .unwrap();
        let mut got = saved.lock().unwrap().clone();
        got.sort_by_key(|(meta, _)| *meta);
        assert_eq!(got, vec![(0, 4), (1, 4), (2, 4)]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
