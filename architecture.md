# programmer-mcp Architecture

## Overview

Multi-LSP MCP server written in idiomatic async Rust. Wraps multiple language servers simultaneously, exposing a unified batched execution interface via the Model Context Protocol.

## Key Differentiators from Go Reference

1. **Multi-LSP**: Connects to N language servers keyed by language name; routes by language or broadcasts to all
2. **Single batched tool**: One `execute` tool accepting array of operations (vs 5 separate tools in Go ref)
3. **Parallel execution**: All operations in a batch run concurrently via `tokio::spawn`
4. **Smart symbol fallback**: Case variations (camelCase, snake_case, etc.) + fuzzy file/symbol matching

## Directory Structure

```
src/
  main.rs              -- Entry point: CLI parsing, manager start, watcher spawn, MCP server on stdio
  config.rs            -- CLI args with clap: --workspace, --lsp (repeatable)
  server.rs            -- MCP ServerHandler with #[tool_router], single `execute` tool
  
  lsp/
    mod.rs             -- Re-exports
    client.rs          -- LspClient: spawn LSP process, initialize, track open files, cache diagnostics
    transport.rs       -- JSON-RPC stdio transport for jsonrpsee (Content-Length framing)
    manager.rs         -- LspManager: HashMap<String, Arc<LspClient>>, routes by language
    detect_lang.rs     -- File extension → language ID mapping
  
  tools/
    mod.rs             -- Operation enum (tagged union), execute_batch, execute_on_clients/first
    definition.rs      -- workspace/symbol + textDocument/documentSymbol for full definition
    references.rs      -- textDocument/references with context lines
    diagnostics.rs     -- publishDiagnostics cache + context formatting
    hover.rs           -- textDocument/hover with markup formatting
    rename.rs          -- textDocument/rename with WorkspaceEdit application
    symbol_search.rs   -- Smart fallback: case variations, fuzzy matching (strsim)
    formatting.rs      -- Shared utilities: path_to_uri, uri_to_path, line numbers, ranges
  
  watcher.rs           -- notify-based file watcher, sends didChangeWatchedFiles
```

## Data Flow

1. **Startup**: `main.rs` parses CLI → `LspManager::start()` spawns all LSPs → file watcher spawns → MCP server listens on stdio
2. **Request**: MCP client sends `tools/call` with `execute` tool and `ExecuteRequest { operations: [...] }`
3. **Routing**: Each operation specifies optional `language` → `LspManager::resolve()` returns matching client(s)
4. **Execution**: `execute_batch()` spawns tokio tasks for all ops → each calls appropriate tool module → results merged
5. **Response**: `OperationResult` array formatted and returned via `CallToolResult`

## Key Components

### LspClient (lsp/client.rs)

- Wraps `jsonrpsee::Client` over stdio transport
- Tracks open files: `HashMap<String, OpenFileInfo>` with version numbers
- Caches diagnostics: `HashMap<String, Vec<Diagnostic>>` from `textDocument/publishDiagnostics` notifications
- Methods: `workspace_symbol`, `document_symbol`, `references`, `hover`, `rename`, `get_cached_diagnostics`
- Handles LSP requests: `workspace/applyEdit`, `workspace/configuration`, `client/registerCapability`

### LspManager (lsp/manager.rs)

- Multi-LSP coordinator: `HashMap<String, Arc<LspClient>>` keyed by language name
- `resolve(language, file_path)` returns `Vec<&Arc<LspClient>>`:
  - If `language` specified → that client
  - If `file_path` specified → detect language from extension
  - Otherwise → all clients (for symbol-based ops)

### Operation Enum (tools/mod.rs)

Tagged enum with variants:
- `Definition { symbol_name, language }`
- `References { symbol_name, language }`
- `Diagnostics { file_path, context_lines, show_line_numbers, language }`
- `Hover { file_path, line, column, language }`
- `RenameSymbol { file_path, line, column, new_name, language }`

Execution:
- **Symbol ops** (definition, references): `execute_on_clients()` → query all, merge results
- **File ops** (diagnostics, hover, rename): `execute_on_first()` → use first matching client

### Smart Symbol Search (tools/symbol_search.rs)

`find_symbol_with_fallback()`:
1. Try exact match via `workspace/symbol`
2. Generate case variations (heck crate): snake_case, camelCase, PascalCase, SCREAMING_SNAKE_CASE
3. Try each variation
4. If still not found: fuzzy match file names (jaro_winkler > 0.8) → `textDocument/documentSymbol` on similar files

## Dependencies (crates.io)

- `rmcp` 1.2 — MCP SDK with `#[tool]`, `#[tool_handler]`, `#[tool_router]` macros
- `lsp-types` 0.97 — LSP protocol types (uses `Uri` not `Url`)
- `jsonrpsee` 0.24 — JSON-RPC async client
- `tokio` 1 — async runtime
- `serde`, `serde_json` — serialization
- `thiserror` 2 — error types
- `tracing`, `tracing-subscriber` — logging
- `notify` 7 — file watching
- `strsim` 0.11 — string similarity
- `heck` 0.5 — case conversion
- `clap` 4 — CLI parsing
- `anyhow` 1 — error handling
- `futures`, `async-trait` — async utilities

## Critical Implementation Details

### Uri vs Url

`lsp-types` 0.97 uses `Uri` (newtype around `fluent_uri::Uri<String>`) which lacks `from_file_path()`/`to_file_path()`. We implement helpers in `tools/formatting.rs`:
- `path_to_uri(path) -> Result<Uri, String>` — manual `file://` construction
- `uri_to_path(uri) -> Option<String>` — strip `file://` prefix

### schemars Re-export

`rmcp::schemars::JsonSchema` derive needs `use rmcp::schemars;` at module level for macro expansion (crate must be importable).

### File Watching

`notify` 7.x watches workspace recursively → on changes, sends `workspace/didChangeWatchedFiles` to all LSPs + updates `textDocument/didChange` for open files.

### Diagnostics Caching

LSP servers push diagnostics via `textDocument/publishDiagnostics` notifications. `LspClient` subscribes to these and maintains a `HashMap<String, Vec<Diagnostic>>` cache. The `diagnostics` tool opens the file (if not already open), waits 2 seconds for initial diagnostics, then returns cached results.

### Error Handling

Tools return `Result<String, LspClientError>`. Failed operations are logged at debug level but don't fail the batch. `execute_on_clients` merges all successful results; `execute_on_first` returns first success or last error.

## Testing Strategy

1. Build: `cargo check`
2. Run with MCP inspector: `npx @modelcontextprotocol/inspector cargo run -- --workspace . --lsp rust:rust-analyzer`
3. Test single ops: definition, references, diagnostics, hover, rename
4. Test batch ops: verify parallel execution
5. Test multi-LSP: configure multiple servers, verify routing
6. Test fallback: query with wrong case, verify case variation works
