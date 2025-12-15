use crate::{
    health::write_health_snapshot, IndexStats, IndexerError, ModelIndexSpec,
    MultiModelProjectIndexer, ProjectIndexer, Result,
};
use log::{error, info, warn};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{broadcast, mpsc, watch, Mutex as TokioMutex};
use tokio::time;

const DEFAULT_ALERT_REASON: &str = "fs_event";

#[derive(Debug, Clone)]
pub struct IndexUpdate {
    pub completed_at: SystemTime,
    pub duration_ms: u64,
    pub stats: Option<IndexStats>,
    pub success: bool,
    pub reason: String,
    pub store_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexerHealth {
    pub last_success: Option<SystemTime>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub last_duration_ms: Option<u64>,
    pub pending_events: usize,
    pub indexing: bool,
    pub last_throughput_files_per_sec: Option<f32>,
    pub p95_duration_ms: Option<u64>,
    pub last_index_size_bytes: Option<u64>,
    pub alert_log_json: String,
    pub alert_log_len: usize,
}

impl IndexerHealth {
    fn initial() -> Self {
        Self {
            last_success: None,
            last_error: None,
            consecutive_failures: 0,
            last_duration_ms: None,
            pending_events: 0,
            indexing: false,
            last_throughput_files_per_sec: None,
            p95_duration_ms: None,
            last_index_size_bytes: None,
            alert_log_json: String::from("[]"),
            alert_log_len: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamingIndexerConfig {
    pub debounce: Duration,
    pub max_batch_wait: Duration,
    pub notify_poll_interval: Duration,
}

impl Default for StreamingIndexerConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(750),
            max_batch_wait: Duration::from_secs(3),
            notify_poll_interval: Duration::from_secs(2),
        }
    }
}

#[derive(Clone)]
pub struct StreamingIndexer {
    inner: Arc<StreamingIndexerInner>,
}

struct StreamingIndexerInner {
    command_tx: mpsc::Sender<WatcherCommand>,
    update_tx: broadcast::Sender<IndexUpdate>,
    health_tx: watch::Sender<IndexerHealth>,
    _watcher: Arc<std::sync::Mutex<Option<RecommendedWatcher>>>,
    _health_guard: TokioMutex<watch::Receiver<IndexerHealth>>,
}

enum WatcherCommand {
    Trigger { reason: String },
    Shutdown,
}

impl StreamingIndexer {
    pub fn start(indexer: Arc<ProjectIndexer>, config: StreamingIndexerConfig) -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel(1024);
        let (command_tx, command_rx) = mpsc::channel(16);
        let (health_tx, health_rx) = watch::channel(IndexerHealth::initial());
        let (update_tx, _) = broadcast::channel(32);

        let watcher = create_fs_watcher(indexer.root(), event_tx, config.notify_poll_interval)?;
        let watcher = Arc::new(std::sync::Mutex::new(Some(watcher)));

        spawn_index_loop(
            indexer,
            config.clone(),
            event_rx,
            command_rx,
            update_tx.clone(),
            health_tx.clone(),
        );

        Ok(Self {
            inner: Arc::new(StreamingIndexerInner {
                command_tx,
                update_tx,
                health_tx,
                _watcher: watcher,
                _health_guard: TokioMutex::new(health_rx),
            }),
        })
    }

    pub async fn trigger(&self, reason: impl Into<String>) -> Result<()> {
        self.inner
            .command_tx
            .send(WatcherCommand::Trigger {
                reason: reason.into(),
            })
            .await
            .map_err(|e| IndexerError::Other(format!("failed to send trigger: {e}")))?;
        Ok(())
    }

    #[must_use]
    pub fn subscribe_updates(&self) -> broadcast::Receiver<IndexUpdate> {
        self.inner.update_tx.subscribe()
    }

    #[must_use]
    pub fn health_snapshot(&self) -> IndexerHealth {
        self.inner.health_tx.subscribe().borrow().clone()
    }

    #[must_use]
    pub fn health_stream(&self) -> watch::Receiver<IndexerHealth> {
        self.inner.health_tx.subscribe()
    }
}

impl Drop for StreamingIndexer {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            let _ = self.inner.command_tx.try_send(WatcherCommand::Shutdown);
        }
    }
}

