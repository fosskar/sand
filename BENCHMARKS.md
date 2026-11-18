# Benchmarks

Retrieval quality and latency compared against five MCP memory systems, using 1,000 developer memories and 200 search queries. Full benchmark suite: [sediment-benchmark](https://github.com/rendro/sediment-benchmark).

## Results

Measured on Apple M3 Max, 36GB RAM.

| Metric | Sediment | ChromaDB | Mem0 | MCP Memory Service | MCP Server Memory | Basic Memory |
|--------|----------|----------|------|--------------------|--------------------|-------------|
| Recall@1 | **50.0%** | 47.0% | 47.0% | 38.0% | 1.0% | 9.0% |
| Recall@5 | 77.5% | **78.5%** | **78.5%** | 66.0% | 2.0% | 11.5% |
| MRR | **61.9%** | 60.8% | 60.8% | 49.0% | 1.5% | 10.2% |
| nDCG@5 | 58.7% | **59.9%** | **59.9%** | 47.8% | 1.6% | 10.4% |
| Recency@1 | **100%** | 14% | 14% | 10% | 0% | 0% |
| Consolidation | **99%** | 0% | 0% | 0% | 0% | 0% |
| Store p50 | 50ms | 692ms | 14ms | **2ms** | **2ms** | 49ms |
| Recall p50 | 103ms | 694ms | **8ms** | 10ms | **2ms** | 28ms |

MCP Server Memory and Basic Memory use keyword/entity matching rather than vector search, which explains their low retrieval scores on semantic queries.

## Key takeaways

- **Retrieval quality**: Best R@1 (50.0%) and MRR (61.9%) among all systems — top result is correct more often than alternatives
- **Temporal correctness**: 100% Recency@1 — updated memories always rank first. Best competitor: 14%
- **Deduplication**: 99% consolidation rate — near-duplicates auto-merged. All others: 0%
- **Latency**: 14x faster store than ChromaDB (50ms vs 692ms). All operations local, no network

## Embedding model comparison

Sediment supports multiple embedding models via the `SEDIMENT_EMBEDDING_MODEL` environment variable. All models achieve 100% temporal correctness and 99% dedup consolidation.

| Metric | all-MiniLM-L6-v2 (default) | bge-base-en-v1.5 | bge-small-en-v1.5 | e5-small-v2 |
|--------|---------------------------|-------------------|-------------------|-------------|
| Dimensions | 384 | 768 | 384 | 384 |
| Recall@1 | 50.0% | **50.5%** | 46.5% | 42.0% |
| Recall@5 | **77.5%** | 76.0% | 75.0% | 68.0% |
| MRR | 61.9% | **62.0%** | 58.4% | 53.5% |
| nDCG@5 | **58.7%** | 57.6% | 55.5% | 51.1% |
| Store p50 | **50ms** | 92ms | 64ms | 63ms |
| Recall p50 | **103ms** | 149ms | 120ms | 118ms |

all-MiniLM-L6-v2 remains the default: essentially tied with bge-base-en-v1.5 on quality but ~2x faster due to smaller dimensions (384 vs 768).

## Embedding model comparison, sand 0.6.0 (2026-11-18)

This section re-measures the embedding models on sand and adds newer candidates. It does not replace the Sediment results above, which were measured by upstream on an Apple M3 Max.

### Machine and method

- **Machine**: AMD Ryzen 9 9955HX (16 cores, 32 threads), 91 GiB RAM, NixOS, Linux 7.3.4. Inference on the CPU through Candle 0.8.3.
- **Suite**: [sediment-benchmark](https://github.com/rendro/sediment-benchmark) at commit `49ab23a`, Sediment adapter only, all four phases (retrieval, temporal, dedup, latency). Python 3.14 with `mcp` 2.3.0 from nixpkgs; the adapter reads `CallToolResult.isError`, which `mcp` 2.x renamed to `is_error`. The wrapper script adds the old name as an alias. The wrapper also gives every run its own `/tmp` directory, because the server puts `access.db` next to the database directory and the adapter's temporary directories would otherwise share one.
- **Builds**: `cargo build --release --features bench`. The model rows were measured on the sediment 0.5.1 code base plus the models of this change. The default model was re-run on sand 0.6.0 (R@1 49.5%, R@5 78.5%, MRR 62.1%, nDCG@5 59.1%), which is within the run-to-run spread below. The rename did not change the embedding or search code.
- **Run-to-run spread**: background consolidation runs concurrently with the queries, so repeated runs differ. Runs of all-MiniLM-L6-v2 gave R@1 49.5 to 50.0%, R@5 77.5 to 78.5% and MRR 61.8 to 62.1% (three runs). Differences below about 1 point are noise. One query is 0.5 points.
- **Quality** runs ran up to five at a time while other builds ran. Quality does not depend on load.
- **Latency** is a separate pass with the latency phase only (100 stores, then 50 recalls), one model at a time, started only when the 1-minute load average was below 2 and no build was running. Each model ran three times. The table shows the median p50 and the highest load average at the start and end of the three runs.
- **Peak RSS** is the highest `VmHWM` of the server process over all phases.
- **Correctness of the embeddings**: for every model, query and document vectors from Candle were compared with sentence-transformers 6.2.0 using the prompts and pooling from the model card. The lowest cosine similarity was 0.9968 (granite-embedding-small-english-r2), all others 0.999 or higher.
- Models marked "no" in the Offered column were measured with a scratch build and are not part of the `EmbeddingModel` enum. The ModernBERT models (granite-embedding-small-english-r2, granite-embedding-english-r2, gte-modernbert-base) need a ModernBERT encoder without the masked-LM head, which candle-transformers 0.8 does not export and candle 0.9 does. Candle 0.9 needs `half` 2.5 while lancedb 0.17 pins `half = 2.4.1`. The scratch build contains a copy of candle's `modernbert.rs` instead.

Candidates that were not measured: snowflake-arctic-embed-m-v2.0 (custom `GteModel` architecture, not in Candle), nomic-embed-text-v1.5 (`NomicBertModel`, not in candle-transformers 0.8), EmbeddingGemma-300m (Gemma 3 with bidirectional attention and two dense layers, not in candle-transformers 0.8, and the download requires accepting a licence on Hugging Face), jina-embeddings-v2-small-en (not attempted after the ModernBERT results, although Candle has `jina_bert`).

### Results in the sand pipeline

Recall p50 and Store p50 are medians of three latency-phase runs. Cells marked open are not measured yet: while other builds ran, the load average did not drop below 2.

| Model | Dim | Size (MB) | Recall@1 | Recall@5 | MRR | nDCG@5 | Recency@1 | Consolidation | Store p50 | Recall p50 | Load start/end | Peak RSS (MiB) | Offered |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| all-MiniLM-L6-v2 | 384 | 91 | 49.5% | 78.0% | 61.8% | 58.8% | 100% | 99% | open | open | - | 335 | default |
| e5-small-v2 | 384 | 133 | 42.0% | 70.5% | 53.5% | 51.8% | 98% | 99% | open | open | - | 426 | yes |
| bge-small-en-v1.5 | 384 | 133 | 46.5% | 75.0% | 58.4% | 55.5% | 100% | 99% | open | open | - | 413 | yes |
| bge-base-en-v1.5 | 768 | 438 | 50.0% | 75.5% | 61.4% | 57.0% | 100% | 99% | open | open | - | 861 | yes |
| all-MiniLM-L12-v2 | 384 | 133 | 49.0% | 81.0% | 62.7% | 60.9% | 100% | 99% | open | open | - | 392 | added |
| snowflake-arctic-embed-m-v1.5 | 768 | 436 | 50.0% | 77.5% | 61.8% | 58.1% | 100% | 99% | open | open | - | 859 | added |
| snowflake-arctic-embed-xs | 384 | 90 | 43.5% | 68.5% | 55.0% | 51.7% | 100% | 99% | open | open | - | 361 | no |
| snowflake-arctic-embed-s | 384 | 133 | 45.5% | 71.5% | 57.3% | 54.0% | 100% | 99% | open | open | - | 388 | no |
| snowflake-arctic-embed-m | 768 | 436 | 48.0% | 73.5% | 59.2% | 56.0% | 100% | 99% | open | open | - | 858 | no |
| gte-small | 384 | 67 | 48.0% | 73.0% | 58.4% | 54.8% | 100% | 99% | open | open | - | 417 | no |
| gte-base | 768 | 219 | 42.5% | 70.0% | 54.9% | 52.2% | 100% | 99% | open | open | - | 704 | no |
| mxbai-embed-xsmall-v1 | 384 | 48 | 48.5% | 77.5% | 61.2% | 57.7% | 100% | 99% | open | open | - | 373 | no |
| multilingual-e5-small | 384 | 471 | 40.5% | 67.5% | 51.4% | 48.9% | 100% | 99% | open | open | - | 1196 | no |
| granite-embedding-small-english-r2 | 384 | 95 | 49.5% | 73.5% | 60.0% | 55.8% | 100% | 99% | open | open | - | 467 | no (ModernBERT) |
| granite-embedding-english-r2 | 768 | 298 | 49.0% | 73.0% | 59.7% | 56.8% | 100% | 99% | open | open | - | 903 | no (ModernBERT) |
| gte-modernbert-base | 768 | 298 | 47.0% | 79.0% | 60.8% | 57.1% | 100% | 99% | open | open | - | 903 | no (ModernBERT) |

### Embedding quality without the pipeline

The same 1,000 memories and 200 queries ranked by cosine similarity alone (documents embedded with the document prefix, queries with the query prefix, nothing else). This measures the embedding model by itself.

| Model | Recall@1 | Recall@5 | MRR | nDCG@5 |
|---|---|---|---|---|
| all-MiniLM-L6-v2 | 47.0% | 79.0% | 61.0% | 59.8% |
| e5-small-v2 | 43.0% | 77.0% | 57.9% | 56.9% |
| bge-small-en-v1.5 | 51.5% | 77.5% | 62.4% | 58.8% |
| bge-base-en-v1.5 | 52.0% | 86.0% | 66.2% | 63.9% |
| all-MiniLM-L12-v2 | 51.0% | 82.0% | 63.9% | 62.1% |
| snowflake-arctic-embed-m-v1.5 | 59.5% | 89.5% | 72.2% | 68.9% |
| snowflake-arctic-embed-xs | 50.0% | 79.0% | 62.3% | 59.5% |
| snowflake-arctic-embed-s | 52.0% | 78.5% | 63.3% | 60.7% |
| snowflake-arctic-embed-m | 59.0% | 87.5% | 71.2% | 68.3% |
| gte-small | 51.0% | 81.5% | 64.1% | 61.0% |
| gte-base | 53.5% | 86.5% | 67.9% | 65.0% |
| mxbai-embed-xsmall-v1 | 47.0% | 76.5% | 60.6% | 58.1% |
| multilingual-e5-small | 42.0% | 71.0% | 54.4% | 51.4% |
| granite-embedding-small-english-r2 | 56.5% | 86.5% | 69.0% | 65.9% |
| granite-embedding-english-r2 | 56.5% | 88.0% | 69.6% | 67.2% |
| gte-modernbert-base | 56.5% | 87.5% | 69.8% | 65.9% |

### Pipeline effects on model differences

On cosine similarity alone, snowflake-arctic-embed-m-v1.5 scores above all-MiniLM-L6-v2 by 12.5 points R@1 and 11.2 points MRR, and the 384-dim granite-embedding-small-english-r2 by 9.5 and 8.0. In the sand pipeline every model ends up between 40% and 50% R@1. Retrieval-only runs with one part of the pipeline switched off (bench build; decay scoring was switched off in a scratch build):

| Model | Pipeline R@1 / R@5 / MRR | BM25 boost off (`SEDIMENT_FTS_BOOST_MAX=0`) | Decay scoring off |
|---|---|---|---|
| all-MiniLM-L6-v2 | 49.5 / 78.0 / 61.8 | 42.0 / 72.0 / 55.8 | 52.5 / 81.0 / 64.7 |
| bge-base-en-v1.5 | 50.0 / 75.5 / 61.4 | 41.5 / 73.5 / 53.9 | 54.0 / 85.0 / 65.8 |
| snowflake-arctic-embed-m-v1.5 | 50.0 / 77.5 / 61.8 | 39.0 / 70.5 / 53.5 | 54.0 / 83.0 / 66.1 |
| granite-embedding-small-english-r2 | 49.5 / 73.5 / 60.0 | 35.5 / 69.0 / 51.0 | 56.0 / 83.5 / 67.0 |

- **Decay scoring** multiplies every result by `1 + 0.1 * ln(1 + access_count)`, and every recall records an access for all returned items. Over 200 queries, items that were returned before rank higher in later, unrelated queries. Switching it off raises every model by 3 to 6.5 points R@1; the newer models then score above all-MiniLM-L6-v2 by 1.5 to 3.5 points R@1 instead of the 5 to 12.5 points they lead by on cosine similarity.
- **BM25 boost** helps every model: switching it off lowers R@1 by 7.5 to 14 points. Part of the queries are keyword lists ("Sidekiq asynchronous job processing email sending"), which favours lexical matching. The boost constants (`FTS_BOOST_MAX` 0.12, `FTS_GAMMA` 2.0) were tuned on all-MiniLM-L6-v2 and are added to `1 / (1 + d²)` of the vector distance, a scale that differs per model. They were not re-tuned per model here.
- **Consolidation** merges items above a fixed similarity during the query phase. After storing the 1,000 memories and running the 200 queries, 988 items were left with all-MiniLM-L6-v2, 970 with snowflake-arctic-embed-m-v1.5 and 934 with gte-small. A merged ground-truth memory can no longer be found by its content.
- **bge pooling**: the bge model cards specify CLS pooling, and sand mean-pools them (cosine 0.91 against sentence-transformers). With CLS pooling bge-small-en-v1.5 scored 49.0 / 72.5 / 59.0 and bge-base-en-v1.5 48.0 / 78.0 / 60.7 in the pipeline, which is no clear gain, and the change would make existing bge stores inconsistent, so mean pooling stays.

### German check

20 German notes written for this check, stored together with the first 300 English memories of the dataset. Each note has one German query (de→de) and one English query (en→de). One query is 5 points. The check runs through the full pipeline; BM25 matches German words directly, which helps every model on de→de.

| Model | de→de Recall@1 | de→de Recall@5 | de→de MRR | en→de Recall@1 | en→de Recall@5 | en→de MRR | Multilingual |
|---|---|---|---|---|---|---|---|
| all-MiniLM-L6-v2 | 65% | 85% | 74% | 60% | 60% | 60% | no |
| e5-small-v2 | 65% | 85% | 74% | 70% | 80% | 74% | no |
| bge-small-en-v1.5 | 65% | 90% | 75% | 65% | 80% | 72% | no |
| bge-base-en-v1.5 | 65% | 85% | 75% | 80% | 90% | 85% | no |
| all-MiniLM-L12-v2 | 80% | 90% | 84% | 55% | 65% | 60% | no |
| snowflake-arctic-embed-m-v1.5 | 60% | 85% | 73% | 75% | 90% | 82% | no |
| snowflake-arctic-embed-xs | 65% | 85% | 74% | 65% | 85% | 72% | no |
| snowflake-arctic-embed-s | 75% | 85% | 80% | 85% | 90% | 88% | no |
| snowflake-arctic-embed-m | 60% | 85% | 74% | 80% | 95% | 88% | no |
| gte-small | 70% | 90% | 79% | 70% | 75% | 72% | no |
| gte-base | 75% | 85% | 81% | 85% | 90% | 88% | no |
| mxbai-embed-xsmall-v1 | 65% | 85% | 74% | 55% | 60% | 57% | no |
| multilingual-e5-small | 70% | 95% | 81% | 60% | 65% | 62% | yes (~100 languages) |
| granite-embedding-small-english-r2 | 60% | 80% | 70% | 75% | 90% | 82% | no |
| granite-embedding-english-r2 | 70% | 85% | 78% | 95% | 100% | 98% | no |
| gte-modernbert-base | 75% | 85% | 80% | 95% | 100% | 98% | no |

The only multilingual model, multilingual-e5-small, is not better on German than the English models here and is the weakest on the English benchmark. The ModernBERT models and bge-base-en-v1.5 handle English queries against German notes best (en→de).

### Compared with upstream's numbers

| Model | Upstream R@1 / R@5 / MRR / nDCG@5 (M3 Max) | This run |
|---|---|---|
| all-MiniLM-L6-v2 | 50.0 / 77.5 / 61.9 / 58.7 | 49.5 / 78.0 / 61.8 / 58.8 |
| e5-small-v2 | 42.0 / 68.0 / 53.5 / 51.1 | 42.0 / 70.5 / 53.5 / 51.8 |
| bge-small-en-v1.5 | 46.5 / 75.0 / 58.4 / 55.5 | 46.5 / 75.0 / 58.4 / 55.5 |
| bge-base-en-v1.5 | 50.5 / 76.0 / 62.0 / 57.6 | 50.0 / 75.5 / 61.4 / 57.0 |

Quality matches within the run-to-run spread, except e5-small-v2 R@5 (+2.5 points, 5 queries). e5-small-v2 also missed one of the 50 temporal sequences (Recency@1 98%). Upstream reports 100% for all models. Latency is not comparable: upstream measured on an Apple M3 Max, this table on an x86 CPU with Candle's CPU backend.

### Conclusion

all-MiniLM-L6-v2 stays the default. In the sand pipeline no model is clearly better: the best alternatives (all-MiniLM-L12-v2, snowflake-arctic-embed-m-v1.5, bge-base-en-v1.5) are within 1 to 3 points of it and cost more time and memory. The embedding quality of newer models is real (up to +12.5 points R@1 on cosine similarity alone), but the decay scoring, the BM25 constants and the consolidation threshold take most of it away. A different default would help only after those are made model-aware.

all-MiniLM-L12-v2 and snowflake-arctic-embed-m-v1.5 were added to `SAND_EMBEDDING_MODEL` as options; the other candidates were left out because they scored below the default in the pipeline, or need a ModernBERT encoder that the current Candle version does not export.

## Category breakdown

### Recall@5 by category

| Category | Sediment | ChromaDB | Mem0 |
|----------|----------|----------|------|
| `architecture` | **82.9%** | 71.4% | 71.4% |
| `code_patterns` | **88.6%** | **88.6%** | **88.6%** |
| `cross_project` | 65.6% | **68.8%** | **68.8%** |
| `project_facts` | 60.6% | **75.8%** | **75.8%** |
| `troubleshooting` | 78.1% | **81.2%** | **81.2%** |
| `user_preferences` | **87.9%** | 84.9% | 84.9% |

### MRR by category

| Category | Sediment | ChromaDB | Mem0 |
|----------|----------|----------|------|
| `architecture` | **66.8%** | 55.8% | 55.8% |
| `code_patterns` | 70.4% | **71.1%** | **71.1%** |
| `cross_project` | **50.7%** | 47.1% | 47.1% |
| `project_facts` | 51.6% | **59.2%** | **59.2%** |
| `troubleshooting` | **63.2%** | 62.8% | 62.8% |
| `user_preferences` | 67.6% | **67.9%** | **67.9%** |

## Temporal correctness

| Metric | Sediment | ChromaDB | Mem0 |
|--------|----------|----------|------|
| Recency@1 | **100%** | 14% | 14% |
| Recency@3 | **100%** | 94% | 94% |
| MRR | **100%** | 48.8% | 48.8% |
| Mean Rank | **1.00** | 2.38 | 2.38 |

## Latency

### Store latency

| Metric | Sediment | ChromaDB | Mem0 |
|--------|----------|----------|------|
| p50 | 50ms | 692ms | **14ms** |
| p95 | 63ms | 726ms | **19ms** |
| p99 | 65ms | 729ms | **20ms** |

### Recall latency

| Metric | Sediment | ChromaDB | Mem0 |
|--------|----------|----------|------|
| p50 | 103ms | 694ms | **8ms** |
| p95 | 110ms | 728ms | **12ms** |
| p99 | 124ms | 746ms | **12ms** |

## Methodology

- **Dataset**: 1,000 memories across 6 categories (architecture, code patterns, project facts, troubleshooting, user preferences, cross-project)
- **Queries**: 200 queries with known ground-truth expected results
- **Temporal**: 50 sequences testing whether updated information ranks above stale versions
- **Dedup**: 50 pairs of near-duplicate content testing consolidation
- **Systems tested**: Sediment (hybrid vector + BM25, local Candle embeddings), ChromaDB (default ONNX embeddings), Mem0 (local Qdrant + HuggingFace), MCP Memory Service (ChromaDB-based), MCP Server Memory (entity graph), Basic Memory (markdown file-based)
