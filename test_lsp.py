#!/usr/bin/env python3
"""Manual test for basedpyright-langserver LSP communication."""

import subprocess, json, time, select, sys

def send_lsp(proc, msg):
    body = json.dumps(msg)
    header = f"Content-Length: {len(body)}\r\n\r\n"
    proc.stdin.write(header.encode())
    proc.stdin.write(body.encode())
    proc.stdin.flush()

_leftover = b""

def read_one(proc, timeout=5):
    """Read one complete LSP message."""
    global _leftover
    start = time.time()
    buf = _leftover
    while time.time() - start < timeout:
        if select.select([proc.stdout], [], [], 0.2)[0]:
            chunk = proc.stdout.read1(8192)
            if chunk:
                buf += chunk
        text = buf.decode("utf-8", errors="replace")
        idx = text.find("Content-Length:")
        if idx >= 0:
            hdr_end = text.find("\r\n\r\n", idx)
            if hdr_end >= 0:
                clen = None
                for line in text[idx:hdr_end].split("\r\n"):
                    if line.startswith("Content-Length:"):
                        clen = int(line.split(":")[1].strip())
                if clen is not None:
                    body_start = hdr_end + 4
                    if len(text[body_start:].encode("utf-8")) >= clen:
                        body_text = text[body_start:body_start + clen]
                        consumed = body_start + clen
                        _leftover = text[consumed:].encode("utf-8")
                        return json.loads(body_text)
    _leftover = buf
    return None

def respond_to_server_request(proc, msg):
    method = msg["method"]
    rid = msg["id"]
    print(f"  -> Responding to server request: {method} id={rid}")
    if method == "workspace/configuration":
        send_lsp(proc, {"jsonrpc": "2.0", "id": rid, "result": [{}]})
    elif method in ("client/registerCapability", "window/workDoneProgress/create"):
        send_lsp(proc, {"jsonrpc": "2.0", "id": rid, "result": None})
    else:
        print(f"     UNKNOWN server request: {method}")
        send_lsp(proc, {"jsonrpc": "2.0", "id": rid, "result": None})

proc = subprocess.Popen(
    ["basedpyright-langserver", "--stdio"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
)

# Use same capabilities as our Rust code
print("=== Sending initialize ===")
send_lsp(proc, {
    "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": {
        "processId": None,
        "rootUri": "file:///home/sirati/devel/rust/programmer-mcp",
        "capabilities": {
            "workspace": {
                "workspaceFolders": True,
                "configuration": True,
                "didChangeConfiguration": {"dynamicRegistration": True},
                "didChangeWatchedFiles": {"dynamicRegistration": True, "relativePatternSupport": True},
                "symbol": {"dynamicRegistration": True},
                "applyEdit": True,
            },
            "textDocument": {
                "documentSymbol": {"dynamicRegistration": True},
                "publishDiagnostics": {"versionSupport": True},
            },
        },
        "clientInfo": {"name": "programmer-mcp", "version": "0.1.0"},
    }
})

found_init = False
found_symbols = False
sent_doc = False
server_req_count = 0
init_time = None
start = time.time()

while time.time() - start < 30 and not found_symbols:
    msg = read_one(proc, timeout=3)
    if msg is None:
        # Wait 15 seconds after init (like our Rust code) before sending didOpen
        if found_init and not sent_doc and time.time() - init_time > 15:
            print(f"  (waited {time.time() - init_time:.1f}s after init, sending didOpen + documentSymbol)")
            send_lsp(proc, {
                "jsonrpc": "2.0", "method": "textDocument/didOpen",
                "params": {"textDocument": {
                    "uri": "file:///home/sirati/devel/rust/programmer-mcp/test_py.py",
                    "languageId": "python", "version": 1,
                    "text": 'x: int = 42\nclass Foo:\n    def bar(self) -> str:\n        return "hello"\n'
                }}
            })
            time.sleep(0.3)
            send_lsp(proc, {
                "jsonrpc": "2.0", "id": 2, "method": "textDocument/documentSymbol",
                "params": {"textDocument": {"uri": "file:///home/sirati/devel/rust/programmer-mcp/test_py.py"}}
            })
            sent_doc = True
        continue

    has_id = "id" in msg
    has_method = "method" in msg

    if has_id and not has_method:
        # Response to our request
        if msg["id"] == 1:
            print("Got initialize response")
            found_init = True
            init_time = time.time()
            send_lsp(proc, {"jsonrpc": "2.0", "method": "initialized", "params": {}})
        elif msg["id"] == 2:
            result = json.dumps(msg.get("result", msg.get("error")))[:400]
            print(f"SUCCESS documentSymbol: {result}")
            found_symbols = True
        else:
            print(f"Response id={msg['id']}: {json.dumps(msg)[:200]}")
    elif has_id and has_method:
        server_req_count += 1
        if server_req_count <= 5 or server_req_count % 100 == 0:
            print(f"  -> Server request #{server_req_count}: {msg['method']} id={msg['id']}")
        respond_to_server_request(proc, msg)
    else:
        notif = msg.get("method", "?")
        if notif not in ("window/logMessage",):
            print(f"  Notification: {notif}")

if not found_symbols:
    print("FAILED: did not get documentSymbol response in time")

proc.terminate()
proc.wait()
print("Done.")