#[derive(Clone)]
pub struct MultiModelStreamingIndexer {
    inner: Arc<MultiModelStreamingIndexerInner>,
}

struct MultiModelStreamingIndexerInner {
    command_tx: mpsc::Sender<WatcherCommand>,
    update_tx: broadcast::Sender<IndexUpdate>,
    health_tx: watch::Sender<IndexerHealth>,
    _watcher: Arc<std::sync::Mutex<Option<RecommendedWatcher>>>,
    _health_guard: TokioMutex<watch::Receiver<IndexerHealth>>,
    models: Arc<TokioMutex<Vec<ModelIndexSpec>>>,
}

impl MultiModelStreamingIndexer {
    pub fn start(
        indexer: Arc<MultiModelProjectIndexer>,
        models: Vec<ModelIndexSpec>,
        config: StreamingIndexerConfig,
    ) -> Result<Self> {
        if models.is_empty() {
            return Err(IndexerError::Other(
                "MultiModelStreamingIndexer requires at least one model".to_string(),
            ));
        }

        let (event_tx, event_rx) = mpsc::channel(1024);
        let (command_tx, command_rx) = mpsc::channel(16);
        let (health_tx, health_rx) = watch::channel(IndexerHealth::initial());
        let (update_tx, _) = broadcast::channel(32);

        let watcher = create_fs_watcher(indexer.root(), event_tx, config.notify_poll_interval)?;
        let watcher = Arc::new(std::sync::Mutex::new(Some(watcher)));

        let models = Arc::new(TokioMutex::new(models));

        spawn_multi_model_index_loop(
            indexer,
            config.clone(),
            event_rx,
            command_rx,
            update_tx.clone(),
            health_tx.clone(),
            models.clone(),
        );

        Ok(Self {
            inner: Arc::new(MultiModelStreamingIndexerInner {
                command_tx,
                update_tx,
                health_tx,
                _watcher: watcher,
                _health_guard: TokioMutex::new(health_rx),
                models,
            }),
        })
    }

    pub async fn trigger(&self, reason: impl Into<String>) -> Result<()> {
        self.inner
            .command_tx
            .send(WatcherCommand::Trigger {
                reason: reason.into(),
            })
            .await
            .map_err(|e| IndexerError::Other(format!("failed to send trigger: {e}")))?;
        Ok(())
    }

    pub async fn set_models(&self, models: Vec<ModelIndexSpec>) -> Result<()> {
        if models.is_empty() {
            return Err(IndexerError::Other(
                "MultiModelStreamingIndexer models must not be empty".to_string(),
            ));
        }
        let mut guard = self.inner.models.lock().await;
        *guard = models;
        Ok(())
    }

    #[must_use]
    pub fn subscribe_updates(&self) -> broadcast::Receiver<IndexUpdate> {
        self.inner.update_tx.subscribe()
    }

    #[must_use]
    pub fn health_snapshot(&self) -> IndexerHealth {
        self.inner.health_tx.subscribe().borrow().clone()
    }

    #[must_use]
    pub fn health_stream(&self) -> watch::Receiver<IndexerHealth> {
        self.inner.health_tx.subscribe()
    }
}

impl Drop for MultiModelStreamingIndexer {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            let _ = self.inner.command_tx.try_send(WatcherCommand::Shutdown);
        }
    }
}

fn create_fs_watcher(
    root: &Path,
    sender: mpsc::Sender<notify::Result<Event>>,
    poll_interval: Duration,
) -> Result<RecommendedWatcher> {
    let root = root.to_path_buf();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = sender.blocking_send(res);
        },
        NotifyConfig::default().with_poll_interval(poll_interval),
    )
    .map_err(|e| IndexerError::Other(format!("watcher init failed: {e}")))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| IndexerError::Other(format!("failed to watch {}: {e}", root.display())))?;
    Ok(watcher)
}

