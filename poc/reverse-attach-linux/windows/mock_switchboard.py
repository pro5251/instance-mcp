#!/usr/bin/env python3
"""Mock openab-sb switchboard for smoke-testing the Windows node's switchboard mode.
stdlib only. GET /vm/attach with Authorization: Bearer <SECRET> -> 101, then acts as the
MCP client (initialize, tools/list, tools/call sys_info) and closes with a code from
CLOSES (comma list; last repeats). 401 on a wrong secret. Logs to LOG.
Env: PORT (18101), SECRET, CLOSES (default 1000), LOG."""
import base64, hashlib, json, os, socket, struct, threading, time

PORT = int(os.environ.get("PORT", "18101"))
SECRET = os.environ.get("SECRET", "sb-secret")
CLOSES = [int(c) for c in os.environ.get("CLOSES", "1000").split(",")]
LOG = os.environ.get("LOG", "/tmp/mock-sb.jsonl")
GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
attach_count = 0
lock = threading.Lock()

def log(ev):
    ev["t"] = round(time.time(), 3)
    with lock, open(LOG, "a") as f:
        f.write(json.dumps(ev) + "\n")

def ws_send(conn, opcode, payload):
    hdr = bytes([0x80 | opcode])
    n = len(payload)
    if n < 126: hdr += bytes([n])
    elif n < 65536: hdr += bytes([126]) + struct.pack(">H", n)
    else: hdr += bytes([127]) + struct.pack(">Q", n)
    conn.sendall(hdr + payload)

def recv_exact(conn, n):
    b = b""
    while len(b) < n:
        c = conn.recv(n - len(b))
        if not c: raise ConnectionError("eof")
        b += c
    return b

def ws_recv(conn):
    b1, b2 = recv_exact(conn, 2)
    opcode = b1 & 0x0F; masked = b2 & 0x80; n = b2 & 0x7F
    if n == 126: n = struct.unpack(">H", recv_exact(conn, 2))[0]
    elif n == 127: n = struct.unpack(">Q", recv_exact(conn, 8))[0]
    mask = recv_exact(conn, 4) if masked else None
    data = recv_exact(conn, n)
    if mask: data = bytes(d ^ mask[i % 4] for i, d in enumerate(data))
    return opcode, data

def ws_close(conn, code):
    ws_send(conn, 0x8, struct.pack(">H", code))
    try:
        conn.settimeout(3); ws_recv(conn)
    except Exception: pass

def rpc(conn, id_, method, params=None):
    msg = {"jsonrpc": "2.0", "id": id_, "method": method}
    if params is not None: msg["params"] = params
    ws_send(conn, 0x1, json.dumps(msg).encode())
    while True:
        op, data = ws_recv(conn)
        if op == 0x1:
            r = json.loads(data); log({"ev": "rpc", "method": method, "reply": r}); return r
        if op == 0x9: ws_send(conn, 0xA, data)
        if op == 0x8: raise ConnectionError("peer closed")

def read_http(conn):
    buf = b""
    while b"\r\n\r\n" not in buf:
        c = conn.recv(4096)
        if not c: return None
        buf += c
    head = buf.split(b"\r\n\r\n", 1)[0].decode("latin1").split("\r\n")
    method, path, _ = head[0].split(" ", 2)
    h = {}
    for l in head[1:]:
        k, _, v = l.partition(":"); h[k.strip().lower()] = v.strip()
    return method, path, h

def handle(conn):
    global attach_count
    try:
        req = read_http(conn)
        if not req: return
        method, path, h = req
        if method != "GET" or not path.endswith("/vm/attach"):
            conn.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); return
        bearer = h.get("authorization", "")[7:] if h.get("authorization", "").startswith("Bearer ") else None
        if bearer != SECRET:
            log({"ev": "attach", "status": 401})
            conn.sendall(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); return
        key = h["sec-websocket-key"]
        accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
        conn.sendall(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                      f"Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").encode())
        with lock:
            attach_count += 1; nth = attach_count
        code = CLOSES[min(nth - 1, len(CLOSES) - 1)]
        log({"ev": "attach", "status": 101, "nth": nth, "will_close": code})
        init = rpc(conn, 1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                           "clientInfo": {"name": "openab-sb", "version": "0"}})
        ws_send(conn, 0x1, json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}).encode())
        rpc(conn, 2, "tools/list")
        rpc(conn, 3, "tools/call", {"name": "sys_info", "arguments": {}})
        ws_close(conn, code)
        log({"ev": "closed", "code": code, "nth": nth})
    except Exception as e:
        log({"ev": "error", "err": repr(e)})
    finally:
        try: conn.close()
        except Exception: pass

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", PORT)); srv.listen(8)
log({"ev": "listening", "port": PORT, "closes": CLOSES})
while True:
    c, _ = srv.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
