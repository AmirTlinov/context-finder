use crate::error::{IndexerError, Result};
use crate::scanner::FileScanner;
use crate::stats::IndexStats;
use context_code_chunker::{Chunker, ChunkerConfig};
use context_vector_store::current_model_id;
use context_vector_store::EmbeddingTemplates;
use context_vector_store::VectorStore;
use context_vector_store::{corpus_path_for_project_root, ChunkCorpus};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

#[derive(Clone, Debug)]
pub struct ModelIndexSpec {
    pub model_id: String,
    pub templates: EmbeddingTemplates,
}

impl ModelIndexSpec {
    pub fn new(model_id: impl Into<String>, templates: EmbeddingTemplates) -> Self {
        Self {
            model_id: model_id.into(),
            templates,
        }
    }
}

/// Project indexer that scans, chunks, and indexes code
pub struct ProjectIndexer {
    root: PathBuf,
    store_path: PathBuf,
    model_id: String,
    chunker: Chunker,
    templates: Option<EmbeddingTemplates>,
}

/// Multi-model project indexer that scans/chunks files once and embeds the resulting chunks into
/// multiple model-specific indices.
pub struct MultiModelProjectIndexer {
    root: PathBuf,
    chunker: Chunker,
}

impl ProjectIndexer {
    /// Create new indexer for project
    pub async fn new(root: impl AsRef<Path>) -> Result<Self> {
        let model_id = current_model_id().unwrap_or_else(|_| "bge-small".to_string());
        Self::new_with_model_and_templates(root, model_id, None).await
    }

    pub async fn new_for_model(
        root: impl AsRef<Path>,
        model_id: impl Into<String>,
    ) -> Result<Self> {
        Self::new_with_model_and_templates(root, model_id.into(), None).await
    }

    pub async fn new_with_embedding_templates(
        root: impl AsRef<Path>,
        templates: EmbeddingTemplates,
    ) -> Result<Self> {
        let model_id = current_model_id().unwrap_or_else(|_| "bge-small".to_string());
        Self::new_with_model_and_templates(root, model_id, Some(templates)).await
    }

    pub async fn new_for_model_with_embedding_templates(
        root: impl AsRef<Path>,
        model_id: impl Into<String>,
        templates: EmbeddingTemplates,
    ) -> Result<Self> {
        Self::new_with_model_and_templates(root, model_id.into(), Some(templates)).await
    }

    async fn new_with_model_and_templates(
        root: impl AsRef<Path>,
        model_id: String,
        templates: Option<EmbeddingTemplates>,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();

        if !root.exists() {
            return Err(IndexerError::InvalidPath(format!(
                "Path does not exist: {}",
                root.display()
            )));
        }

        let model_dir = model_id_dir_name(&model_id);
        let store_path = root
            .join(".context-finder")
            .join("indexes")
            .join(model_dir)
            .join("index.json");

        // Create .context-finder directory if needed
        if let Some(parent) = store_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let chunker = Chunker::new(ChunkerConfig::for_embeddings());

        Ok(Self {
            root,
            store_path,
            model_id,
            chunker,
            templates,
        })
    }

    /// Index the project (with incremental support)
    pub async fn index(&self) -> Result<IndexStats> {
        self.index_with_mode(false).await
    }

    /// Index the project in full mode (skip incremental check)
    pub async fn index_full(&self) -> Result<IndexStats> {
        self.index_with_mode(true).await
    }

