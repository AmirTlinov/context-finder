# Context Finder

**Мгновенная навигация и контекст для ИИ-моделей в любом проекте**

Context Finder — это CLI-инструмент для семантического поиска по кодовым базам, оптимизированный для использования ИИ-моделями через shell commands. Фокус на точности поиска и эффективном использовании embeddings + AST-aware анализа.

## 🎯 Основные возможности

### Phase 3 (Code Intelligence) 🧠 **NEW!**

- **Code Graph Analysis** — AST-based extraction call chains & dependencies
- **Context-Aware Search** — автоматическая сборка related code (flagship!)
- **Smart Context Assembly** — relevance scoring по distance + relationship type
- **Parallel Processing** — 16x concurrent file reads (3-5x ускорение)

### Phase 2 (Flagship AI Agent Optimization) ✨

- **Контекстуальные embeddings** — imports + docstrings + code для богатой семантики
- **Инкрементальная индексация** — 62x ускорение (33.8s → 0.54s при отсутствии изменений)
- **Batch search API** — параллельный multi-query для эффективных AI workflows
- **Qualified names** — структурированные имена методов (Class::method)

### Core Features

- **Семантическое разбиение кода** — AST-aware chunking с Tree-sitter
- **Гибридный поиск** — semantic (adaptive) + fuzzy + RRF fusion для 100% точности
- **Векторный поиск** — ONNX Runtime (CUDA) + HNSW для точного семантического поиска
- **CLI с JSON выводом** — 4 команды, полностью parseable для ИИ-моделей
- **Адаптивная query expansion** — 100+ code-specific синонимов, tokenization
- **Мультиязычность** — Rust, Python, JS/TS с полным AST-пониманием

## 📊 Архитектура системы

```
┌─────────────────────────────────────────────────────────────────────┐
│                          Context Finder                               │
│                     Flagship-level Code Navigation                    │
└─────────────────────────────────────────────────────────────────────┘

         ┌───────────────┐
         │  Source Code  │
         │  (любой язык) │
         └───────┬───────┘
                 │
                 ▼
    ┌────────────────────────┐
    │   Code Chunker         │ ◄─── Tree-sitter AST Parser
    │   (AST-aware)          │      • Семантические границы
    └────────┬───────────────┘      • Контекст (imports, scopes)
             │                      • Метаданные (types, names)
             │
             ├──────────────────────┬─────────────────────┐
             ▼                      ▼                     ▼
    ┌─────────────────┐   ┌─────────────────┐   ┌──────────────┐
    │  Vector Store   │   │  Fuzzy Index    │   │   Indexer    │
    │  (HNSW + FAISS) │   │  (nucleo)       │   │  (metadata)  │
    │                 │   │                 │   │              │
    │  • Embeddings   │   │  • Path match   │   │  • Symbols   │
    │  • ANN Search   │   │  • Content fuzz │   │  • Relations │
    └────────┬────────┘   └────────┬────────┘   └──────┬───────┘
             │                     │                    │
             └──────────┬──────────┴────────────────────┘
                        │
                        ▼
              ┌───────────────────┐
              │  Retrieval Engine │
              │  (Hybrid Search)  │
              ├───────────────────┤
              │ 1. Fuzzy Search   │ ──► Top-K candidates
              │ 2. Semantic Search│ ──► Top-K candidates
              │ 3. Fusion (RRF)   │ ──► Combined results
              │ 4. Reranking      │ ──► Final ranked list
              └─────────┬─────────┘
                        │
                        │
                        ▼
              ┌───────────────────┐
              │   CLI (4 команды) │
              │   JSON output     │
              │                   │
              │  • index          │
              │  • search         │
              │  • get-context    │
              │  • list-symbols   │
              └───────────────────┘
```

## 🔍 Pipeline гибридного поиска

```
Query: "async error handling"
   │
   ├─► Fuzzy Search (nucleo-matcher)
   │     • Поиск по путям файлов
   │     • Поиск в содержимом
   │     • Score: 0-1 (normalized)
   │     └─► [ {chunk, score: 0.85}, ... ] (Top 50)
   │
   ├─► Semantic Search (embeddings)
   │     • Векторизация запроса
   │     • ANN через HNSW index
   │     • Cosine similarity
   │     └─► [ {chunk, score: 0.92}, ... ] (Top 50)
   │
   └─► Fusion (RRF - Reciprocal Rank Fusion)
         • Combine: fuzzy × 0.3 + semantic × 0.7
         • RRF formula: Σ 1/(k + rank_i)
         • k = 60 (tunable constant)
         └─► [ {chunk, fused_score}, ... ]
               │
               ▼
         Reranking (Contextual)
               • Cross-encoder (опционально)
               • Context similarity
               • Boost по metadata
               └─► Final Top-N Results
```

