# TODO

## Completed

### Body output edit-friendly
- `body` now outputs raw source code without line numbers
- Header line shows relative path + line range: `# src/tools/edit.rs L10-L50`
- `definition` is now distinct: shows location + signature + docstring (not full body)

### Grep command with symbol-aware output
- `grep` now searches symbol index first, then text
- Output: "Symbol matches" section (with kind, container, language) then "Text matches"
- Supports quoted patterns: `grep "fn main"`

### Better help / usage documentation for DSL
- Updated tool description with all commands including callers/callees/grep/search/read
- Added CD SCOPING section explaining how scoping works
- Added quoting examples
- Documented Parent.child syntax

### Quote/escape support in DSL
- Supports `"double"` and `'single'` quotes for args with spaces
- Quote-aware `|` pipe splitting and `#` comment stripping
- `unquote()` utility for handlers

### Symbol index persistence
- Saves to `.cache/programmer-mcp/{lang}.symbols.json`
- Stores per-file mtimes; on restart only re-indexes changed files
- Instant symbol availability on restart (no waiting for LSP scan)

### Relative paths in all output
- All user-facing output now shows workspace-relative paths
- `LspClient.workspace_root()` accessor added
- `relative_to()` utility in formatting module

### cd warning
- Warns when `cd` is the only command (no persistence between execute calls)
- Warns when `cd` is the last command in a chain (no effect)

### Symbol result deduplication
- `find_symbol_with_fallback` deduplicates by name + URI + proximity (within 5 lines)
- Handles workspace/symbol vs documentSymbol returning same symbol at slightly different positions
- `exec_helpers` filters "not found" noise when other clients found the symbol
- Cross-client dedup by identical output text

### Definition signature extraction
- Skips leading attribute macros (#[tool(...)], #[derive(...)], @decorators)
- Shows actual function/struct declaration, not macro noise
- Doc comment extraction improved: no longer captures regular `//` comments

### SOURCE_EXTS consolidation
- Single source of truth in `lsp/detect_lang.rs`
- `tools/mod.rs` re-exports from there
- Extended with missing extensions (pyi, cxx, kts, php, dart, etc.)

### Compiler warnings fixed
- Removed unused imports (BoundedStore, Arc, path_to_uri)
- Removed dead code (add_line_numbers)

### Go doc comment support
- `extract_doc_lines` now language-aware: `//` comments are doc comments in Go/C/JS but NOT in Rust
- `.rs` files only use `///` as doc comments; other languages treat `//` above definitions as doc

### References position fix
- `textDocument/references` was failing because position pointed at doc comment, not identifier
- Added `find_identifier_position()` — scans forward from range start to find the actual identifier
- Applied to both `references` and `callers`/`callees` (call hierarchy)
- Shared utility in `formatting.rs`

### Multi-client noise filtering
- `is_not_found_msg()` properly classifies "No references found" / "X not found" messages
- Filters out "not found" noise from non-matching LSP clients when real results exist

### File size compliance
- Consolidated 7 repeated `handle_symbol_cmd` arms in `dispatch()` to a single arm
- Moved `detect_dir_language` from `dsl/ops/lsp.rs` to `lsp/detect_lang.rs` (thematic home)
- Moved `default_search_limit` to `serde_helpers.rs`
- Extracted parse tests to `parse_tests.rs`
- Reduced: dsl/mod.rs 328→281, dsl/ops/lsp.rs 333→288, parse.rs 334→266

### External path filtering
- `find_symbol_with_fallback` filters external results when workspace results exist
- Definition results capped at MAX_RESULTS = 10 with truncation message

## Remaining ideas

### Performance
- Consider binary format (bincode) for cache instead of JSON for faster load

### Usability
- Auto-complete for symbol names in DSL commands
- Show file language icon/badge in search results
- Better error messages when LSP is not ready
