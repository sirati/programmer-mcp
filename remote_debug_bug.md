# Remote Debug Bug: `--debug --remote localhost` Fails in Zed / Claude MCP

## Summary

`--debug` (direct mode) works perfectly in Zed / Claude MCP.  
`--debug --remote localhost` always fails — Zed reports "context server request timeout",
`claude mcp list` reports "Failed to connect" after ~60 seconds.

**Both are generic error labels** — they fire for any MCP failure, not just actual timeouts.

### Key confirmed fact
SSH tunnelling works and the session IS established in ~1 second (confirmed via stderr logs):
```
INFO  found remote control socket socket=~/.local/share/programmer-mcp/debug-mcp.sock
DEBUG starting SSH tunnel (ctrl)
INFO  session established session=19cdfc614fa
DEBUG starting SSH tunnel (session)
INFO  initial connection ready          ← SSH fully up at ~t+1s
ERROR remote proxy serve error: ConnectionClosed("initialize request")
```
The `ConnectionClosed` error in that run was an **artifact of stdin being closed** (process
was backgrounded with `&`). It is NOT the real failure mode when launched by an MCP client.

The real failure causes a ~60 s wait before `claude mcp list` gives up, which matches
either the `wait_for_setup` 60 s timeout or the relay's `INIT_TIMEOUT_SECS=10` +
`RELAY_TIMEOUT_SECS=30` + 30-attempt reconnect loop adding up.

---

## The Two Code Paths

### Working: `--debug`
`main.rs` → `run_debug_server(config)`:
1. Creates `DebugServer`
2. Starts `RemoteListener` on `~/.local/share/programmer-mcp/debug-mcp.sock`
3. Calls `server.serve(stdio())` — serves MCP **directly** over the process stdio to Zed

### Failing: `--debug --remote localhost`
`main.rs` → `run_remote_client(config)` (because `config.remote.is_some()` is checked first):
1. Spawns SSH setup task in background:
   - `ssh_command(spec, "echo -n ~/.local/share/programmer-mcp")` — runs `ssh localhost '...'`
   - With `debug_mode=true`, constructs path `<socket_dir>/debug-mcp.sock`
   - `ConnectionParams::connect()`:
     a. `start_ssh_forward` → `ssh -N -L /tmp/xxx/ctrl.sock:<remote_ctrl_socket> localhost`
     b. `wait_for_socket(ctrl.sock)` — up to 30s
     c. `establish_session` — sends `SESSION <id>`, gets back `OK <id> /tmp/programmer-mcp-sessions/<id>.sock`
     d. Kills control SSH
     e. `start_ssh_forward` → `ssh -N -L /tmp/xxx/sess.sock:/tmp/.../sess.sock localhost`
     f. `wait_for_socket(sess.sock)` — up to 30s
     g. `UnixStream::connect(sess.sock)` → creates `RelayChannel`
2. Creates `RemoteProxyServer` and calls `proxy.serve(stdio())` — serves MCP over stdio to Zed
3. Zed's `initialize` → answered immediately by `get_info()` (no SSH needed, static)
4. Zed's `tools/list` → `list_tools` → `relay_with_reconnect` → `wait_for_setup` (up to 60s) → relay to debug server

---

## What Is Known

- The issue is **NOT** about `nix develop` startup time or SSH connection timing per se.
- Zed's error is its generic failure response for any MCP brokenness.
- No special-casing of `localhost` vs real remote hosts is acceptable — localhost is just a test case for the general remote feature.
- The `relay.rs` framing **is compatible** with rmcp's async_rw transport: both use newline-delimited JSON (verified in rmcp-1.2.0 source, `JsonRpcMessageCodec` encodes as `json + \n`, decodes by finding `\n`).
- `RemoteProxyServer::get_info()` is synchronous and returns immediately, so `initialize` works even before SSH is ready.
- `DebugServer::list_tools` when `proxy_mode=false` (fresh start) just returns `self.tool_router.list_all()` — no child process involved.

---

## Current Diagnostic Approach

Tracing has been added at every significant point in the relay flow (`src/relay.rs` and
`src/remote/client.rs`).  In addition, `src/main.rs` now writes a **fresh timestamped log
file** for every remote-proxy invocation so stderr is captured even when the process is a
silent subprocess:

```
~/.local/share/programmer-mcp/logs/remote-<host>-<timestamp_ms>.log
```

**To reproduce and capture logs:**
1. Ensure debug server is running (`--debug --workspace ./`).
2. Copy the freshly built binary to `/home/sirati/programmer-mcp`.
3. `cd ~/tmpclaude && claude mcp remove debug-mcp-remote-test 2>/dev/null; claude mcp add debug-mcp-remote-test -- /home/sirati/programmer-mcp --debug --remote localhost`
4. `claude mcp list`  (will time out after ~60 s — that is expected)
5. `cat ~/.local/share/programmer-mcp/logs/remote-localhost-*.log | tail -80`

The log will show exactly which `relay:` tracing line is the last one printed, identifying
the hang point.

---

## What Has Been Ruled Out