## 🚀 Быстрый старт

### Установка

```bash
# Из исходников
git clone https://github.com/yourusername/context-finder
cd context-finder
cargo build --release

# Установка глобально
cargo install --path crates/cli
```

### Выбор embedding-модели

По умолчанию используется ONNX Runtime CUDA (BGE-small). Скачайте ONNX + tokenizer в `~/.cache/context-finder/models/bge-small` (или `CONTEXT_FINDER_MODEL_DIR`) через `python scripts/download_onnx_models.py`. CPU fallback отключён, требуется GPU с CUDA. 
Для загрузки моделей нужен `huggingface_hub` (`pip install huggingface_hub`). Поддерживаемая модель: `bge-small`.

**Переменные окружения (GPU):**
- `CONTEXT_FINDER_EMBEDDING_MODEL` — `bge-small` (default)
- `CONTEXT_FINDER_MODEL_DIR` — корень кэша моделей (если не `~/.cache/context-finder/models`)
- `CONTEXT_FINDER_CUDA_DEVICE` — ID GPU (int, optional, default 0)
- `CONTEXT_FINDER_CUDA_MEM_LIMIT_MB` — лимит арены CUDA EP в мегабайтах (optional)
- `CONTEXT_FINDER_PROFILE` — профиль правил поиска (`general` по умолчанию, можно передать `--profile targeted/venorus`)

- `CONTEXT_FINDER_EMBEDDING_MODEL=bge-small` (default, 384d)
- `ORT_LIB_LOCATION=$HOME/.cache/ort.pyke.io/dfbin/x86_64-unknown-linux-gnu/<hash>/onnxruntime/lib`
- `LD_LIBRARY_PATH=$ORT_LIB_LOCATION:$HOME/.local/lib/python3.12/site-packages/nvidia/cublas/lib:$HOME/.local/lib/python3.12/site-packages/nvidia/cuda_runtime/lib:$HOME/.local/lib/python3.12/site-packages/nvidia/curand/lib:$HOME/.local/lib/python3.12/site-packages/nvidia/cufft/lib:$HOME/.local/lib/python3.12/site-packages/nvidia/cudnn/lib:/usr/local/cuda/targets/x86_64-linux/lib:/usr/local/cuda/lib64`

### Использование CLI

```bash
# Индексация проекта
context-finder index /path/to/project
# Output: {"status":"ok","chunks":1893,"files":247,"time_ms":8300}

# Поиск по проекту
context-finder search "async error handling" --limit 10
# Output: JSON с results[{file, lines, symbol, score, content, context}]

# Получить контекст для строки (для ИИ навигации)
context-finder get-context src/main.rs 42 --window 20
# Output: JSON с symbol, parent, imports, content, window

# Список символов в файле
context-finder list-symbols src/lib.rs
# Output: JSON с symbols[{name, type, parent, line}]
```

## Профили поиска и rerank

- Профиль `general` (по умолчанию) усиляет `src/lib/utils/configs/tools/bench`, штрафует `tests/docs/docker/infra/vendor/logs` и задаёт пороги rerank (fuzzy ≥ 0.18, semantic ≥ 0.05), BM25 окно 210 и boosts: path 1.8, symbol 2.2, yaml 0.9, bm25 1.1.
- Профиль `targeted/venorus` наследует `general`, добавляет must-hit для ключевых конфигов/скриптов Venorus и более агрессивные boosts (path 2.0, symbol 2.6, yaml 1.0, bm25 1.2).
- Переключение профиля: `context-finder search "query" --profile targeted/venorus` или `CONTEXT_FINDER_PROFILE=targeted/venorus`.
- Формат профиля (JSON/TOML): секции `paths` (boost/penalty/reject/noise + must_hit), `rerank.thresholds` (min_fuzzy_score, min_semantic_score), `rerank.bm25` (k1, b, window), `rerank.boosts` (path/symbol/yaml_path/bm25), `rerank.must_hit.base_bonus` (буст для обязателных попаданий).

## Локальный CUDA runtime (без глобальной установки)

- Скачайте нужные so (onnxruntime-gpu + CUDA libs, включая NVRTC) в локальный кэш: `bash scripts/setup_cuda_deps.sh` (≈1.8–2 ГБ в `.deps/ort_cuda`, игнорируется Git).
- Обёртка `scripts/run_cf_cuda.sh` выставляет ORT_STRATEGY=system и указывает ORT_LIB_LOCATION/LD_LIBRARY_PATH на `.deps/ort_cuda` (локальный ORT + CUDA). Если ругается на `libnvrtc.so.12`, обновите deps через скрипт (тянется `nvidia-cuda-nvrtc-cu12`) или докиньте so вручную.
- Запуск с локальными либами: `scripts/run_cf_cuda.sh command --json '{"action":"search","payload":{"query":"foo","limit":3}}' --project /path/to/repo`.
- Если на хосте нет подходящего драйвера/устройства, можно форсить CPU: `ORT_DISABLE_CUDA=1 scripts/run_cf_cuda.sh ...`.