fn spawn_index_loop(
    indexer: Arc<ProjectIndexer>,
    config: StreamingIndexerConfig,
    mut event_rx: mpsc::Receiver<notify::Result<Event>>,
    mut command_rx: mpsc::Receiver<WatcherCommand>,
    update_tx: broadcast::Sender<IndexUpdate>,
    health_tx: watch::Sender<IndexerHealth>,
) {
    tokio::spawn(async move {
        let mut state = DebounceState::new(config.debounce, config.max_batch_wait);
        let mut health = IndexerHealth::initial();
        let mut duration_history: VecDeque<u64> = VecDeque::new();
        let mut alert_log: VecDeque<AlertRecord> = VecDeque::new();

        loop {
            let next_deadline = state.next_deadline();

            tokio::select! {
                Some(event) = event_rx.recv() => {
                    if handle_event(indexer.root(), event, &mut state) {
                        health.pending_events = state.pending();
                        let _ = health_tx.send(health.clone());
                    }
                }
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        WatcherCommand::Trigger { reason } => {
                            state.force_run(reason);
                            health.pending_events = state.pending();
                            let _ = health_tx.send(health.clone());
                        }
                        WatcherCommand::Shutdown => break,
                    }
                }
                () = async {
                    if let Some(deadline) = next_deadline {
                        time::sleep_until(deadline).await;
                    }
                }, if state.should_run() && next_deadline.is_some() => {
                    health.indexing = true;
                    let _ = health_tx.send(health.clone());

                    match run_index_cycle(indexer.clone(), state.take_reason().unwrap_or_else(|| DEFAULT_ALERT_REASON.to_string())).await {
                        Ok((cycle_stats, duration, reason, store_size)) => {
                            health.last_success = Some(SystemTime::now());
                            health.last_duration_ms = Some(duration);
                            health.last_error = None;
                            health.consecutive_failures = 0;
                            health.indexing = false;
                            health.pending_events = 0;
                            if duration > 0 {
                                let files_per_sec =
                                    cycle_stats.files as f32 / (duration as f32 / 1000.0);
                                health.last_throughput_files_per_sec = Some(files_per_sec);
                            }
                            health.last_index_size_bytes = store_size;
                            record_duration(&mut duration_history, duration);
                            health.p95_duration_ms = compute_p95(&duration_history);
                            health.alert_log_json = serialize_alerts(&alert_log);
                            health.alert_log_len = alert_log.len();
                            if let Err(err) = write_health_snapshot(
                                indexer.root(),
                                &cycle_stats,
                                &reason,
                                health.p95_duration_ms,
                                Some(health.pending_events),
                            )
                            .await
                            {
                                warn!("Failed to persist health snapshot after watcher index: {err}");
                            }
                            let _ = health_tx.send(health.clone());
                            let _ = update_tx.send(IndexUpdate {
                                completed_at: SystemTime::now(),
                                duration_ms: duration,
                                stats: Some(cycle_stats.clone()),
                                success: true,
                                reason,
                                store_size_bytes: store_size,
                            });
                        }
                        Err((err, duration, reason)) => {
                            error!("Streaming index failure: {err}");
                            health.last_error = Some(err.clone());
                            health.consecutive_failures += 1;
                            health.last_duration_ms = Some(duration);
                            health.indexing = false;
                            health.pending_events = 0;
                            if let Err(e) = crate::append_failure_reason(
                                indexer.root(),
                                &reason,
                                &err,
                                health.p95_duration_ms,
                            )
                            .await
                            {
                                warn!("Failed to persist failure reason: {e}");
                            }
                            push_alert(&mut alert_log, "error", &reason, &err);
                            health.alert_log_json = serialize_alerts(&alert_log);
                            health.alert_log_len = alert_log.len();
                            let _ = health_tx.send(health.clone());
                            let _ = update_tx.send(IndexUpdate {
                                completed_at: SystemTime::now(),
                                duration_ms: duration,
                                stats: None,
                                success: false,
                                reason,
                                store_size_bytes: None,
                            });
                        }
                    }

                    state.reset();
                }
            }
        }
    });
}

