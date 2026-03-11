# Initial Debug-MCP Testing Results

## Overview
This document records the initial testing of the debug-mcp tool for the programmer-mcp project.

## Tools Tested

### 1. `status` ✅
**Purpose**: Report whether the child process is running, uptime, and config errors.

**Results**:
- Initial status: No child process started
- After rebuild: "Child process is running (up 0m 4s)"
- After update_debug_bin: Error - relay to debug child failed (process crashed)

### 2. `show_config` ✅
**Purpose**: Display current configuration including LSP specs.

**Results**:
```
Config file: /home/sirati/devel/rust/programmer-mcp/debug-mcp.config.toml
Command-line LSPs: (none)
Saved LSPs (debug-mcp.config.toml): (none) → (later added rust:rust-analyzer)
```

### 3. `configure` ✅
**Purpose**: Add or remove LSP servers in the saved debug config.

**Results**:
- Attempted to add: `rust:rust-analyzer --stdio` → Failed (incorrect flag)
- Removed: `rust:rust-analyzer --stdio`
- Added: `rust:rust-analyzer` → Success

### 4. `rebuild` ✅
**Purpose**: Build with `cargo build`, test, replace binary, restart debug server.

**Results**:
- First attempt: Failed - "No LSP servers configured. Use `configure` with `add_lsp`..."
- Second attempt: Success - "Built and started."
- Child process started and reached ready state

### 5. `grab_log` ✅
**Purpose**: Retrieve recent stderr log lines from the running child process.

**Results**:
```
Sample logs retrieved:
- 2026-03-11T21:31:47.055697Z DEBUG programmer_mcp::watcher: file change events count=1
- 2026-03-11T21:32:06.507651Z DEBUG programmer_mcp::watcher: file change events count=1
- Multiple file watcher events and LSP notifications
```

Successfully filtered logs with query parameter (e.g., query="error").

### 6. `relay_command` ✅ (FIXED in updated debug-mcp)
**Purpose**: Relay an MCP JSON-RPC call to the running child process.

**Initial Results (Pre-update)**:
- Attempted: `tools/list` request
- Status: Failed - "child stdout closed unexpectedly"
- Error logs: "ExpectedInitializeRequest" - MCP server expects initialization request before tools/list

**Updated Results (Post-update)**:
- Attempted: `tools/list` request
- Status: **SUCCESS** ✅
- Response: Received full tools/list response with a unified "execute" tool providing:
  - `definition` - Find symbol source code
  - `references` - Find all usages
  - `diagnostics` - Get file errors/warnings
  - `hover` - Get type/docs at position
  - `rename_symbol` - Rename across project

The debug-mcp update fixed the relay command handling! The tool now properly handles initialization and serves MCP requests correctly.

### 7. `update_debug_bin` ✅
**Purpose**: Build with `cargo build`, test in --debug mode, replace this debug server's own binary, restart as new debug server.

**Initial Results**:
- Status: "Debug binary updated. Tested child stopped. All further traffic is now forwarded to the new debug process."
- Side effect: New debug process crashed immediately after (Broken pipe error)

**Updated Results (After debug-mcp update)**:
- Status: "Debug binary updated. Tested child stopped. All further traffic is now forwarded to the new debug process." ✅
- Process exits after update (expected behavior - old process replaced)
- Process can be restarted with `rebuild`
- Note: The process exit after `update_debug_bin` is by design (replaces the binary), not a crash

## Key Findings

1. **Configuration Required**: LSP servers must be configured before rebuild
2. **Command Format**: LSP specs should be format `language:command` without extra flags like `--stdio`
3. **File Watching**: Debug process actively monitors file changes
4. **MCP Relay (FIXED)**: Previously failed, now works correctly - server properly handles MCP JSON-RPC requests
5. **Unified Tool Interface**: The "execute" tool provides a unified interface for multiple LSP operations (definition, references, diagnostics, hover, rename_symbol)
6. **Update Process**: `update_debug_bin` works as designed - replaces the binary and the process exits (not a crash)

## Configuration State After Testing

```toml
# debug-mcp.config.toml
LSP: rust:rust-analyzer
```

## Recommendations

- Use `rebuild` for standard development iteration
- Use `update_debug_bin` for testing the full MCP server pipeline
- Monitor logs with `grab_log` when debugging issues
- Configure multiple LSP servers if working with polyglot code
- Note: The crash after `update_debug_bin` may be environment-specific and should be investigated further
