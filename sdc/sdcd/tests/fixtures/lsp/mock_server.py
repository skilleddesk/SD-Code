#!/usr/bin/env python3
"""A minimal language server for SDC's tests: speaks LSP over stdio, and reports an error for every line containing BAD
(and a warning for every line containing TODO). With --crash-after=n it exits after answering n documents."""
import json, os, sys

def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode().strip()
        if line == "":
            break
        if line.lower().startswith("content-length:"):
            length = int(line.split(":")[1])
    if length is None:
        return None
    return json.loads(sys.stdin.buffer.read(length).decode())

def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body)
    sys.stdout.buffer.flush()

documents = 0
crash_after = int(os.environ.get("MOCK_LSP_CRASH_AFTER", "0"))
for argument in sys.argv[1:]:
    if argument.startswith("--crash-after="):
        crash_after = int(argument.split("=")[1])

while True:
    message = read()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": message["id"], "result": {"capabilities": {"textDocumentSync": 1}, "serverInfo": {"name": "mock-lsp"}}})
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        params = message["params"]
        document = params["textDocument"]
        text = document.get("text") or params.get("contentChanges", [{}])[0].get("text", "")
        found = []
        for number, line in enumerate(text.splitlines()):
            if "BAD" in line:
                found.append({"range": {"start": {"line": number, "character": line.index("BAD")}, "end": {"line": number, "character": line.index("BAD") + 3}}, "severity": 1, "message": "cannot find name `BAD`", "source": "mock"})
            if "TODO" in line:
                found.append({"range": {"start": {"line": number, "character": 0}, "end": {"line": number, "character": 4}}, "severity": 2, "message": "unfinished", "source": "mock"})
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": document["uri"], "diagnostics": found}})
        documents += 1
        if crash_after and documents >= crash_after:
            sys.exit(3)
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        break
