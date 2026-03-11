# programmer-mcp Supported Functionality

## Supported Operations

The MCP server exposes a **single tool** called `execute` that accepts a batch of operations. All operations in a batch run in parallel.

### 1. Definition

**Purpose**: Find the source code definition of a symbol

**Parameters**:
- `symbolName` (required): Symbol to look up (e.g. `MyType`, `MyType.method`, `MyType::method`)
- `language` (optional): Target a specific LSP server by language name

**Behavior**:
1. Queries all LSPs (or specified one) via `workspace/symbol` for the symbol
2. Smart fallback: tries case variations (snake_case, camelCase, PascalCase, SCREAMING_SNAKE_CASE) if not found
3. Fuzzy fallback: searches similar file names, then similar symbols within those files (jaro_winkler > 0.8)
4. Gets full definition range via `textDocument/documentSymbol`
5. Returns source code with line numbers

**Example**:
```json
{
  "operation": "definition",
  "symbolName": "LspManager",
  "language": "rust"
}
```

### 2. References

**Purpose**: Find all references to a symbol across the workspace

**Parameters**:
- `symbolName` (required): Symbol to search for
- `language` (optional): Target specific LSP server

**Behavior**:
1. Uses same smart fallback as definition to locate symbol
2. Calls `textDocument/references` at symbol location
3. Groups references by file
4. Shows 5 lines of context around each reference
5. Returns formatted list with line numbers

**Example**:
```json
{
  "operation": "references",
  "symbolName": "execute_batch"
}
```

### 3. Diagnostics

**Purpose**: Get errors, warnings, and hints for a file

**Parameters**:
- `filePath` (required): Path to file
- `contextLines` (optional, default 5): Lines of context around each diagnostic
- `showLineNumbers` (optional, default true): Include line numbers
- `language` (optional): Target specific LSP (auto-detected from file extension if omitted)

**Behavior**:
1. Opens file in LSP if not already open
2. Waits 2 seconds for LSP to publish diagnostics
3. Retrieves cached diagnostics from `textDocument/publishDiagnostics`
4. Formats with severity, message, and context lines
5. Returns summary count + detailed list

**Example**:
```json
{
  "operation": "diagnostics",
  "filePath": "src/main.rs",
  "contextLines": 3,
  "showLineNumbers": true
}
```

### 4. Hover

**Purpose**: Get type information and documentation at a specific position

**Parameters**:
- `filePath` (required): Path to file
- `line` (required, 1-indexed): Line number
- `column` (required, 1-indexed): Column number
- `language` (optional): Target specific LSP

**Behavior**:
1. Opens file in LSP if not already open
2. Sends `textDocument/hover` at position
3. Formats hover contents (supports Scalar, Array, Markup)
4. Returns type signature and documentation

**Example**:
```json
{
  "operation": "hover",
  "filePath": "src/lsp/client.rs",
  "line": 50,
  "column": 10
}
```

### 5. RenameSymbol

**Purpose**: Rename a symbol across the entire project

**Parameters**:
- `filePath` (required): File containing the symbol
- `line` (required, 1-indexed): Line number
- `column` (required, 1-indexed): Column number
- `newName` (required): New name for the symbol
- `language` (optional): Target specific LSP

**Behavior**:
1. Opens file in LSP if not already open
2. Sends `textDocument/rename` at position with new name
3. Applies `WorkspaceEdit` changes to disk
4. Returns summary of files modified and change count

**Example**:
```json
{
  "operation": "rename_symbol",
  "filePath": "src/server.rs",
  "line": 20,
  "column": 15,
  "newName": "new_function_name"
}
```

## Batch Execution

All operations support batching for parallel execution:

```json
{
  "operations": [
    { "operation": "definition", "symbolName": "LspClient" },
    { "operation": "references", "symbolName": "execute_batch" },
    { "operation": "diagnostics", "filePath": "src/main.rs" }
  ]
}
```

**Behavior**:
- Each operation spawns a separate tokio task
- All operations run concurrently
- Results returned in same order as request
- Individual operation failures don't affect others

**Result Format**:
```
=== Operation 1 (definition) [OK] ===
[definition output]

=== Operation 2 (references) [OK] ===
[references output]

=== Operation 3 (diagnostics) [ERROR] ===
[error message]
```

## Multi-LSP Support

### Language Routing

Specify `language` parameter to route to a specific LSP server:
```json
{ "operation": "definition", "symbolName": "MyClass", "language": "python" }
```

### Auto-Detection

For file-based operations (diagnostics, hover, rename), language is auto-detected from file extension:
- `.rs` → `rust`
- `.py` → `python`
- `.go` → `go`
- `.ts`, `.tsx` → `typescript`
- `.js`, `.jsx` → `javascript`
- etc. (see `src/lsp/detect_lang.rs`)

### Broadcast Mode

For symbol-based operations (definition, references) without `language` specified, all LSP servers are queried and results merged.

## Smart Symbol Fallback

When a symbol is not found by exact name:

1. **Case Variations**: Automatically tries:
   - Original case
   - snake_case
   - camelCase
   - PascalCase
   - SCREAMING_SNAKE_CASE

2. **Fuzzy Matching**: If still not found:
   - Finds files with similar names (jaro_winkler similarity > 0.8)
   - Queries symbols in those files
   - Returns best match by symbol name similarity

3. **Qualified Name Handling**: Supports `Type.method` and `Type::method` syntax, filters workspace results to exact matches

## File Watching

The server automatically watches the workspace directory and:
- Sends `workspace/didChangeWatchedFiles` notifications to all LSPs
- Updates `textDocument/didChange` for files already open in LSPs
- Keeps LSP diagnostics in sync with file changes

## Not Supported

The following LSP features are **explicitly excluded**:
- **edit_file**: No code modification tool (only rename via LSP's own rename operation)
- **code actions**: No quick fixes or refactorings
- **completion**: No autocomplete
- **formatting**: No code formatting
- **go to type definition**: Only go to definition
- **signature help**: No function signature assistance

## Usage Example

Start the server:
```bash
programmer-mcp --workspace /path/to/project \
  --lsp rust:rust-analyzer \
  --lsp python:pyright \
  --lsp go:gopls
```

Test with MCP inspector:
```bash
npx @modelcontextprotocol/inspector cargo run -- \
  --workspace . \
  --lsp rust:rust-analyzer
```

Sample MCP `tools/call` request:
```json
{
  "name": "execute",
  "arguments": {
    "operations": [
      {
        "operation": "definition",
        "symbolName": "LspManager",
        "language": "rust"
      },
      {
        "operation": "diagnostics",
        "filePath": "src/main.rs",
        "contextLines": 5
      }
    ]
  }
}
```