## ♾ Непрерывный индекс + health RPC

### Watch-демон

1. Запустите долгоживущий сервер, который сам индексирует изменения:

   ```bash
   context-finder serve \
     --project /path/to/repo \
     --bind 0.0.0.0:50051 \
     --graph-language rust \
     --context-depth 2
   ```

2. По умолчанию включается `StreamingIndexer`: notify-вотчер собирает события, дебаунсит бурсты и триггерит инкрементальные пересборки (<2 с для одиночного файла). Для ручного режима есть `--no-watch`.

3. Параметры отклика тюнятся флагами `--watch-debounce-ms` и `--watch-max-batch-ms`. Первый задаёт дебаунс одного события, второй — максимальное ожидание перед форсированием пачки.

### Health / Trigger RPC

gRPC API публикует состояние цикла и алерты. Примеры (используйте `grpcurl`):

```bash
grpcurl -plaintext -d '{}' \
  127.0.0.1:50051 contextfinder.ContextFinder/GetHealth
```

Ответ содержит:

```json
{
  "hasWatcher": true,
  "indexing": false,
  "lastSuccessUnixMs": 1732031388123,
  "lastDurationMs": 421,
  "filesPerSecond": 5120.4,
  "indexSizeBytes": 18765432,
  "durationP95Ms": 560,
  "alertLogJson": "[{\"timestamp_unix_ms\":...,\"level\":\"error\",...}]"
}
```

`alertLogJson` — готовый JSON лог для автоматических алертов (последние 20 событий). Для ручного перезапуска индекса запустите:

В CLI те же данные попадают в `meta`: `health_last_success_ms`, `health_last_failure_ms`, `health_failure_reasons`, а также размеры `index_size_bytes` и `graph_cache_size_bytes` — можно сразу понять, свежий ли индекс и прогрет ли граф.
Дополнительно выводятся `health_p95_ms` (p95 длительности последних прогонов) и `health_failure_count` — помогает быстро увидеть деградацию индексации.
`health_files_per_sec` и `health_stale_ms` дают throughput и «возраст» индекса; при >15 минут появится warn-hint о необходимости пересборки.

```bash
grpcurl -plaintext -d '{"reason":"nightly-regen"}' \
  127.0.0.1:50051 contextfinder.ContextFinder/TriggerIndex
```

Команда вернёт подтверждение и поставит задачу в очередь, минуя дебаунс.

#### Prometheus endpoint

Для scrape без gRPC поднимите дополнительный HTTP endpoint:

```bash
context-finder serve \
  --project /path/to/repo \
  --metrics-bind 127.0.0.1:9100

curl http://127.0.0.1:9100/metrics
```

Экспортируются gauge-метрики `contextfinder_last_index_duration_ms`, `contextfinder_files_per_second`, `contextfinder_alert_log_len` и др., поэтому Prometheus/Alertmanager могут напрямую наблюдать за задержками и сбоями.

### Использование как библиотека

```rust
use context_code_chunker::{Chunker, ChunkerConfig};
use context_vector_store::VectorStore;
use context_search::HybridSearch;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Chunking
    let chunker = Chunker::new(ChunkerConfig::for_embeddings());
    let chunks = chunker.chunk_file("src/main.rs")?;

    // 2. Vector store + indexing
    let mut store = VectorStore::new("vectors.db").await?;
    store.add_chunks(chunks.clone()).await?;

    // 3. Hybrid search (semantic + fuzzy)
    let search = HybridSearch::new(store).await?;
    let results = search.search("error handling", 10).await?;

    // 4. Output as JSON
    println!("{}", serde_json::to_string_pretty(&results)?);

    Ok(())
}
```

## 📦 Компоненты

### 1. **code-chunker** — Семантическое разбиение кода

- Tree-sitter AST parsing для Rust/Python/JS/TS
- Сохранение контекста (imports, parent scopes)
- Стратегии: Semantic (primary), LineCount, TokenAware
- Метаданные: symbol names, types, documentation

### 2. **vector-store** — Векторное хранилище

- ONNX Runtime (CUDA) для embeddings (BGE/Jina, 384–1024d)
- HNSW index для быстрого ANN search
- Персистентность (JSON + binary)
- Batch processing для эффективности

### 3. **search** — Гибридный поиск

