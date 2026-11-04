#!/usr/bin/env python3
"""Stateful mock R2 / S3 endpoint for the vault-desktop release
pipeline smoke test. Records every uploaded object under
$MOCK_R2_STORE/<bucket>/<key> so the publish-update.sh test can
inspect what was actually written.

Usage:

    MOCK_R2_STORE=/tmp/mock-r2-store python3 mock-r2.py 18234 &

Then run the release scripts with `R2_ENDPOINT_OVERRIDE=http://127.0.0.1:18234`
to redirect them at this mock. Captured evidence at
`scripts/release/evidence/2026-10-06-mock-r2-round-trip.md`.
"""
import http.server
import socketserver
import os
import sys
import datetime

STORE_ROOT = os.environ.get("MOCK_R2_STORE", "/tmp/mock-r2-store")
os.makedirs(STORE_ROOT, exist_ok=True)


def _path(bucket: str, key: str) -> str:
    safe_key = key.lstrip("/")
    return os.path.join(STORE_ROOT, bucket, safe_key)


class MockR2(http.server.BaseHTTPRequestHandler):
    def _path_for(self) -> str:
        return self.path.lstrip("/")

    def _read_body(self) -> bytes:
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            length = 0
        if length <= 0:
            return b""
        if length > 64 * 1024 * 1024:
            self.send_response(413)
            self.end_headers()
            return b""
        return self.rfile.read(length)

    def _reply(self, code, body=b"", content_type="application/octet-stream"):
        self.send_response(code)
        if self.command != "HEAD":
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body and self.command != "HEAD":
            try:
                self.wfile.write(body)
            except BrokenPipeError:
                pass

    def do_HEAD(self):
        full = self._path_for()
        if "/" not in full:
            self._reply(200)
            return
        bucket, _, key = full.partition("/")
        path = _path(bucket, key)
        if os.path.exists(path):
            size = os.path.getsize(path)
            self.send_response(200)
            self.send_header("Content-Length", str(size))
            self.end_headers()
        else:
            self._reply(404)

    def do_PUT(self):
        body = self._read_body()
        full = self._path_for().split("?", 1)[0]
        if "/" not in full:
            self._reply(200)
            return
        bucket, _, key = full.partition("/")
        path = _path(bucket, key)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(body)
        self._reply(200)

    def do_GET(self):
        full = self._path_for().split("?", 1)[0]
        if "/" not in full:
            self._reply(200)
            return
        bucket, _, key = full.partition("/")
        path = _path(bucket, key)
        if not os.path.exists(path):
            self._reply(404, b"Not Found", content_type="text/plain")
            return
        with open(path, "rb") as f:
            data = f.read()
        ct = "application/octet-stream"
        if key.endswith(".json"):
            ct = "application/json"
        self._reply(200, data, content_type=ct)

    def do_DELETE(self):
        full = self._path_for().split("?", 1)[0]
        if "/" not in full:
            self._reply(204)
            return
        bucket, _, key = full.partition("/")
        path = _path(bucket, key)
        try:
            os.remove(path)
        except FileNotFoundError:
            pass
        self._reply(204)

    def log_message(self, *args, **kwargs):
        return


def serve(port=18234):
    socketserver.TCPServer.allow_reuse_address = True
    s = socketserver.TCPServer(("127.0.0.1", port), MockR2)
    print(
        f"mock-r2 ready on :{port} (store: {STORE_ROOT}) {datetime.datetime.now().isoformat()}",
        flush=True,
    )
    s.serve_forever()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18234
    serve(port)