1. **Framing mismatch between relay and rmcp transport**: Both use newline-delimited JSON. Not the issue.
2. **Protocol version mismatch**: relay sends `"2025-06-18"`, rmcp echoes whatever version it receives. `read_matching_response` doesn't check version, only matches `id`. Not the issue.
3. **Concurrent request contention**: `relay_with_reconnect` holds `self.conn` Mutex, serialising requests. Zed's `tools/list` would just wait. Not the issue.
4. **ID collision between relay-internal and Zed-facing IDs**: The relay uses its own `next_id` counter for requests it sends to the debug server; rmcp handles Zed's IDs separately on the proxy side. Not the issue.
5. **SSH stdout contaminating the MCP stdio**: `start_ssh_forward` sets SSH stdout to `Stdio::null()`. Not the issue.
6. **`proxy_mode` being true on first connection**: `proxy_mode` starts `false`, only becomes `true` after `update_debug_bin`. Not the issue.
7. **`workspace` being absent for `--remote`**: `Config::parse_and_validate` explicitly skips workspace validation when `--remote` is set. Not the issue.
8. **localhost special-casing as a fix**: EXPLICITLY REJECTED by user. localhost is only a test; no hostname-based routing differences allowed.
9. **SSH itself not working**: Confirmed working — session is established in ~1 second in every test run.
10. **`ConnectionClosed("initialize request")` being the real bug**: That error only appeared when the process was backgrounded (stdin closed). It is not what MCP clients see.

---

## Remaining Hypotheses (Most Likely First)

### H1: ~~SSH to localhost simply fails~~ — RULED OUT
SSH works; session is established in ~1 s every time.

### H2: SSH socket forwarding is disabled on the remote sshd
Even though `ssh localhost cmd` and session establishment work, `AllowStreamLocalForwarding`
might be `no` for the SESSION socket forward (step e in the connect flow). The control
socket forward (step a) succeeds (needed to establish the session), but the session socket
forward silently fails → `wait_for_socket(sess.sock)` times out after 30 s.

**Status**: plausible — needs log to confirm or deny.

### H3: `ensure_initialized` in the relay times out (MOST LIKELY)
After the full SSH setup, `relay.relay()` is called for the first time and calls
`ensure_initialized`. This sends `initialize` to the debug server over the session socket.
If the debug server does not respond within `INIT_TIMEOUT_SECS=10s`, the relay fails.
After failure, `relay_with_reconnect` tries to reconnect (up to 30 attempts × ~1 s each),
each of which again times out in `ensure_initialized`. Total wall-clock time could reach
60 s before giving up.

Possible sub-causes:
- The session socket tunnel IS up but data doesn't flow (SSH buffering / forwarding issue).
- The debug server accepted the session socket connection but its rmcp `serve().await` task
  isn't reading from it (race / scheduler starvation — unlikely but possible).
- The rmcp library rejects or drops the relay's `initialize` message for some reason
  (e.g. unexpected `capabilities` field, unsupported protocol version "2025-06-18").

**Status**: most likely — the new log file will show exactly which `relay:` line is last.

### H4: The relay reads an unexpected message and hangs
`read_matching_response` silently discards non-matching lines forever. If the debug server
sends a notification or log line before the `initialize` response, and the `id` field never
matches, the loop never exits (until the 10 s timeout). Unlikely but possible if rmcp
emits server-initiated messages on connect.

### H5: Session socket path length
`listener.rs` uses `std::env::temp_dir()` which under `nix develop` may expand to something
like `/tmp/nix-shell-XXXX/nix-shell.YYYY/`. The resulting session socket path can approach
or exceed the 108-char `SUN_LEN` limit on Linux. If the bind fails, no session is served.

**Status**: worth checking in the log — the session path is logged at `INFO`.

---

## Critical Code Locations

| What | File | Lines |
|------|------|-------|
| Entry point routing | `src/main.rs` | L46-52 |
| Remote client main flow | `src/remote/client.rs` | L316-382 |
| SSH setup & socket find | `src/remote/client.rs` | `find_remote_socket` L435-476 |
| SSH forward spawner | `src/remote/client.rs` | `start_ssh_forward` L411-433 |
| Connection / session | `src/remote/client.rs` | `ConnectionParams::connect` L87-120 |
| wait_for_setup / errors | `src/remote/client.rs` | L162-191 |
| Session request handler | `src/remote/listener.rs` | `handle_session_request` L84-155 |
| Relay protocol | `src/relay.rs` | entire file |
| DebugServer list_tools | `src/debug/server.rs` | L388-402 |

---

## Recommended Next Steps

1. **Add tracing/logging in `run_remote_client`** so the exact failure point in the SSH setup task is visible in stderr. Specifically:
   - Log after `find_remote_socket` succeeds
   - Log after first `wait_for_socket` (ctrl)
   - Log after `establish_session`
   - Log after second `wait_for_socket` (session)
   - Log any errors with full context

2. **Test SSH to localhost manually** inside the same nix shell:
   ```
   nix develop /home/sirati/devel/rust/programmer-mcp# --command ssh localhost 'echo ok'
   ```
   If this fails → H1 is the root cause.

3. **Run `test_remote.py`** from inside nix shell against a running debug server to see stderr output:
   ```
   nix develop ... --command python3 test_remote.py 2>&1
   ```

4. **Check if the issue is at `wait_for_setup` level or at `relay` level** by seeing whether the error is "SSH setup failed" vs "relay timed out" in the logs.

5. **Consider whether the feature should support a non-SSH local socket mode** (connecting directly to the control socket without SSH when the control socket path is local), but this must be triggered by something other than hostname (e.g., an explicit `--remote-socket /path/to.sock` flag or a `socket://` URI scheme), not by detecting `localhost`.