fn spawn_multi_model_index_loop(
    indexer: Arc<MultiModelProjectIndexer>,
    config: StreamingIndexerConfig,
    mut event_rx: mpsc::Receiver<notify::Result<Event>>,
    mut command_rx: mpsc::Receiver<WatcherCommand>,
    update_tx: broadcast::Sender<IndexUpdate>,
    health_tx: watch::Sender<IndexerHealth>,
    models: Arc<TokioMutex<Vec<ModelIndexSpec>>>,
) {
    tokio::spawn(async move {
        let mut state = DebounceState::new(config.debounce, config.max_batch_wait);
        let mut health = IndexerHealth::initial();
        let mut duration_history: VecDeque<u64> = VecDeque::new();
        let mut alert_log: VecDeque<AlertRecord> = VecDeque::new();

        loop {
            let next_deadline = state.next_deadline();

            tokio::select! {
                Some(event) = event_rx.recv() => {
                    if handle_event(indexer.root(), event, &mut state) {
                        health.pending_events = state.pending();
                        let _ = health_tx.send(health.clone());
                    }
                }
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        WatcherCommand::Trigger { reason } => {
                            state.force_run(reason);
                            health.pending_events = state.pending();
                            let _ = health_tx.send(health.clone());
                        }
                        WatcherCommand::Shutdown => break,
                    }
                }
                () = async {
                    if let Some(deadline) = next_deadline {
                        time::sleep_until(deadline).await;
                    }
                }, if state.should_run() && next_deadline.is_some() => {
                    health.indexing = true;
                    let _ = health_tx.send(health.clone());

                    let snapshot_models = {
                        let guard = models.lock().await;
                        guard.clone()
                    };

                    if snapshot_models.is_empty() {
                        warn!("Multi-model watcher has no configured models; skipping index cycle");
                        health.indexing = false;
                        health.pending_events = 0;
                        let _ = health_tx.send(health.clone());
                        state.reset();
                        continue;
                    }

                    match run_multi_model_index_cycle(
                        indexer.clone(),
                        snapshot_models,
                        state.take_reason().unwrap_or_else(|| DEFAULT_ALERT_REASON.to_string()),
                    ).await {
                        Ok((cycle_stats, duration, reason, store_size)) => {
                            health.last_success = Some(SystemTime::now());
                            health.last_duration_ms = Some(duration);
                            health.last_error = None;
                            health.consecutive_failures = 0;
                            health.indexing = false;
                            health.pending_events = 0;
                            if duration > 0 {
                                let files_per_sec =
                                    cycle_stats.files as f32 / (duration as f32 / 1000.0);
                                health.last_throughput_files_per_sec = Some(files_per_sec);
                            }
                            health.last_index_size_bytes = store_size;
                            record_duration(&mut duration_history, duration);
                            health.p95_duration_ms = compute_p95(&duration_history);
                            health.alert_log_json = serialize_alerts(&alert_log);
                            health.alert_log_len = alert_log.len();
                            if let Err(err) = write_health_snapshot(
                                indexer.root(),
                                &cycle_stats,
                                &reason,
                                health.p95_duration_ms,
                                Some(health.pending_events),
                            )
                            .await
                            {
                                warn!("Failed to persist health snapshot after watcher index: {err}");
                            }
                            let _ = health_tx.send(health.clone());
                            let _ = update_tx.send(IndexUpdate {
                                completed_at: SystemTime::now(),
                                duration_ms: duration,
                                stats: Some(cycle_stats.clone()),
                                success: true,
                                reason,
                                store_size_bytes: store_size,
                            });
                        }
                        Err((err, duration, reason)) => {
                            error!("Streaming index failure: {err}");
                            health.last_error = Some(err.clone());
                            health.consecutive_failures += 1;
                            health.last_duration_ms = Some(duration);
                            health.indexing = false;
                            health.pending_events = 0;
                            if let Err(e) = crate::append_failure_reason(
                                indexer.root(),
                                &reason,
                                &err,
                                health.p95_duration_ms,
                            )
                            .await
                            {
                                warn!("Failed to persist failure reason: {e}");
                            }
                            push_alert(&mut alert_log, "error", &reason, &err);
                            health.alert_log_json = serialize_alerts(&alert_log);
                            health.alert_log_len = alert_log.len();
                            let _ = health_tx.send(health.clone());
                            let _ = update_tx.send(IndexUpdate {
                                completed_at: SystemTime::now(),
                                duration_ms: duration,
                                stats: None,
                                success: false,
                                reason,
                                store_size_bytes: None,
                            });
                        }
                    }

                    state.reset();
                }
            }
        }
    });
}

