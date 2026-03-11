# MCP Language Server Project Overview

This is an **MCP server** (Model Context Protocol) that wraps language servers (LSP) and exposes their capabilities as tools for LLMs. It's essentially a bridge between MCP clients (like Claude Desktop) and Language Servers Protocol servers (gopls, rust-analyzer, pyright, etc.).

## Architecture

### Main Components

**`main.go`** - Entry point that:
- Parses CLI flags (`--workspace`, `--lsp`)
- Creates an MCP server and registers tools
- Handles parent process monitoring for graceful shutdown
- Manages LSP client lifecycle

**`tools.go`** - Tool registration for MCP (6 tools):
- `definition` - Get symbol definitions
- `references` - Find all symbol usages
- `diagnostics` - Get file diagnostics
- `hover` - Get hover information
- `rename_symbol` - Rename symbols across project
- `edit_file` - Apply multiple text edits (commented: codelens tools)

### Internal Packages

**`internal/lsp/`** - LSP client implementation:
- `client.go` - Main client managing language server process
- `methods.go` - Generated LSP method calls
- `transport.go` - JSON-RPC message protocol
- `protocol.go` - LSP protocol setup
- `server-request-handlers.go` - Handles server requests
- Manages file open/close, diagnostics caching, initialization

**`internal/tools/`** - Tool implementations:
- `definition.go` - Symbol lookup via workspace/symbol
- `references.go` - Find all references
- `edit_file.go` - Text edit application
- `diagnostics.go` - Diagnostic retrieval
- `hover.go` - Hover info
- `rename-symbol.go` - Symbol renaming
- `utilities.go` - Helper functions

**`internal/protocol/`** - LSP protocol types (generated from TypeScript):
- `tsprotocol.go` - LSP type definitions
- `interfaces.go` - Type handling workarounds
- `uri.go` - URI utilities
- `tables.go` - Kind tables

**`internal/logging/`** - Structured logging:
- Component-based logging (Core, LSP, Wire, Process, Watcher, Tools)
- Environment variable configuration (`LOG_LEVEL`)

**`internal/watcher/`** - File system monitoring for workspace changes

## Key Features

- Supports multiple language servers (Go, Rust, Python, TypeScript, C/C++)
- Graceful shutdown with parent process monitoring
- Diagnostic caching
- File change notifications
- Context-aware symbol operations
- MCP tool-based interface for LLMs

## Dependencies

- `github.com/mark3labs/mcp-go` - MCP server framework
- `github.com/fsnotify/fsnotify` - File system watching
- `golang.org/x/tools` - Go tooling
- Standard libraries for LSP communication

The project is well-structured with clear separation of concerns between MCP integration, LSP client management, and language-specific tool implementations.
