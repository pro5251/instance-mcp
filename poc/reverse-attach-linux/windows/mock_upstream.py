#!/usr/bin/env python3
"""Minimal mock of an upstream MCP server (like @playwright/mcp) over Streamable HTTP,
for smoke-testing the Windows node's --upstream forwarding and profile filtering.
stdlib only. Advertises three tools: browser_navigate, browser_snapshot (both on the
desktop allowlist) and browser_evaluate (owner-only). PORT env sets the port."""
import json, os, socket, threading, uuid

PORT = int(os.environ.get("PORT", "18100"))
TOOLS = [
    {"name": "browser_navigate", "description": "navigate", "inputSchema": {"type": "object", "properties": {"url": {"type": "string"}}}},
    {"name": "browser_snapshot", "description": "snapshot", "inputSchema": {"type": "object", "properties": {}}},
    {"name": "browser_evaluate", "description": "evaluate JS", "inputSchema": {"type": "object", "properties": {"fn": {"type": "string"}}}},
]

def body_for(req):
    m = req.get("method"); i = req.get("id")
    if m == "initialize":
        return {"jsonrpc": "2.0", "id": i, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "mock-pw", "version": "0"}}}
    if m == "tools/list":
        return {"jsonrpc": "2.0", "id": i, "result": {"tools": TOOLS}}
    if m == "tools/call":
        name = req.get("params", {}).get("name", "")
        return {"jsonrpc": "2.0", "id": i, "result": {"content": [{"type": "text", "text": f"mock {name} ok"}], "structuredContent": {"ran": name}}}
    return {"jsonrpc": "2.0", "id": i, "error": {"code": -32601, "message": "method not found"}}

def handle(conn):
    try:
        buf = b""
        while b"\r\n\r\n" not in buf:
            d = conn.recv(4096)
            if not d: return
            buf += d
        head, rest = buf.split(b"\r\n\r\n", 1)
        n = 0
        for line in head.decode("latin1").split("\r\n")[1:]:
            k, _, v = line.partition(":")
            if k.strip().lower() == "content-length": n = int(v.strip())
        while len(rest) < n:
            rest += conn.recv(4096)
        req = json.loads(rest[:n] or b"{}")
        if req.get("id") is None:  # a notification: 202, no body
            conn.sendall(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            return
        payload = json.dumps(body_for(req)).encode()
        hdr = (f"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
               f"Mcp-Session-Id: {uuid.uuid4().hex}\r\nContent-Length: {len(payload)}\r\n"
               f"Connection: close\r\n\r\n").encode()
        conn.sendall(hdr + payload)
    except Exception:
        pass
    finally:
        try: conn.close()
        except Exception: pass

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", PORT)); srv.listen(16)
print(f"mock upstream on 127.0.0.1:{PORT}", flush=True)
while True:
    c, _ = srv.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
