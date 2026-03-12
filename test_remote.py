#!/usr/bin/env python3
"""Test MCP remote connection: runs initialize + tools/list for both --remote and --debug --remote.
Uses threads so reads never block forever."""

import subprocess
import json
import sys
import time
import threading
import queue

BINARY = "./debug-mcp/programmer-mcp"


def start_reader(stream, q):
    """Read lines from stream into queue; put None on EOF."""
    def _run():
        try:
            while True:
                line = stream.readline()
                if not line:
                    q.put(None)
                    return
                line = line.strip()
                if line:
                    q.put(line)
        except Exception as e:
            q.put(None)
    t = threading.Thread(target=_run, daemon=True)
    t.start()
    return t


def recv_json(q, timeout=10.0):
    """Pull lines from queue until we get valid JSON or timeout."""
    deadline = time.time() + timeout
    while True:
        remaining = deadline - time.time()
        if remaining <= 0:
            return None
        try:
            item = q.get(timeout=remaining)
        except queue.Empty:
            return None
        if item is None:
            return None  # EOF
        try:
            return json.loads(item)
        except json.JSONDecodeError:
            pass  # skip non-JSON lines (e.g. tracing output on stdout)


def send(proc, msg):
    line = json.dumps(msg) + "\n"
    proc.stdin.write(line.encode())
    proc.stdin.flush()


def drain_stderr(proc, timeout=1.0):
    lines = []
    def _read():
        try:
            for line in proc.stderr:
                lines.append(line.decode(errors="replace").rstrip())
        except Exception:
            pass
    t = threading.Thread(target=_read, daemon=True)
    t.start()
    t.join(timeout=timeout)
    return lines


def run_test(label, extra_args):
    print(f"\n{'='*60}")
    print(f"TEST: {label}")
    print(f"ARGS: {[BINARY] + extra_args}")
    print('='*60)

    proc = subprocess.Popen(
        [BINARY] + extra_args,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    out_q = queue.Queue()
    start_reader(proc.stdout, out_q)

    # Give the process time to establish the SSH tunnel / session
    time.sleep(3.0)

    rc = proc.poll()
    if rc is not None:
        print(f"!!! Process exited early with code {rc}")
        for line in drain_stderr(proc, timeout=1.0):
            print(f"  stderr: {line}")
        return

    # 1. initialize
    init_req = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "test-client", "version": "0.1"},
        },
    }
    print("\n>>> initialize")
    send(proc, init_req)

    resp = recv_json(out_q, timeout=10.0)
    if resp is None:
        print("!!! No response to initialize (timeout or EOF)")
        proc.kill()
        for line in drain_stderr(proc, timeout=1.0):
            print(f"  stderr: {line}")
        return
    print(f"<<< {json.dumps(resp, indent=2)}")

    if "error" in resp:
        print("!!! initialize returned error, aborting")
        proc.kill()
        return

    # 2. notifications/initialized
    notif = {
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {},
    }
    print("\n>>> notifications/initialized")
    send(proc, notif)
    time.sleep(0.2)

    # 3. tools/list
    tools_req = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {},
    }
    print("\n>>> tools/list")
    send(proc, tools_req)

    resp = recv_json(out_q, timeout=15.0)
    if resp is None:
        print("!!! No response to tools/list (timeout or EOF)")
        proc.kill()
        for line in drain_stderr(proc, timeout=1.0):
            print(f"  stderr: {line}")
        return

    if "result" in resp and "tools" in resp.get("result", {}):
        names = [t["name"] for t in resp["result"]["tools"]]
        print(f"<<< tools/list OK — tools: {names}")
    else:
        print(f"<<< {json.dumps(resp, indent=2)}")

    # Clean shutdown
    proc.stdin.close()
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()

    for line in drain_stderr(proc, timeout=1.0):
        print(f"  stderr: {line}")
    print(f"\nProcess exited with code {proc.returncode}")


def call_tool(proc, out_q, tool_name, arguments=None, timeout=30.0):
    """Send a tools/call request and return the response."""
    req = {
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments or {},
        },
    }
    print(f"\n>>> tools/call {tool_name} (timeout={timeout}s)")
    send(proc, req)
    resp = recv_json(out_q, timeout=timeout)
    if resp is None:
        print(f"!!! No response to tools/call {tool_name} (timeout or EOF)")
        return None
    return resp


def run_debug_tool_tests(extra_args):
    """Run tests specific to the --debug remote case: status and rebuild."""
    label = "debug tool calls: " + " ".join(extra_args)
    print(f"\n{'='*60}")
    print(f"TEST: {label}")
    print('='*60)

    proc = subprocess.Popen(
        [BINARY] + extra_args,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    out_q = queue.Queue()
    start_reader(proc.stdout, out_q)

    time.sleep(3.0)

    rc = proc.poll()
    if rc is not None:
        print(f"!!! Process exited early with code {rc}")
        for line in drain_stderr(proc, timeout=1.0):
            print(f"  stderr: {line}")
        return

    # Full MCP handshake
    send(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "test-client", "version": "0.1"}}})
    resp = recv_json(out_q, timeout=10.0)
    if resp is None or "error" in resp:
        print(f"!!! initialize failed: {resp}")
        proc.kill()
        return
    print(f"<<< initialize OK")

    send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
    time.sleep(0.2)

    # Test: status (should be fast)
    resp = call_tool(proc, out_q, "status", timeout=10.0)
    if resp:
        content = resp.get("result", {}).get("content", [{}])
        text = content[0].get("text", "") if content else ""
        print(f"<<< status: {text!r}")

    # Test: rebuild (slow — will expose relay timeout if < build time)
    print(f"\n>>> tools/call rebuild  [NOTE: watching for relay timeout vs actual build]")
    t0 = time.time()
    resp = call_tool(proc, out_q, "rebuild", timeout=300.0)
    elapsed = time.time() - t0
    if resp is None:
        print(f"!!! rebuild: no response after {elapsed:.1f}s")
    elif "error" in resp:
        print(f"<<< rebuild ERROR after {elapsed:.1f}s: {resp['error']}")
    else:
        content = resp.get("result", {}).get("content", [{}])
        text = content[0].get("text", "") if content else ""
        is_error = resp.get("result", {}).get("isError", False)
        print(f"<<< rebuild {'ERROR' if is_error else 'OK'} after {elapsed:.1f}s: {text!r}")

    proc.stdin.close()
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()

    for line in drain_stderr(proc, timeout=1.0):
        print(f"  stderr: {line}")
    print(f"\nProcess exited with code {proc.returncode}")


if __name__ == "__main__":
    run_test("--remote localhost (no --debug)", ["--remote", "localhost"])
    run_test("--debug --remote localhost", ["--debug", "--remote", "localhost"])
    run_debug_tool_tests(["--debug", "--remote", "localhost"])
