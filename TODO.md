# TODO

## Body output not edit-friendly
`body` output includes line number decoration (`61|async fn...`), making it
impossible to copy-paste into the MCP's own edit commands. The edit tools
exist but the workflow breaks when you need to read code first and then
edit it — the decorated output can't be used as-is.

Possible fix: provide a raw/undecorated output mode, or make the edit
commands accept line-numbered input and strip decoration automatically.

## Grep command with symbol-aware output
The MCP lacks a text-level search that works across files. LSP `references`
only finds symbol references, not arbitrary text patterns (import paths,
string literals, comments, etc.).

Need a `grep` command whose output is sorted by:
1. Found symbols (LSP-resolved matches first)
2. Plain text matches (fallback)

This gives structured, prioritized results rather than raw line dumps.

## Better help / usage documentation for the DSL
Results are noisy when the command line isn't used precisely — the tool
falls back to fuzzy/guessing searches across all LSPs. The fix is better
documentation and hints so the user (or AI agent) knows how to use the
tool correctly:
- Clear examples in the help text for each command
- Show how `cd` scoping works and when to use it
- Explain bracket syntax vs bare args
- Document language auto-detection behavior
