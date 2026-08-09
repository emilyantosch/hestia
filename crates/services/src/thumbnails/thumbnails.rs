use crate::thumbnails::generator::ThumbnailGenerator;
use anyhow::{Context, Result};
use model::services::file::FileSystemFile as File;
use model::services::thumbnail::ThumbnailSize;
use repositories::thumbnail::operations::ThumbnailOperations;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ThumbnailJobStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub(crate) struct ThumbnailJob {
    pub file_id: i32,
    pub file_path: PathBuf,
    pub size: ThumbnailSize,
    pub status: ThumbnailJobStatus,
    pub created_at: Instant,
    pub retry_count: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessingStats {
    pub pending_jobs: usize,
    pub processing_jobs: usize,
    pub completed_jobs: u64,
    pub failed_jobs: u64,
    pub active_workers: usize,
    pub throughput_per_second: f64,
    pub avg_processing_time: Duration,
}

#[derive(Debug)]
pub enum ThumbnailMessage {
    QueueFiles {
        file_infos: Vec<File>,
        sizes: Vec<ThumbnailSize>,
    },
    QueueSingleFile {
        file_id: i32,
        file_path: PathBuf,
        size: ThumbnailSize,
    },
    QueueMissingFiles,
    GetStats {
        respond_to: oneshot::Sender<ProcessingStats>,
    },
    GetPendingCount {
        respond_to: oneshot::Sender<usize>,
    },
    Shutdown,
}

#[derive(Debug)]
pub struct ThumbnailProcessorHandler {
    pub sender: mpsc::UnboundedSender<ThumbnailMessage>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessorConfig {
    pub worker_count: usize,
    pub batch_size: usize,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub processing_timeout: Duration,
    pub memory_limit_mb: usize,
}

impl Default for ProcessorConfig {
    fn default() -> Self {
        Self {
            worker_count: num_cpus::get().max(2),
            batch_size: 50,
            max_retries: 3,
            retry_delay: Duration::from_secs(5),
            processing_timeout: Duration::from_secs(30),
            memory_limit_mb: 100,
        }
    }
}

impl Default for ProcessingStats {
    fn default() -> Self {
        Self {
            pending_jobs: 0,
            processing_jobs: 0,
            completed_jobs: 0,
            failed_jobs: 0,
            active_workers: 0,
            throughput_per_second: 0.0,
            avg_processing_time: Duration::from_millis(0),
        }
    }
}

// Worker struct responsible for actual thumbnail generation
pub struct ThumbnailWorker {
    worker_id: usize,
    job_queue: Arc<Mutex<Vec<ThumbnailJob>>>,
    repository: Arc<ThumbnailOperations>,
    generator: Arc<ThumbnailGenerator>,
    stats: Arc<Mutex<ProcessingStats>>,
    config: ProcessorConfig,
}

impl ThumbnailWorker {
    pub fn new(
        worker_id: usize,
        job_queue: Arc<Mutex<Vec<ThumbnailJob>>>,
        repository: Arc<ThumbnailOperations>,
        generator: Arc<ThumbnailGenerator>,
        stats: Arc<Mutex<ProcessingStats>>,
        config: ProcessorConfig,
    ) -> Self {
        Self {
            worker_id,
            job_queue,
            repository,
            generator,
            stats,
            config,
        }
    }

    pub async fn run(self, shutdown_signal: Arc<tokio::sync::Notify>) {
        tracing::info!("Worker {} started", self.worker_id);

        loop {
            tokio::select! {
                _ = shutdown_signal.notified() => {
                    tracing::info!("Worker {} received shutdown signal", self.worker_id);
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if let Some(job) = self.get_next_job().await {
                        if let Err(e) = self.process_job(job).await {
                            tracing::error!("Worker {} failed to process job: {}", self.worker_id, e);
                        }
                    }
                }
            }
        }

        tracing::info!("Worker {} stopped", self.worker_id);
    }

    async fn get_next_job(&self) -> Option<ThumbnailJob> {
        let mut queue = self.job_queue.lock().await;

        // Find first pending job
        for (index, job) in queue.iter().enumerate() {
            if job.status == ThumbnailJobStatus::Pending {
                let mut job = queue.remove(index);
                job.status = ThumbnailJobStatus::Processing;
                return Some(job);
            }
        }

        None
    }

    async fn process_job(&self, job: ThumbnailJob) -> anyhow::Result<()> {
        let start_time = Instant::now();

        tracing::debug!(
            "Worker {} processing thumbnail job for file {} size {:?}",
            self.worker_id,
            job.file_id,
            job.size
        );

        // Generate thumbnail with timeout
        let generation_result = timeout(
            self.config.processing_timeout,
            self.generator
                .generate_from_file_path(&job.file_path, job.size),
        )
        .await;

        match generation_result {
            Ok(Ok(thumbnail)) => {
                // Save thumbnail to database
                match self
                    .repository
                    .create_thumbnail(job.file_id, thumbnail)
                    .await
                {
                    Ok(_) => {
                        let processing_time = start_time.elapsed();
                        tracing::info!(
                            "Worker {} successfully generated thumbnail for file {} size {:?} in {:?}",
                            self.worker_id,
                            job.file_id,
                            job.size,
                            processing_time
                        );

                        // Update stats
                        let mut stats = self.stats.lock().await;
                        stats.completed_jobs += 1;
                        self.update_avg_processing_time(&mut stats, processing_time);
                    }
                    Err(e) => {
                        tracing::error!(
                            "Worker {} failed to save thumbnail to database: {}",
                            self.worker_id,
                            e
                        );
                        self.handle_failed_job(job).await;
                        return Err(e);
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "Worker {} failed to generate thumbnail for file {}: {}",
                    self.worker_id,
                    job.file_id,
                    e
                );
                self.handle_failed_job(job).await;
                return Err(e);
            }
            Err(_) => {
                tracing::error!(
                    "Worker {} thumbnail generation timed out for file {}",
                    self.worker_id,
                    job.file_id
                );
                self.handle_failed_job(job).await;
                anyhow::bail!("worker {} thumbnail generation timed out", self.worker_id);
            }
        }

        Ok(())
    }

    async fn handle_failed_job(&self, mut job: ThumbnailJob) {
        job.retry_count += 1;

        if job.retry_count < self.config.max_retries {
            // Retry the job after delay
            job.status = ThumbnailJobStatus::Pending;

            let delay = self.config.retry_delay * job.retry_count; // Exponential backoff
            tracing::info!(
                "Worker {} retrying failed job (attempt {}/{})",
                self.worker_id,
                job.retry_count,
                self.config.max_retries
            );

            tokio::spawn({
                let job_queue = Arc::clone(&self.job_queue);
                async move {
                    tokio::time::sleep(delay).await;
                    let mut queue = job_queue.lock().await;
                    queue.push(job);
                }
            });
        } else {
            // Move to failed jobs
            job.status = ThumbnailJobStatus::Failed;

            let mut stats = self.stats.lock().await;
            stats.failed_jobs += 1;

            tracing::warn!(
                "Worker {} job permanently failed after {} retries: file {} size {:?}",
                self.worker_id,
                self.config.max_retries,
                job.file_id,
                job.size
            );
        }
    }

    fn update_avg_processing_time(&self, stats: &mut ProcessingStats, processing_time: Duration) {
        // Simple moving average calculation
        let current_avg_ms = stats.avg_processing_time.as_millis() as f64;
        let new_time_ms = processing_time.as_millis() as f64;
        let total_jobs = stats.completed_jobs as f64;

        let new_avg_ms = if total_jobs <= 1.0 {
            new_time_ms
        } else {
            (current_avg_ms * (total_jobs - 1.0) + new_time_ms) / total_jobs
        };

        stats.avg_processing_time = Duration::from_millis(new_avg_ms as u64);
    }
}

// Processor struct responsible for message handling and coordination
pub struct ThumbnailProcessor {
    message_receiver: mpsc::UnboundedReceiver<ThumbnailMessage>,
    job_queue: Arc<Mutex<Vec<ThumbnailJob>>>,
    repository: Arc<ThumbnailOperations>,
    generator: Arc<ThumbnailGenerator>,
    stats: Arc<Mutex<ProcessingStats>>,
    config: ProcessorConfig,
    shutdown_signal: Arc<tokio::sync::Notify>,
}

impl ThumbnailProcessor {
    pub fn new(
        message_receiver: mpsc::UnboundedReceiver<ThumbnailMessage>,
        repository: Arc<ThumbnailOperations>,
        generator: Arc<ThumbnailGenerator>,
    ) -> Self {
        Self {
            message_receiver,
            job_queue: Arc::new(Mutex::new(Vec::new())),
            repository,
            generator,
            stats: Arc::new(Mutex::new(ProcessingStats::default())),
            config: ProcessorConfig::default(),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn with_config(mut self, config: ProcessorConfig) -> Self {
        self.config = config;
        self
    }

    pub async fn run(mut self) -> Result<()> {
        tracing::info!(
            "Starting ThumbnailProcessor with {} workers",
            self.config.worker_count
        );

        // Start worker tasks
        let mut worker_handles = Vec::new();
        for worker_id in 0..self.config.worker_count {
            let worker = ThumbnailWorker::new(
                worker_id,
                Arc::clone(&self.job_queue),
                Arc::clone(&self.repository),
                Arc::clone(&self.generator),
                Arc::clone(&self.stats),
                self.config.clone(),
            );

            let shutdown_signal = Arc::clone(&self.shutdown_signal);
            let handle = tokio::spawn(async move {
                worker.run(shutdown_signal).await;
            });
            worker_handles.push(handle);
        }

        // Start stats updater task
        let stats_handle = self.spawn_stats_updater().await;

        // Main message processing loop
        while let Some(message) = self.message_receiver.recv().await {
            match message {
                ThumbnailMessage::QueueFiles { file_infos, sizes } => {
                    self.queue_files_for_processing(file_infos, sizes).await?;
                }
                ThumbnailMessage::QueueSingleFile {
                    file_id,
                    file_path,
                    size,
                } => {
                    self.queue_single_file(file_id, file_path, size).await?;
                }
                ThumbnailMessage::QueueMissingFiles => {
                    self.queue_missing_files().await?;
                }
                ThumbnailMessage::GetStats { respond_to } => {
                    let stats = self.get_current_stats().await;
                    let _ = respond_to.send(stats);
                }
                ThumbnailMessage::GetPendingCount { respond_to } => {
                    let count = self.get_pending_job_count().await;
                    let _ = respond_to.send(count);
                }
                ThumbnailMessage::Shutdown => {
                    tracing::info!("Shutdown signal received, stopping processor");
                    break;
                }
            }
        }

        // Signal shutdown to all workers
        self.shutdown_signal.notify_waiters();

        // Wait for all workers to complete
        for handle in worker_handles {
            if let Err(e) = handle.await {
                tracing::error!("Worker task failed: {}", e);
            }
        }

        // Stop stats updater
        if let Err(e) = stats_handle.await {
            tracing::error!("Stats updater task failed: {}", e);
        }

        tracing::info!("ThumbnailProcessor stopped successfully");
        Ok(())
    }

    async fn queue_files_for_processing(
        &mut self,
        file_infos: Vec<File>,
        sizes: Vec<ThumbnailSize>,
    ) -> Result<usize> {
        let mut queue = self.job_queue.lock().await;
        let mut queued_count = 0;

        for file_info in file_infos {
            for &size in &sizes {
                let file_id = file_info
                    .id
                    .context("file ID for thumbnail generation was not provided")?;
                // Check if thumbnail already exists
                match self.repository.get_by_file_and_size(file_id, size).await {
                    Ok(Some(_)) => {
                        tracing::debug!(
                            "Thumbnail already exists for file {} size {:?}",
                            file_id,
                            size
                        );
                        continue;
                    }
                    Ok(None) => {
                        // Need to generate thumbnail
                        let job = ThumbnailJob {
                            file_id: file_id,
                            file_path: file_info.path.clone(),
                            size,
                            status: ThumbnailJobStatus::Pending,
                            created_at: Instant::now(),
                            retry_count: 0,
                        };
                        queue.push(job);
                        queued_count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to check existing thumbnail for file {}: {}",
                            file_id,
                            e
                        );
                        // Queue anyway to be safe
                        let job = ThumbnailJob {
                            file_id: file_id,
                            file_path: file_info.path.clone(),
                            size,
                            status: ThumbnailJobStatus::Pending,
                            created_at: Instant::now(),
                            retry_count: 0,
                        };
                        queue.push(job);
                        queued_count += 1;
                    }
                }
            }
        }

        tracing::info!("Queued {} thumbnail generation jobs", queued_count);
        Ok(queued_count)
    }

    async fn queue_missing_files(&mut self) -> Result<usize> {
        let mut queued_jobs = 0;
        let all_thumbnail_sizes = ThumbnailSize::all().to_vec();
        let file_models = self
            .repository
            .get_files_without_thumbnails_sizes(all_thumbnail_sizes, None)
            .await?;
        let files: Vec<File> = file_models.into_iter().map(|v| v.into()).collect();

        self.queue_files_for_processing(files, ThumbnailSize::all().to_vec())
            .await
    }

    async fn queue_single_file(
        &mut self,
        file_id: i32,
        file_path: PathBuf,
        size: ThumbnailSize,
    ) -> Result<()> {
        // Check if thumbnail already exists
        match self.repository.get_by_file_and_size(file_id, size).await {
            Ok(Some(_)) => {
                tracing::debug!(
                    "Thumbnail already exists for file {} size {:?}",
                    file_id,
                    size
                );
                return Ok(());
            }
            Ok(None) => {
                // Need to generate thumbnail
                let job = ThumbnailJob {
                    file_id,
                    file_path,
                    size,
                    status: ThumbnailJobStatus::Pending,
                    created_at: Instant::now(),
                    retry_count: 0,
                };

                let mut queue = self.job_queue.lock().await;
                queue.push(job);
                tracing::info!(
                    "Queued single thumbnail job for file {} size {:?}",
                    file_id,
                    size
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to check existing thumbnail for file {}: {}",
                    file_id,
                    e
                );
                // Queue anyway to be safe
                let job = ThumbnailJob {
                    file_id,
                    file_path,
                    size,
                    status: ThumbnailJobStatus::Pending,
                    created_at: Instant::now(),
                    retry_count: 0,
                };

                let mut queue = self.job_queue.lock().await;
                queue.push(job);
            }
        }

        Ok(())
    }

    async fn spawn_stats_updater(&self) -> tokio::task::JoinHandle<()> {
        let stats = Arc::clone(&self.stats);
        let shutdown_signal = Arc::clone(&self.shutdown_signal);

        tokio::spawn(async move {
            let mut last_completed = 0u64;
            let mut last_update = Instant::now();

            loop {
                tokio::select! {
                    _ = shutdown_signal.notified() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        let mut stats_guard = stats.lock().await;
                        let now = Instant::now();
                        let elapsed = now.duration_since(last_update).as_secs_f64();

                        if elapsed > 0.0 {
                            let completed_delta = stats_guard.completed_jobs - last_completed;
                            stats_guard.throughput_per_second = completed_delta as f64 / elapsed;
                        }

                        last_completed = stats_guard.completed_jobs;
                        last_update = now;
                    }
                }
            }
        })
    }

    async fn get_current_stats(&self) -> ProcessingStats {
        let stats = self.stats.lock().await;
        let queue = self.job_queue.lock().await;

        let pending_jobs = queue
            .iter()
            .filter(|job| job.status == ThumbnailJobStatus::Pending)
            .count();
        let processing_jobs = queue
            .iter()
            .filter(|job| job.status == ThumbnailJobStatus::Processing)
            .count();

        ProcessingStats {
            pending_jobs,
            processing_jobs,
            completed_jobs: stats.completed_jobs,
            failed_jobs: stats.failed_jobs,
            active_workers: self.config.worker_count,
            throughput_per_second: stats.throughput_per_second,
            avg_processing_time: stats.avg_processing_time,
        }
    }

    async fn get_pending_job_count(&self) -> usize {
        let queue = self.job_queue.lock().await;
        queue
            .iter()
            .filter(|job| job.status == ThumbnailJobStatus::Pending)
            .count()
    }
}

impl ThumbnailProcessorHandler {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<ThumbnailMessage>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self { sender }, receiver)
    }

    pub async fn queue_files(
        &self,
        file_infos: Vec<File>,
        sizes: Vec<ThumbnailSize>,
    ) -> Result<()> {
        self.sender
            .send(ThumbnailMessage::QueueFiles { file_infos, sizes })?;
        Ok(())
    }

    pub async fn queue_missing_files(&self) -> Result<()> {
        match self.sender.send(ThumbnailMessage::QueueMissingFiles) {
            Ok(()) => {
                tracing::info!("Thumbnail tasked with creating missing thumbnails");
            }
            Err(e) => {
                tracing::error!(
                    "Sending message to creating missing thumbnails failed due to: {e}"
                );
                return Err(e)?;
            }
        }
        Ok(())
    }

    pub async fn queue_single_file(
        &self,
        file_id: i32,
        file_path: PathBuf,
        size: ThumbnailSize,
    ) -> Result<()> {
        self.sender.send(ThumbnailMessage::QueueSingleFile {
            file_id,
            file_path,
            size,
        })?;
        Ok(())
    }

    pub async fn get_stats(&self) -> Result<ProcessingStats> {
        let (respond_to, response) = oneshot::channel();

        self.sender
            .send(ThumbnailMessage::GetStats { respond_to })?;

        Ok(response.await?)
    }

    pub async fn get_pending_count(&self) -> Result<usize> {
        let (respond_to, response) = oneshot::channel();

        self.sender
            .send(ThumbnailMessage::GetPendingCount { respond_to })?;

        Ok(response.await?)
    }

    pub async fn shutdown(&self) -> Result<()> {
        Ok(self.sender.send(ThumbnailMessage::Shutdown)?)
    }
}