    /// Index with specified mode
    #[allow(clippy::cognitive_complexity)]
    #[allow(clippy::too_many_lines)]
    async fn index_with_mode(&self, force_full: bool) -> Result<IndexStats> {
        let start = Instant::now();
        let mut stats = IndexStats::new();

        log::info!("Indexing project at {}", self.root.display());

        // 1. Scan for files
        let scanner = FileScanner::new(&self.root);
        let files = scanner.scan();
        let live_files: HashSet<String> = files.iter().map(|p| self.normalize_path(p)).collect();

        let corpus_path = corpus_path_for_project_root(&self.root);
        let (mut corpus, corpus_full_rebuild) = if force_full {
            (ChunkCorpus::new(), true)
        } else if corpus_path.exists() {
            match ChunkCorpus::load(&corpus_path).await {
                Ok(corpus) => (corpus, false),
                Err(err) => {
                    log::warn!(
                        "Failed to load chunk corpus {}: {err}; will rebuild corpus",
                        corpus_path.display()
                    );
                    (ChunkCorpus::new(), true)
                }
            }
        } else {
            (ChunkCorpus::new(), true)
        };
        let mut corpus_dirty = corpus_full_rebuild;

        // 2. Load or create vector store
        let allow_incremental_store =
            !force_full && !corpus_full_rebuild && self.store_path.exists();
        let (mut store, existing_mtimes) = if allow_incremental_store {
            log::info!("Loading existing index for incremental update");
            let loaded = if let Some(templates) = self.templates.clone() {
                VectorStore::load_with_templates_for_model(
                    &self.store_path,
                    templates,
                    &self.model_id,
                )
                .await
            } else {
                VectorStore::load_for_model(&self.store_path, &self.model_id).await
            };
            match loaded {
                Ok(store) => {
                    // Load mtimes from metadata file if exists
                    let mtimes = self.load_mtimes().await.unwrap_or_default();
                    (store, Some(mtimes))
                }
                Err(e) => {
                    log::warn!("Failed to load existing index: {e}, starting fresh");
                    let store = if let Some(templates) = self.templates.clone() {
                        VectorStore::new_with_templates_for_model(
                            &self.store_path,
                            &self.model_id,
                            templates,
                        )?
                    } else {
                        VectorStore::new_for_model(&self.store_path, &self.model_id)?
                    };
                    (store, None)
                }
            }
        } else {
            if corpus_full_rebuild && self.store_path.exists() {
                log::info!(
                    "Chunk corpus rebuild detected; rebuilding semantic index at {}",
                    self.store_path.display()
                );
            }
            let store = if let Some(templates) = self.templates.clone() {
                VectorStore::new_with_templates_for_model(
                    &self.store_path,
                    &self.model_id,
                    templates,
                )?
            } else {
                VectorStore::new_for_model(&self.store_path, &self.model_id)?
            };
            (store, None)
        };

        // 3. Determine which files to process
        let files_to_process = if corpus_full_rebuild {
            files.clone()
        } else if let Some(ref mtimes_map) = existing_mtimes {
            self.filter_changed_files(&files, mtimes_map).await?
        } else {
            files.clone()
        };

        if existing_mtimes.is_some() {
            log::info!(
                "Incremental: processing {} of {} files",
                files_to_process.len(),
                files.len()
            );

            // Purge chunks that belong to files no longer present in the project (deleted/renamed).
            let removed = store.purge_missing_files(&live_files);
            if removed > 0 {
                log::info!("Purged {removed} stale chunks from deleted files");
            }

            let removed = corpus.purge_missing_files(&live_files);
            if removed > 0 {
                log::info!("Purged {removed} missing files from chunk corpus");
                corpus_dirty = true;
            }
        }

        // 4. Process files (parallel for better performance)
        let mut current_mtimes = HashMap::new();

        // Collect mtimes for all files first
        for file_path in &files {
            if let Ok(metadata) = tokio::fs::metadata(&file_path).await {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                        current_mtimes.insert(
                            file_path
                                .strip_prefix(&self.root)
                                .unwrap_or(file_path)
                                .to_string_lossy()
                                .to_string(),
                            duration.as_secs(),
                        );
                    }
                }
            }
        }

        // Process changed files in parallel (with concurrency limit)
        let changed_rels: HashSet<String> = files_to_process
            .iter()
            .map(|p| self.normalize_path(p))
            .collect();
        let corpus_targets: Vec<PathBuf> = if corpus_full_rebuild {
            files.clone()
        } else {
            files_to_process.clone()
        };

        if !corpus_targets.is_empty() {
            let results = self.process_files_parallel(&corpus_targets).await?;

            // Aggregate results
            for result in results {
                match result {
                    Ok((relative_path, chunks, language, lines)) => {
                        stats.add_file(&language, lines);
                        stats.add_chunks(chunks.len());

                        corpus.set_file_chunks(relative_path.clone(), chunks.clone());
                        corpus_dirty = true;

                        if changed_rels.contains(&relative_path) {
                            if existing_mtimes.is_some() {
                                store.remove_chunks_for_file(&relative_path);
                            }
                            store.add_chunks(chunks).await?;
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to process file: {e}");
                        stats.add_error(e);
                    }
                }
            }
        }

        // 5. Save store and mtimes
        if corpus_dirty {
            corpus.save(&corpus_path).await?;
        }
        store.save().await?;
        self.save_mtimes(&current_mtimes).await?;

        #[allow(clippy::cast_possible_truncation)]
        {
            stats.time_ms = start.elapsed().as_millis() as u64;
            if stats.time_ms == 0 {
                stats.time_ms = 1;
            }
        }
        log::info!("Indexing completed: {stats:?}");

        Ok(stats)
    }

    /// Filter files that have changed since last index
    async fn filter_changed_files(
        &self,
        files: &[PathBuf],
        existing_mtimes: &HashMap<String, u64>,
    ) -> Result<Vec<PathBuf>> {
        let mut changed = Vec::new();

        for file_path in files {
            let relative_path = file_path
                .strip_prefix(&self.root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            // Check if file is new or modified
            let metadata = tokio::fs::metadata(file_path).await?;
            let modified = metadata.modified()?;
            let mtime = modified.duration_since(SystemTime::UNIX_EPOCH)?.as_secs();

            let is_changed = existing_mtimes
                .get(&relative_path)
                .is_none_or(|&old_mtime| mtime > old_mtime); // New file

            if is_changed {
                changed.push(file_path.clone());
            }
        }

        Ok(changed)
    }

    /// Save file mtimes for incremental indexing
    async fn save_mtimes(&self, mtimes: &HashMap<String, u64>) -> Result<()> {
        let mtimes_path = self
            .store_path
            .parent()
            .ok_or_else(|| IndexerError::InvalidPath("store path has no parent".into()))?
            .join("mtimes.json");
        let json = serde_json::to_string_pretty(mtimes)?;
        tokio::fs::write(&mtimes_path, json).await?;
        Ok(())
    }

    /// Load file mtimes from previous index
    async fn load_mtimes(&self) -> Result<HashMap<String, u64>> {
        let mtimes_path = self
            .store_path
            .parent()
            .ok_or_else(|| IndexerError::InvalidPath("store path has no parent".into()))?
            .join("mtimes.json");
        if !mtimes_path.exists() {
            return Ok(HashMap::new());
        }

        let json = tokio::fs::read_to_string(&mtimes_path).await?;
        let mtimes: HashMap<String, u64> = serde_json::from_str(&json)?;
        Ok(mtimes)
    }

    /// Process files in parallel with concurrency limit
    async fn process_files_parallel(
        &self,
        files: &[PathBuf],
    ) -> Result<
        Vec<
            std::result::Result<
                (String, Vec<context_code_chunker::CodeChunk>, String, usize),
                String,
            >,
        >,
    > {
        const MAX_CONCURRENT: usize = 16;

        if files.is_empty() {
            return Ok(Vec::new());
        }

        let mut aggregated = Vec::with_capacity(files.len());

        for file_chunk in files.chunks(MAX_CONCURRENT) {
            let mut tasks = Vec::with_capacity(file_chunk.len());
            for file_path in file_chunk {
                let file_path = file_path.clone();
                let task = tokio::spawn(async move { Self::read_file_static(file_path).await });
                tasks.push(task);
            }

            for task in tasks {
                match task.await {
                    Ok(Ok((file_path, content, lines))) => {
                        let relative_path = self.normalize_path(&file_path);
                        match self.chunker.chunk_str(&content, Some(&relative_path)) {
                            Ok(chunks) => {
                                if chunks.is_empty() {
                                    aggregated.push(Ok((
                                        relative_path,
                                        vec![],
                                        "unknown".to_string(),
                                        lines,
                                    )));
                                } else {
                                    let language = chunks[0]
                                        .metadata
                                        .language
                                        .as_deref()
                                        .unwrap_or("unknown")
                                        .to_string();
                                    aggregated.push(Ok((relative_path, chunks, language, lines)));
                                }
                            }
                            Err(e) => {
                                aggregated.push(Err(format!("{}: {e}", file_path.display())));
                            }
                        }
                    }
                    Ok(Err(e)) => aggregated.push(Err(e)),
                    Err(e) => aggregated.push(Err(format!("Task panicked: {e}"))),
                }
            }
        }

        Ok(aggregated)
    }

    /// Static method for file reading (IO bound)
    async fn read_file_static(
        file_path: PathBuf,
    ) -> std::result::Result<(PathBuf, String, usize), String> {
        let content = tokio::fs::read_to_string(&file_path)
            .await
            .map_err(|e| format!("{}: {e}", file_path.display()))?;

        let lines = content.lines().count();

        Ok((file_path, content, lines))
    }

    /// Process single file (legacy method, kept for compatibility)
    #[allow(dead_code)]
    async fn process_file(
        &self,
        file_path: &Path,
        store: &mut VectorStore,
        stats: &mut IndexStats,
    ) -> Result<()> {
        log::debug!("Processing file: {}", file_path.display());

        let content = tokio::fs::read_to_string(file_path).await?;
        let lines = content.lines().count();

        // Chunk the file
        let relative_path = self.normalize_path(file_path);
        let chunks = self.chunker.chunk_str(&content, Some(&relative_path))?;

        if chunks.is_empty() {
            return Ok(());
        }

        let language = chunks[0].metadata.language.as_deref().unwrap_or("unknown");

        stats.add_file(language, lines);
        stats.add_chunks(chunks.len());

        // Add to vector store (batch embedding happens here)
        store.add_chunks(chunks).await?;

        Ok(())
    }

    /// Get store path
    #[must_use]
    pub fn store_path(&self) -> &Path {
        &self.store_path
    }

    /// Get project root
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn normalize_path(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.root).unwrap_or(path).to_path_buf();
        let mut normalized = relative.to_string_lossy().to_string();
        if normalized.contains('\\') {
            normalized = normalized.replace('\\', "/");
        }
        normalized
    }
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

