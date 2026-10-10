use crate::thumbnails::generator::ThumbnailGenerator;
use anyhow::{Context, Result};
use itertools::Itertools;
use model::services::thumbnail::ThumbnailSize;
use repositories::thumbnail::operations::ThumbnailOperations;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinSet;
use tokio::time::timeout;

#[derive(Debug)]
struct ThumbnailJob {
    file_id: i32,
    file_path: PathBuf,
    size: ThumbnailSize,
    attempts: u32,
}

type JobReceiver = Arc<Mutex<mpsc::UnboundedReceiver<ThumbnailJob>>>;

#[derive(Clone, Copy, Debug)]
pub enum ThumbnailMessage {
    QueueMissingFiles,
}

#[derive(Debug)]
pub struct ThumbnailProcessorHandler {
    pub sender: mpsc::UnboundedSender<ThumbnailMessage>,
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessorConfig {
    pub worker_count: usize,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub processing_timeout: Duration,
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            worker_count: num_cpus::get().max(2),
            max_retries: 3,
            retry_delay: Duration::from_secs(5),
            processing_timeout: Duration::from_secs(30),
        }
    }
}

// Worker struct responsible for actual thumbnail generation
struct ThumbnailWorker {
    worker_id: usize,
    jobs: mpsc::UnboundedSender<ThumbnailJob>,
    repository: Arc<ThumbnailOperations>,
    generator: Arc<ThumbnailGenerator>,
    config: ProcessorConfig,
}

impl ThumbnailWorker {
    async fn run(self, receiver: JobReceiver) {
        tracing::info!("Worker {} started", self.worker_id);
        loop {
            let job = receiver.lock().await.recv().await;
            let Some(job) = job else { break };
            let start_time = Instant::now();
            match self.process_job(&job).await {
                Ok(()) => tracing::info!(
                    "Worker {} generated thumbnail for file {} size {:?} in {:?}",
                    self.worker_id,
                    job.file_id,
                    job.size,
                    start_time.elapsed()
                ),
                Err(error) => {
                    tracing::warn!(
                        "Worker {} failed to generate thumbnail for file {}: {error:#}",
                        self.worker_id,
                        job.file_id
                    );
                    self.retry(job);
                }
            }
        }
        tracing::info!("Worker {} stopped", self.worker_id);
    }

    async fn process_job(&self, job: &ThumbnailJob) -> Result<()> {
        let thumbnail = timeout(
            self.config.processing_timeout,
            self.generator
                .generate_from_file_path(&job.file_path, job.size),
        )
        .await
        .context("thumbnail generation timed out")??;
        self.repository
            .create_thumbnail(job.file_id, thumbnail)
            .await?;
        Ok(())
    }

    /// Requeue a failed job after a growing delay, until it runs out of attempts.
    fn retry(&self, mut job: ThumbnailJob) {
        job.attempts += 1;
        if job.attempts >= self.config.max_retries {
            tracing::warn!(
                "Worker {} job permanently failed after {} attempts: file {} size {:?}",
                self.worker_id,
                job.attempts,
                job.file_id,
                job.size
            );
            return;
        }

        let delay = self.config.retry_delay * job.attempts;
        let jobs = self.jobs.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // The processor has stopped if the queue is closed; drop the job then.
            drop(jobs.send(job));
        });
    }
}

// Processor struct responsible for message handling and coordination
#[derive(Debug)]
pub struct ThumbnailProcessor {
    message_receiver: mpsc::UnboundedReceiver<ThumbnailMessage>,
    repository: Arc<ThumbnailOperations>,
    generator: Arc<ThumbnailGenerator>,
    config: ProcessorConfig,
}

impl ThumbnailProcessor {
    #[must_use]
    pub fn new(
        message_receiver: mpsc::UnboundedReceiver<ThumbnailMessage>,
        repository: Arc<ThumbnailOperations>,
        generator: Arc<ThumbnailGenerator>,
    ) -> Self {
        Self {
            message_receiver,
            repository,
            generator,
            config: ProcessorConfig::default(),
        }
    }

    #[must_use]
    pub fn with_config(mut self, config: ProcessorConfig) -> Self {
        self.config = config;
        self
    }

    /// Process messages until every handler is dropped, then stop the workers.
    pub async fn run(mut self) -> Result<()> {
        tracing::info!(
            "Starting ThumbnailProcessor with {} workers",
            self.config.worker_count
        );

        let (jobs, receiver) = mpsc::unbounded_channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = JoinSet::new();
        for worker_id in 0..self.config.worker_count {
            let worker = ThumbnailWorker {
                worker_id,
                jobs: jobs.clone(),
                repository: Arc::clone(&self.repository),
                generator: Arc::clone(&self.generator),
                config: self.config,
            };
            workers.spawn(worker.run(Arc::clone(&receiver)));
        }

        let result = async {
            while let Some(message) = self.message_receiver.recv().await {
                match message {
                    ThumbnailMessage::QueueMissingFiles => self.queue_missing_files(&jobs).await?,
                }
            }
            Ok(())
        }
        .await;

        // Workers hold senders for retries, so the queue never closes on its own.
        workers.shutdown().await;
        tracing::info!("ThumbnailProcessor stopped");
        result
    }

    async fn queue_missing_files(&self, jobs: &mpsc::UnboundedSender<ThumbnailJob>) -> Result<()> {
        let sizes = ThumbnailSize::all();
        let files = self
            .repository
            .get_files_without_thumbnails_sizes(sizes.to_vec(), None)
            .await?;
        for (file, size) in files.iter().cartesian_product(sizes) {
            jobs.send(ThumbnailJob {
                file_id: file.id,
                file_path: PathBuf::from(&file.path),
                size,
                attempts: 0,
            })?;
        }
        tracing::info!(
            "Queued {} thumbnail generation jobs",
            files.len() * sizes.len()
        );
        Ok(())
    }
}

impl ThumbnailProcessorHandler {
    #[must_use]
    pub fn new() -> (Self, mpsc::UnboundedReceiver<ThumbnailMessage>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self { sender }, receiver)
    }

    pub fn queue_missing_files(&self) -> Result<()> {
        self.sender
            .send(ThumbnailMessage::QueueMissingFiles)
            .inspect_err(|error| {
                tracing::error!(
                    "Sending message to creating missing thumbnails failed due to: {error}"
                );
            })?;
        tracing::info!("Thumbnail tasked with creating missing thumbnails");
        Ok(())
    }
}