- Semantic search (70% вес) — embeddings + cosine similarity
- Fuzzy search (30% вес) — nucleo-matcher для имен
- RRF (Reciprocal Rank Fusion) для объединения
- AST-aware boosting (функции > variables)

### 4. **indexer** — Индексация проектов

- Параллельная обработка файлов (rayon)
- .gitignore aware (ignore crate)
- Pipeline: scan → chunk → embed → index
- Incremental updates (только измененные файлы)

### 5. **cli** — Командный интерфейс

- 4 команды: index, search, get-context, list-symbols
- Только JSON output (parseable для ИИ)
- Минимальные зависимости
- Install via `cargo install`

## ⚡ Производительность

| Операция | Время | Примечание |
|----------|-------|------------|
| Chunking (10K LOC) | 50-200ms | AST parsing + metadata |
| Embedding (1 chunk) | 5-15ms | ONNX Runtime CUDA (BGE/Jina) |
| Fuzzy search (100K chunks) | 1-5ms | nucleo-matcher |
| Semantic search (100K) | 10-50ms | HNSW index |
| Full hybrid search | 15-60ms | Fuzzy + Semantic + Fusion |
| Indexing (100K LOC) | 5-15s | Parallel, includes embeddings |

*Тесты на: AMD Ryzen 7 5800X, 32GB RAM, NVMe SSD*

## 🎯 Преимущества перед аналогами

| Аспект | Context Finder | Традиционные LSP | grep/ripgrep |
|--------|----------------|------------------|--------------|
| Семантический поиск | ✅ Гибридный | ❌ Только структура | ❌ Только текст |
| Скорость | ⚡ 15-60ms | 🐢 100-500ms | ⚡⚡ <5ms |
| Контекст | ✅ Полный | 🟡 Частичный | ❌ Нет |
| Мультиязычность | ✅ 10+ языков | 🟡 Зависит от LSP | ✅ Все файлы |
| ИИ-интеграция | ✅ Нативная | ❌ Нет | ❌ Нет |
| Инкрементальность | ✅ Да | ✅ Да | ❌ Нет |

## 🛠️ Разработка

```bash
# Запуск тестов
cargo test --all

# Проверка кода
cargo clippy --all-targets --all-features

# Форматирование
cargo fmt --all

# Benchmark
cargo bench

# Документация
cargo doc --open --no-deps
```

## 📄 Лицензия

MIT OR Apache-2.0

## 🤝 Вклад

Приветствуются pull requests! См. [CONTRIBUTING.md](CONTRIBUTING.md)

## 🙏 Благодарности

- [Codex CLI](https://github.com/openai/codex) — архитектурное вдохновение
- [Tree-sitter](https://tree-sitter.github.io/) — AST parsing
- [HNSW](https://github.com/nmslib/hnswlib) — ANN search
- [ONNX Runtime](https://onnxruntime.ai/) — GPU embeddings backend

---

**Context Finder** — сделай навигацию по коду мгновенной! 🚀

### Task-aware подсказки (быстрые стратегии)

- **Debug (stacktrace/error/panic)** → extended + graph, reuse_graph=true. Тянет связанные вызовы/ошибки.
- **Refactor/rename/migrate** → direct (минимум шума, точные совпадения).
- **Navigation/architecture/overview/map** → extended (шире покрытие, можно добавить graph).
- **Perf/latency/throughput** → deep (транзитивные связи по графу).

Подсказки приходят автоматом в `hints`; выбранная стратегия отражается в `hints`/`meta`.

Примеры запросов:
- `panic stacktrace` → получите hint debug + graph paths из related (пути с типами ребер Calls/Uses/Tests, усечённые до 4 шагов).
- `rename user_service` → hint refactor, стратегия direct, минимум шума.
- `architecture overview` → hint navigation, выдача шире, можно добавить `show_graph=true`.
- `latency spike payment` → hint perf, стратегия deep, graph контекст приоритетен.

### CLI флаги
- `--quiet` — только предупреждения/ошибки в stderr (stdout остаётся чистым JSON)
- `-v/--verbose` — детальные логи в stderr
- SLA-подсказки: при устаревшем индексе (>15 мин), высоком п95 (>2s), большом бэклоге fs-событий или низком throughput поиск выдаст warn-hints и покажет параметры в `meta`.

### A/B сравнение (baseline vs context)

- Команда: `context-finder command --json '{"action":"compare_search","payload":{"queries":["q1","q2"],"limit":6}}'`
- Смотрите `data.summary` и meta: `compare_avg_baseline_ms`, `compare_avg_context_ms`, `compare_avg_overlap_ratio`, `compare_avg_related`.
- Hints выводят краткое резюме (`Baseline avg … vs context … (+related)`), cache-hit, а также health/graph cache состояние.