impl MultiModelProjectIndexer {
    #[allow(clippy::unused_async)]
    pub async fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();

        if !root.exists() {
            return Err(IndexerError::InvalidPath(format!(
                "Path does not exist: {}",
                root.display()
            )));
        }

        Ok(Self {
            root,
            chunker: Chunker::new(ChunkerConfig::for_embeddings()),
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Index a project for multiple models.
    ///
    /// Design goals:
    /// - Scan + chunk once (union of changed files across models),
    /// - Keep incremental correctness per model (per-model mtimes + purge),
    /// - Avoid process-global env mutation (explicit `model_id` wiring).
    #[allow(clippy::cognitive_complexity)]
    #[allow(clippy::too_many_lines)]
    pub async fn index_models(
        &self,
        models: &[ModelIndexSpec],
        force_full: bool,
    ) -> Result<IndexStats> {
        struct ModelPlan {
            model_id: String,
            store_path: PathBuf,
            mtimes_path: PathBuf,
            templates: EmbeddingTemplates,
            incremental: bool,
            changed_files: HashSet<String>,
        }

        let started = Instant::now();
        if models.is_empty() {
            return Err(IndexerError::Other(
                "Multi-model indexing requires at least one model".to_string(),
            ));
        }

        log::info!(
            "Indexing project at {} (models={})",
            self.root.display(),
            models.len()
        );

        // 1. Scan for files once.
        let scanner = FileScanner::new(&self.root);
        let files = scanner.scan();

        let live_files: HashSet<String> = files.iter().map(|p| self.normalize_path(p)).collect();

        let corpus_path = corpus_path_for_project_root(&self.root);
        let (mut corpus, corpus_full_rebuild) = if force_full {
            (ChunkCorpus::new(), true)
        } else if corpus_path.exists() {
            match ChunkCorpus::load(&corpus_path).await {
                Ok(corpus) => (corpus, false),
                Err(err) => {
                    log::warn!(
                        "Failed to load chunk corpus {}: {err}; will rebuild corpus",
                        corpus_path.display()
                    );
                    (ChunkCorpus::new(), true)
                }
            }
        } else {
            (ChunkCorpus::new(), true)
        };
        let mut corpus_dirty = corpus_full_rebuild;

        // 2. Compute current mtimes for all files once.
        let mut current_mtimes: HashMap<String, u64> = HashMap::new();
        for file_path in &files {
            if let Ok(metadata) = tokio::fs::metadata(&file_path).await {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                        current_mtimes.insert(self.normalize_path(file_path), duration.as_secs());
                    }
                }
            }
        }

        // 3. Load per-model mtimes, compute union of changed files.
        let mut plans: Vec<ModelPlan> = Vec::with_capacity(models.len());
        let mut union_changed: HashSet<String> = HashSet::new();
        let mut abs_by_rel: HashMap<String, PathBuf> = HashMap::new();
        for file_path in &files {
            abs_by_rel.insert(self.normalize_path(file_path), file_path.clone());
        }

        for spec in models {
            let model_id = spec.model_id.trim().to_string();
            if model_id.is_empty() {
                return Err(IndexerError::Other(
                    "model_id must not be empty".to_string(),
                ));
            }

            let model_dir = model_id_dir_name(&model_id);
            let store_path = self
                .root
                .join(".context-finder")
                .join("indexes")
                .join(model_dir)
                .join("index.json");
            if let Some(parent) = store_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mtimes_path = store_path
                .parent()
                .expect("index.json has a parent dir")
                .join("mtimes.json");

            let incremental = !force_full && !corpus_full_rebuild && store_path.exists();
            let existing_mtimes = if incremental && mtimes_path.exists() {
                let json = tokio::fs::read_to_string(&mtimes_path).await?;
                serde_json::from_str::<HashMap<String, u64>>(&json)?
            } else {
                HashMap::new()
            };

            let mut changed_files = HashSet::new();
            if force_full || corpus_full_rebuild || !store_path.exists() {
                // Fresh index: process everything.
                for rel in current_mtimes.keys() {
                    changed_files.insert(rel.clone());
                }
            } else {
                for (rel, mtime) in &current_mtimes {
                    let is_changed = existing_mtimes.get(rel).is_none_or(|old| mtime > old);
                    if is_changed {
                        changed_files.insert(rel.clone());
                    }
                }
            }

            union_changed.extend(changed_files.iter().cloned());
            plans.push(ModelPlan {
                model_id,
                store_path,
                mtimes_path,
                templates: spec.templates.clone(),
                incremental,
                changed_files,
            });
        }

        // 4. Chunk the union set once.
        let mut stats = IndexStats::new();
        let mut union_paths: Vec<PathBuf> = if corpus_full_rebuild {
            files.clone()
        } else {
            union_changed
                .iter()
                .filter_map(|rel| abs_by_rel.get(rel).cloned())
                .collect()
        };
        union_paths.sort();

        let processed = if union_paths.is_empty() {
            Vec::new()
        } else {
            self.process_files_parallel(&union_paths).await?
        };

        let mut processed_by_rel: HashMap<String, Vec<context_code_chunker::CodeChunk>> =
            HashMap::new();
        let mut processed_errs: HashMap<String, String> = HashMap::new();

        for result in processed {
            match result {
                Ok((relative_path, chunks, language, lines)) => {
                    stats.add_file(&language, lines);
                    stats.add_chunks(chunks.len());
                    processed_by_rel.insert(relative_path, chunks);
                }
                Err(err) => {
                    stats.add_error(err.clone());
                    // Best-effort: parse "path: error" prefix if present.
                    let rel = err.split_once(':').map(|(p, _)| p.trim().to_string());
                    if let Some(rel) = rel {
                        processed_errs.insert(rel, err);
                    }
                }
            }
        }

        if !corpus_full_rebuild {
            let removed = corpus.purge_missing_files(&live_files);
            if removed > 0 {
                log::info!("Purged {removed} missing files from chunk corpus");
                corpus_dirty = true;
            }
        }

        for (relative_path, chunks) in &processed_by_rel {
            if processed_errs.contains_key(relative_path) {
                continue;
            }
            corpus.set_file_chunks(relative_path.clone(), chunks.clone());
            corpus_dirty = true;
        }

        if corpus_dirty {
            corpus.save(&corpus_path).await?;
        }

        // 5. Apply the chunk deltas per model (embed + update store).
        for plan in plans {
            let mut store = if plan.incremental && plan.store_path.exists() {
                let loaded = VectorStore::load_with_templates_for_model(
                    &plan.store_path,
                    plan.templates.clone(),
                    &plan.model_id,
                )
                .await;
                match loaded {
                    Ok(store) => store,
                    Err(e) => {
                        log::warn!(
                            "Failed to load existing index {}: {e}, starting fresh",
                            plan.store_path.display()
                        );
                        VectorStore::new_with_templates_for_model(
                            &plan.store_path,
                            &plan.model_id,
                            plan.templates.clone(),
                        )?
                    }
                }
            } else {
                VectorStore::new_with_templates_for_model(
                    &plan.store_path,
                    &plan.model_id,
                    plan.templates.clone(),
                )?
            };

            if plan.incremental {
                let removed = store.purge_missing_files(&live_files);
                if removed > 0 {
                    log::info!("Purged {removed} stale chunks for model {}", plan.model_id);
                }
            }

            for rel in &plan.changed_files {
                if processed_errs.contains_key(rel) {
                    continue;
                }
                let Some(chunks) = processed_by_rel.get(rel) else {
                    continue;
                };

                if plan.incremental {
                    store.remove_chunks_for_file(rel);
                }

                store.add_chunks(chunks.clone()).await?;
            }

            store.save().await?;

            // Persist mtimes for this model so incremental correctness is per-model (avoids
            // cross-model skew if users index subsets of experts).
            let json = serde_json::to_string_pretty(&current_mtimes)?;
            tokio::fs::write(&plan.mtimes_path, json).await?;
        }

        #[allow(clippy::cast_possible_truncation)]
        {
            stats.time_ms = started.elapsed().as_millis() as u64;
            if stats.time_ms == 0 {
                stats.time_ms = 1;
            }
        }

        Ok(stats)
    }