async fn run_index_cycle(
    indexer: Arc<ProjectIndexer>,
    reason: String,
) -> std::result::Result<(IndexStats, u64, String, Option<u64>), (String, u64, String)> {
    let started = Instant::now();
    match indexer.index().await {
        Ok(stats) => {
            #[allow(clippy::cast_possible_truncation)]
            let duration = started.elapsed().as_millis() as u64;
            info!("Incremental index finished in {duration}ms");
            let store_size = tokio::fs::metadata(indexer.store_path())
                .await
                .ok()
                .map(|meta| meta.len());
            Ok((stats, duration, reason, store_size))
        }
        Err(e) => {
            #[allow(clippy::cast_possible_truncation)]
            let duration = started.elapsed().as_millis() as u64;
            Err((e.to_string(), duration, reason))
        }
    }
}

async fn run_multi_model_index_cycle(
    indexer: Arc<MultiModelProjectIndexer>,
    models: Vec<ModelIndexSpec>,
    reason: String,
) -> std::result::Result<(IndexStats, u64, String, Option<u64>), (String, u64, String)> {
    let started = Instant::now();
    match indexer.index_models(&models, false).await {
        Ok(stats) => {
            #[allow(clippy::cast_possible_truncation)]
            let duration = started.elapsed().as_millis() as u64;
            info!("Incremental multi-model index finished in {duration}ms");
            let store_size = sum_model_store_sizes(indexer.root(), &models).await;
            Ok((stats, duration, reason, store_size))
        }
        Err(e) => {
            #[allow(clippy::cast_possible_truncation)]
            let duration = started.elapsed().as_millis() as u64;
            Err((e.to_string(), duration, reason))
        }
    }
}

async fn sum_model_store_sizes(root: &Path, models: &[ModelIndexSpec]) -> Option<u64> {
    let mut sum = 0u64;
    let mut any = false;
    for spec in models {
        let path = root
            .join(".context-finder")
            .join("indexes")
            .join(model_id_dir_name(&spec.model_id))
            .join("index.json");
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            sum = sum.saturating_add(meta.len());
            any = true;
        }
    }
    any.then_some(sum)
}

