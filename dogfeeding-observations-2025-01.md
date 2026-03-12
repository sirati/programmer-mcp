# Dogfeeding Observations – January 2025

## Session: parent.child symbol resolution + tasks + nix

### What was implemented

- **Parent.child fuzzy resolution** (`src/tools/symbol_search.rs`):
  - `RelayChannel.relay` now resolves correctly via a document/symbol fallback
  - `Channel.ensure_initialized` (wrong parent) resolves correctly via fuzzy parent match
  - Strategy chain: workspace/symbol(child) + exact parent → workspace/symbol(child) + fuzzy parent → document/symbol on parent's file (exact/fuzzy) → child-only exact → child-only fuzzy

- **`find_containing_symbol_range` fix** (`src/tools/formatting.rs`):
  - Flat document symbol responses now return the *smallest* (most specific) range
    containing the query position, instead of the first/largest one.
  - Previously `body("RelayChannel.relay")` returned the entire `impl` block.

- **SSH `kill_on_drop`** (`src/remote/client.rs`):
  - Added `.kill_on_drop(true)` to the SSH tunnel child processes so they are
    cleaned up when the `Child` is dropped (i.e. on program exit).

- **Nix detection + LSP fallback** (`src/nix.rs`, `src/lsp/manager.rs`):
  - Detects if `nix` is in PATH and if flakes are enabled.
  - When an LSP command is not found, automatically retries with
    `nix run nixpkgs#{pkg} -- {args}`.
  - Known package mappings for ~30 common LSPs; unknown commands fall back to
    the binary name as the nixpkgs attribute.

- **Task management** (`src/background/task.rs`, `src/background/mod.rs`,
  `src/tools/mod.rs`, `src/server.rs`):
  - New operations: `set_task`, `update_task`, `add_subtask`, `complete_task`,
    `complete_subtask`, `list_tasks`, `list_subtasks`.
  - Tasks persisted to `.programmer-mcp/tasks/{name}.json`.
  - `list_tasks` / `list_subtasks` default to showing only pending items.

---

## Observations & issues noticed during dogfeeding

### 1. Concurrent set + list gives stale results (expected, but confusing)

When `set_task` and `list_tasks` are batched in the same `execute` call they
run concurrently. If `list_tasks` wins the mutex race it sees no tasks, even
though `set_task` succeeded in the same call.

**Impact**: Low – callers should always put mutating operations and their
dependent reads in separate calls (sequential calls), or understand that the
result is a snapshot at the time that operation ran.

**Recommendation**: The tool description could note which operations are
"write" vs "read" so the AI knows not to mix them in the same batch expecting
ordered results. Alternatively, consider a `then` sequencing primitive, but
that adds complexity.
instead: for commands that have an obvious ordering like list_tasks, and set_task the then is implied when the order of the commands issued is correct, if the order is unexpected issue a warning 

---

### 2. `list_symbols` operation – `filePath` duplication

The schema requires `filePath` both at the outer `execute` level *and* inside
each operation object for `list_symbols`. Only the per-operation `filePath` is
actually used; the outer one appears to be vestigial.

Example of the duplication:
```json
{
  "filePath": "src/tools/mod.rs",
  "operations": [
    {"filePath": "src/tools/mod.rs", "operation": "list_symbols"}
  ]
}
```

The outer `filePath` is unused by `execute_batch`. It should either be removed
from the schema entirely or used as a default for all operations that accept
`filePath`.

---

### 3. `body` on dotted parent.child returns full impl block (now fixed)

Before the `find_containing_symbol_range` fix, `body(["RelayChannel.relay"])`
returned the entire `impl RelayChannel<W, R> { … }` block. The flat document
symbol response contained both `impl RelayChannel<W, R>` (L22-103) and `relay`
(L31-55), and the old code returned the first symbol whose range *contained*
the query position rather than the smallest one.

Fixed by selecting the minimum-span symbol in the flat case.

---

### 4. Workspace/symbol misses short method names (known LSP limitation)

`workspace/symbol("relay")` returns `RelayChannel` (a struct) because "relay"
is a fuzzy substring match. The method named `relay` does not appear in results
at all – rust-analyzer appears to de-prioritize or omit exact short-method
matches when a struct name dominates.

The document/symbol fallback (step 3 in the chain) resolves this correctly.
Worth keeping in mind for future LSP integrations.

---

### 5. Remote-localhost and local debug-mcp behave identically

Both connections were tested for the parent.child resolution and produce
identical results. No behavioural differences observed.

---

### 6. Tool description could make sequencing requirement clearer

The current description says "ALWAYS batch ALL related operations into a single
call". For *dependent* operations (e.g. set then list), the call must be split
into separate sequential invocations. The instructions should distinguish
between *independent* operations (batch together) and *dependent* ones
(sequential calls required).