    fn normalize_path(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.root).unwrap_or(path).to_path_buf();
        let mut normalized = relative.to_string_lossy().to_string();
        if normalized.contains('\\') {
            normalized = normalized.replace('\\', "/");
        }
        normalized
    }

    async fn process_files_parallel(
        &self,
        files: &[PathBuf],
    ) -> Result<
        Vec<
            std::result::Result<
                (String, Vec<context_code_chunker::CodeChunk>, String, usize),
                String,
            >,
        >,
    > {
        const MAX_CONCURRENT: usize = 16;

        if files.is_empty() {
            return Ok(Vec::new());
        }

        let mut aggregated = Vec::with_capacity(files.len());

        for file_chunk in files.chunks(MAX_CONCURRENT) {
            let mut tasks = Vec::with_capacity(file_chunk.len());
            for file_path in file_chunk {
                let file_path = file_path.clone();
                let task =
                    tokio::spawn(async move { ProjectIndexer::read_file_static(file_path).await });
                tasks.push(task);
            }

            for task in tasks {
                match task.await {
                    Ok(Ok((file_path, content, lines))) => {
                        let relative_path = self.normalize_path(&file_path);
                        match self.chunker.chunk_str(&content, Some(&relative_path)) {
                            Ok(chunks) => {
                                if chunks.is_empty() {
                                    aggregated.push(Ok((
                                        relative_path,
                                        vec![],
                                        "unknown".to_string(),
                                        lines,
                                    )));
                                } else {
                                    let language = chunks[0]
                                        .metadata
                                        .language
                                        .as_deref()
                                        .unwrap_or("unknown")
                                        .to_string();
                                    aggregated.push(Ok((relative_path, chunks, language, lines)));
                                }
                            }
                            Err(e) => {
                                aggregated.push(Err(format!("{}: {e}", file_path.display())));
                            }
                        }
                    }
                    Ok(Err(e)) => aggregated.push(Err(e)),
                    Err(e) => aggregated.push(Err(format!("Task panicked: {e}"))),
                }
            }
        }

        Ok(aggregated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    #[ignore = "Requires ONNX embedding model"]
    async fn test_indexing() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.rs");

        tokio::fs::write(
            &test_file,
            r#"
fn hello() {
    println!("hello");
}

struct Point {
    x: i32,
    y: i32,
}
"#,
        )
        .await
        .unwrap();

        let indexer = ProjectIndexer::new(temp_dir.path()).await.unwrap();
        let stats = indexer.index().await.unwrap();

        assert!(stats.files > 0);
        assert!(stats.chunks > 0);
    }
}
