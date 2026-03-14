# Next TODOs

## 1. Symbol Cache Persistence
Persist the symbol index to `.programmer-mcp/.cache/` so it survives restarts.

- On startup, load cached symbols from disk (skip files whose content hash hasn't changed)
- On file change / re-index, update the cache entry
- Cache format: parallel folder structure under `.cache/symbols/`, one file per source file
- Each cache file stores: content hash + list of indexed symbols (name, kind, range, container)
- Only re-index files whose hash differs from cached hash
- `.cache` files should store last-modified time so we don't need to hash unchanged files

## 2. Diagnostics Formatting Improvement
Make auto-diagnostics output more concise and readable.

Current format is verbose with repeated absolute paths and noise like `#[warn(unused_imports)]` annotations.

Target format:
```
New diagnostics based on recent edits:
cd src/lsp/client
2 new warnings for mod.rs:
  use of deprecated field:
    L119:13 `InitializeParams::root_uri`: Use `workspace_folders` instead
    L120:13 `InitializeParams::root_path`: Use `root_uri` instead
  unused import:
    L164:25 `futures::StreamExt`
```

Requirements:
- Absolute paths always converted to relative paths (based on project root)
- Use `cd` to group files in the same directory
- Group diagnostics by severity (error > warning > hint)
- Within each severity, group by diagnostic type/category
- Remove noise from messages (e.g. `#[warn(unused_imports)]` annotations)
- Sort locations within a group by line number
- If same warning appears multiple times, group the locations
- Only show NEW diagnostics (not ones already reported)

## 3. Local LLM Integration (Ouro 2.6B-Thinking via vLLM)
Add an optional local LLM sidecar for intelligent post-processing, using
[Ouro-2.6B-Thinking](https://huggingface.co/ByteDance/Ouro-2.6B-Thinking) served by vLLM.

Ouro uses a Looped Language Model architecture with iterative shared-weight computation,
matching up to 12B SOTA models at 2.6B params. Runs on consumer GPUs (~8GB VRAM).

### Architecture
- vLLM server runs as a sidecar (configurable endpoint, e.g. `http://localhost:8000`)
- All LLM calls are **batched and parallel** — queue many tasks at once so individual
  latency (~500ms) is irrelevant; the small model size allows very large batch sizes
- Config flag to enable/disable; all features have a deterministic fallback when LLM is off
- Async HTTP client (reqwest) sends batched requests to vLLM's OpenAI-compatible API

### Use Cases (post-processing only, never on critical path)
- **Diagnostics cleanup**: strip noise, group related warnings, produce concise summaries
- **Output formatting**: condense verbose multi-file LSP results into readable output
- **Symbol disambiguation**: when fuzzy matching returns multiple candidates, pick the
  most contextually relevant one (given surrounding code / recent commands)
- **Diff summarization**: after edits, produce a natural-language summary of changes
- **Intelligent classification**: replace heuristic `is_not_found_msg` with robust classifier
- **Semantic deduplication**: group diagnostics/results with different wording but same meaning

### Implementation Notes
- Create `src/llm/` module with: client, batching queue, prompt templates
- Prompt templates should be small and focused (one task per template)
- Response parsing should be strict — fall back to deterministic path on any parse failure
- Rate limiting / queue depth config to avoid overwhelming the GPU
