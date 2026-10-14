#!/usr/bin/env python3
"""Minimal fake of the GitHub releases API + asset downloads, for smoke-testing
scripts/setup-winpoc.ps1 through its -ApiBase seam. stdlib only.

  GET /repos/<o>/<n>/releases        -> ROOT/<o>__<n>.json, {BASE} replaced by http://127.0.0.1:PORT
  GET /repos/ratelimited/<n>/releases -> 403 "API rate limit exceeded" (the rate-limit path)
  GET /dl/<file>                      -> ROOT/<file>
  anything else (an unknown repo)     -> 404

Env: PORT (default 18200), ROOT (fixture dir).
Every request is appended to ROOT/requests.log so a smoke can tell what was fetched."""
import json, os, socket, threading

PORT = int(os.environ.get("PORT", "18200"))
ROOT = os.environ.get("ROOT", ".")
BASE = f"http://127.0.0.1:{PORT}"


def reply(conn, code, body, ctype="application/json"):
    reason = {200: "OK", 403: "Forbidden", 404: "Not Found"}.get(code, "X")
    hdr = (f"HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\n"
           f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n").encode()
    conn.sendall(hdr + body)


def route(path):
    path = path.split("?", 1)[0]
    seg = path.split("/")
    if len(seg) == 5 and seg[1] == "repos" and seg[4] == "releases":
        if seg[2] == "ratelimited":
            return 403, json.dumps({"message": "API rate limit exceeded for 127.0.0.1."}).encode(), "application/json"
        p = os.path.join(ROOT, f"{seg[2]}__{seg[3]}.json")
        if os.path.isfile(p):
            with open(p) as f:
                return 200, f.read().replace("{BASE}", BASE).encode(), "application/json"
    if path.startswith("/dl/"):
        name = os.path.basename(path[4:])
        p = os.path.join(ROOT, name)
        if name and os.path.isfile(p):
            with open(p, "rb") as f:
                return 200, f.read(), "application/octet-stream"
    return 404, json.dumps({"message": "Not Found"}).encode(), "application/json"


def handle(conn):
    try:
        buf = b""
        while b"\r\n\r\n" not in buf:
            d = conn.recv(4096)
            if not d:
                return
            buf += d
        line = buf.split(b"\r\n", 1)[0].decode("latin1")
        parts = line.split(" ")
        path = parts[1] if len(parts) > 1 else "/"
        with open(os.path.join(ROOT, "requests.log"), "a") as f:
            f.write(path + "\n")
        reply(conn, *route(path))
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except Exception:
            pass


srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", PORT)); srv.listen(16)
print(f"mock github on 127.0.0.1:{PORT} (root {ROOT})", flush=True)
while True:
    c, _ = srv.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
