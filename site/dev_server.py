"""Serve ``_site/`` locally and answer the playground natively, for testing.

    python3 site/build.py && python3 site/dev_server.py
    open http://localhost:8000/?backend=local

``?backend=local`` makes the page call this server instead of loading Pyodide,
so the UI and ``qbm_web`` can be exercised with CPython (numpy, scipy, pyyaml).
Without the query parameter the page behaves exactly as on GitHub Pages.
"""
from __future__ import annotations

import json
import sys
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "site")]
import qbm_web  # noqa: E402

ALLOWED = {"prepare", "run_step", "constrained_comparison", "heme_curve"}


class Handler(SimpleHTTPRequestHandler):
    def do_POST(self):
        name = self.path.removeprefix("/api/")
        try:
            if name not in ALLOWED:
                raise ValueError(f"unknown function {name}")
            args = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or "[]")
            status, body = 200, getattr(qbm_web, name)(*args)
        except Exception as error:  # surfaced to the page like a Pyodide error
            status, body = 400, {"error": str(error)}
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8000
    server = ThreadingHTTPServer(("127.0.0.1", port), partial(Handler, directory=str(ROOT / "_site")))
    print(f"http://localhost:{port}/?backend=local")
    server.serve_forever()