fn model_id_dir_name(model_id: &str) -> String {
    model_id
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

fn handle_event(root: &Path, event: notify::Result<Event>, state: &mut DebounceState) -> bool {
    match event {
        Ok(evt) => {
            if evt.paths.is_empty() {
                state.record_event(1, DEFAULT_ALERT_REASON);
                return true;
            }

            let mut relevant = 0;
            for path in evt.paths {
                if is_relevant_path(root, &path) && state.record_path_if_new(&path) {
                    relevant += 1;
                }
            }
            if relevant > 0 {
                state.record_event(relevant, DEFAULT_ALERT_REASON);
                return true;
            }
            false
        }
        Err(err) => {
            warn!("Watcher error: {err}");
            false
        }
    }
}

fn is_relevant_path(root: &Path, path: &Path) -> bool {
    const IGNORED: &[&str] = &[
        ".git",
        ".hg",
        ".svn",
        ".context-finder",
        "target",
        "node_modules",
        "dist",
        "build",
        "out",
        "datasets",
    ];

    if let Ok(relative) = path.strip_prefix(root) {
        let mut components = relative.components();
        if let Some(first) = components.next() {
            let first = first.as_os_str().to_string_lossy().to_lowercase();
            if IGNORED.iter().any(|ignore| first.starts_with(ignore)) {
                return false;
            }
            // bench/logs/*.json noise
            if first == "bench" {
                if let Some(seg2) = components.next() {
                    let s2 = seg2.as_os_str().to_string_lossy().to_lowercase();
                    if s2 == "logs" && path.extension().map(|e| e == "json").unwrap_or(false) {
                        return false;
                    }
                }
            }
        }

        // ignore .gitignore anywhere
        if relative
            .file_name()
            .map(|f| f.to_string_lossy() == ".gitignore")
            .unwrap_or(false)
        {
            return false;
        }
    }

    true
}

#[derive(Debug, Serialize)]
struct AlertRecord {
    timestamp_unix_ms: u64,
    level: String,
    reason: String,
    detail: String,
}

struct DebounceState {
    debounce: Duration,
    max_batch: Duration,
    dirty: bool,
    pending: usize,
    last_event: Option<Instant>,
    first_event: Option<Instant>,
    reason: Option<String>,
    force_immediate: bool,
    recent_paths: VecDeque<(String, Instant)>,
    dedup_window: Duration,
}

impl DebounceState {
    const fn new(debounce: Duration, max_batch: Duration) -> Self {
        Self {
            debounce,
            max_batch,
            dirty: false,
            pending: 0,
            last_event: None,
            first_event: None,
            reason: None,
            force_immediate: false,
            recent_paths: VecDeque::new(),
            dedup_window: Duration::from_millis(750),
        }
    }

    fn record_event(&mut self, count: usize, reason: &str) {
        self.pending += count.max(1);
        self.reason = Some(reason.to_string());
        self.last_event = Some(Instant::now());
        self.first_event.get_or_insert_with(Instant::now);
        self.dirty = true;
    }

    fn force_run(&mut self, reason: String) {
        self.pending += 1;
        self.reason = Some(reason);
        self.force_immediate = true;
        self.dirty = true;
    }

    const fn pending(&self) -> usize {
        self.pending
    }

    const fn should_run(&self) -> bool {
        self.dirty
    }

    fn next_deadline(&self) -> Option<time::Instant> {
        if !self.dirty {
            return None;
        }

        if self.force_immediate {
            return Some(time::Instant::now());
        }

        let mut deadline = None;

        if let Some(last) = self.last_event {
            deadline = Some(last + self.debounce);
        }

        if let Some(first) = self.first_event {
            let forced = first + self.max_batch;
            deadline = Some(match deadline {
                Some(current) if forced < current => forced,
                Some(current) => current,
                None => forced,
            });
        }

        deadline.map(time::Instant::from_std)
    }

    fn take_reason(&mut self) -> Option<String> {
        self.reason.take()
    }

    fn reset(&mut self) {
        self.dirty = false;
        self.pending = 0;
        self.last_event = None;
        self.first_event = None;
        self.reason = None;
        self.force_immediate = false;
        self.recent_paths.clear();
    }

    #[cfg(test)]
    const fn force_flag(&self) -> bool {
        self.force_immediate
    }

    fn record_path_if_new(&mut self, path: &Path) -> bool {
        let now = Instant::now();
        let key = path.to_string_lossy().to_string();
        self.recent_paths
            .retain(|(_, ts)| now.duration_since(*ts) <= self.dedup_window);
        let already = self.recent_paths.iter().any(|(p, _)| p == &key);
        if !already {
            self.recent_paths.push_back((key, now));
            true
        } else {
            false
        }
    }
}

fn record_duration(history: &mut VecDeque<u64>, duration: u64) {
    const MAX_HISTORY: usize = 20;
    history.push_back(duration);
    if history.len() > MAX_HISTORY {
        history.pop_front();
    }
}

fn compute_p95(history: &VecDeque<u64>) -> Option<u64> {
    if history.is_empty() {
        return None;
    }
    let mut sorted: Vec<u64> = history.iter().copied().collect();
    sorted.sort_unstable();
    let idx = ((sorted.len() as f32 - 1.0) * 0.95).round() as usize;
    sorted.get(idx).copied()
}

fn push_alert(log: &mut VecDeque<AlertRecord>, level: &str, reason: &str, detail: &str) {
    const MAX_ALERTS: usize = 20;
    let record = AlertRecord {
        timestamp_unix_ms: current_unix_ms(),
        level: level.to_string(),
        reason: reason.to_string(),
        detail: detail.to_string(),
    };
    log.push_back(record);
    if log.len() > MAX_ALERTS {
        log.pop_front();
    }
}

fn serialize_alerts(log: &VecDeque<AlertRecord>) -> String {
    serde_json::to_string(log).unwrap_or_else(|_| "[]".to_string())
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|dur| dur.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::DebounceState;
    use std::time::Duration;

    #[test]
    fn debounce_generates_deadline() {
        let mut state = DebounceState::new(Duration::from_millis(100), Duration::from_secs(1));
        state.record_event(1, "fs_event");
        assert!(state.should_run());
        assert!(state.next_deadline().is_some());
    }

    #[test]
    fn force_run_sets_immediate_deadline() {
        let mut state = DebounceState::new(Duration::from_secs(5), Duration::from_secs(10));
        state.force_run("manual".to_string());
        assert!(state.should_run());
        assert!(state.force_flag());
        assert!(state.next_deadline().is_some());
    }
}
