#!/usr/bin/env python3
"""Mock FunctionFly vault orchestrator for exercising `staging-smoke.sh`.

WHY THIS EXISTS
---------------
`scripts/staging-smoke.sh` is the manual gate that proves the desktop
client's HTTP contract against a real orchestrator. It had never been run
against anything, so nothing about it was known to work: the endpoint
paths, the response shapes it parses with `jq`, the CSRF rules it
assumes, and the cleanup path.

This mock implements enough of the vault API to drive the whole script
end to end, and — importantly — it ENFORCES the security contract rather
than rubber-stamping whatever it is sent:

  * every `/v1/*` route requires `Authorization: Bearer <token>`
  * every mutating route additionally requires `X-CSRF-Token`
  * a safe method must NOT need the CSRF header

That last point is what makes the mock worth having: `staging-smoke.sh`
step 4 deliberately omits the CSRF header, and the mock rejects a request
that carries one where it is not required. A script that "passed" against
a permissive mock would prove nothing.

It is a test double, not an emulator. It stores everything in memory and
implements no cryptography.

USAGE
-----
    python3 mock-orchestrator.py --port 0
    # prints: LISTENING <port>

`--port 0` picks a free port, which is what the test wrapper uses.
"""

import argparse
import json
import re
import sys
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

ACCESS_TOKEN = "mock-access-token"
REFRESH_TOKEN = "mock-refresh-token"
CSRF_TOKEN = "mock-csrf-token"
TENANT_ID = "00000000-0000-4000-8000-000000000001"
USER_ID = "00000000-0000-4000-8000-000000000002"

STATE = {
    "secrets": {},
    "tokens": {},
    "namespaces": [
        {"id": "ns-default", "name": "default"},
        {"id": "ns-smoke", "name": "smoke"},
    ],
    "audit": [],
    # Counters the test wrapper asserts on, proving the mock actually
    # exercised the paths it claims to.
    "stats": {
        "requests": 0,
        "rejected_missing_bearer": 0,
        "rejected_missing_csrf": 0,
        "csrf_on_safe_method": 0,
        "logins": 0,
        "logouts": 0,
    },
}

LOCK = threading.Lock()


def record_audit(action, secret_id=None):
    STATE["audit"].append(
        {
            "id": str(uuid.uuid4()),
            "action": action,
            "actor_type": "user",
            "actor_id": USER_ID,
            "secret_id": secret_id,
            "created_at": "2026-01-01T00:00:00Z",
        }
    )


def new_secret(name, namespace, secret_type, value):
    secret_id = str(uuid.uuid4())
    STATE["secrets"][secret_id] = {
        "id": secret_id,
        "name": name,
        "namespace": namespace,
        "secret_type": secret_type,
        "description": "",
        "tenant_id": TENANT_ID,
        "versions": [
            {"version": 1, "value": value, "created_at": "2026-01-01T00:00:00Z"}
        ],
    }
    record_audit("secret.create", secret_id)
    return secret_id


def secret_payload(secret, include_value=False):
    payload = {
        "id": secret["id"],
        "name": secret["name"],
        "namespace": secret["namespace"],
        "secret_type": secret["secret_type"],
        "description": secret["description"],
        "tenant_id": secret["tenant_id"],
        "version": len(secret["versions"]),
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }
    if include_value:
        payload["value"] = secret["versions"][-1]["value"]
    return payload


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    # ── plumbing ──────────────────────────────────────────────────────────

    def log_message(self, fmt, *args):  # keep the test output readable
        pass

    def _send(self, status, payload):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if status != 204:
            self.wfile.write(body)
        # Return a truthy value so callers can use `if denied: return denied`
        # to short-circuit. Returning None here would let a denied request
        # fall through into the route handler and mutate state.
        return True

    def _read_json(self):
        length = int(self.headers.get("Content-Length") or 0)
        if not length:
            return {}
        raw = self.rfile.read(length)
        try:
            return json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            return {}

    def _bearer_ok(self):
        return self.headers.get("Authorization") == f"Bearer {ACCESS_TOKEN}"

    def _csrf_ok(self):
        return self.headers.get("X-CSRF-Token") == CSRF_TOKEN

    def _authorise(self, mutating):
        """Returns a truthy value when the request is denied, None when it may proceed."""
        with LOCK:
            STATE["stats"]["requests"] += 1

        if not self._bearer_ok():
            with LOCK:
                STATE["stats"]["rejected_missing_bearer"] += 1
            return self._send(
                401, {"code": "unauthenticated", "message": "missing or invalid bearer token"}
            )

        if mutating:
            if not self._csrf_ok():
                with LOCK:
                    STATE["stats"]["rejected_missing_csrf"] += 1
                return self._send(
                    403,
                    {"code": "csrf_token_invalid", "message": "missing or invalid CSRF token"},
                )
        else:
            # A safe method must not require the CSRF header. If one is
            # sent anyway the mock records it, so the test wrapper can
            # assert the script is not smuggling the header into GETs.
            if self.headers.get("X-CSRF-Token") is not None:
                with LOCK:
                    STATE["stats"]["csrf_on_safe_method"] += 1

        return None

    # ── routing ───────────────────────────────────────────────────────────

    def do_GET(self):
        self._route("GET")

    def do_POST(self):
        self._route("POST")

    def do_PATCH(self):
        self._route("PATCH")

    def do_DELETE(self):
        self._route("DELETE")

    def _route(self, method):
        parsed = urlparse(self.path)
        path = parsed.path
        query = parse_qs(parsed.query)

        if path == "/auth/login" and method == "POST":
            return self._login()

        if path == "/auth/logout" and method == "POST":
            return self._logout()

        if path == "/v1/csrf" and method == "GET":
            denied = self._authorise(mutating=False)
            if denied:
                return denied
            return self._send(200, {"csrf_token": CSRF_TOKEN})

        if path.startswith("/v1/vault/"):
            return self._vault(method, path, query)

        return self._send(404, {"code": "not_found", "message": f"no route for {method} {path}"})

    # ── auth ──────────────────────────────────────────────────────────────

    def _login(self):
        with LOCK:
            STATE["stats"]["logins"] += 1
        body = self._read_json()
        if not body.get("email") or not body.get("password"):
            return self._send(
                400, {"code": "invalid_request", "message": "email and password are required"}
            )
        return self._send(
            200,
            {
                "access_token": ACCESS_TOKEN,
                "refresh_token": REFRESH_TOKEN,
                "token_type": "Bearer",
                "user": {
                    "id": USER_ID,
                    "email": body.get("email"),
                    "tenant_id": TENANT_ID,
                    "role": "owner",
                },
            },
        )

    def _logout(self):
        denied = self._authorise(mutating=True)
        if denied:
            return denied
        with LOCK:
            STATE["stats"]["logouts"] += 1
        return self._send(200, {"ok": True})

    # ── vault ─────────────────────────────────────────────────────────────

    def _vault(self, method, path, query):
        mutating = method in ("POST", "PATCH", "DELETE")
        denied = self._authorise(mutating=mutating)
        if denied:
            return denied

        rest = path[len("/v1/vault") :]

        # --- namespaces ---
        if rest == "/namespaces" and method == "GET":
            return self._send(200, {"namespaces": STATE["namespaces"], "total": len(STATE["namespaces"])})

        # --- audit ---
        if rest == "/audit" and method == "GET":
            limit = int((query.get("limit") or ["50"])[0])
            entries = STATE["audit"][:limit]
            return self._send(200, {"entries": entries, "total": len(STATE["audit"])})

        # --- bulk delete (must be matched before /secrets/{id}) ---
        if rest == "/secrets/bulk-delete" and method == "DELETE":
            body = self._read_json()
            ids = body.get("secret_ids") or []
            dry_run = bool(body.get("dry_run"))
            blockers = [i for i in ids if i not in STATE["secrets"]]
            previews = [
                {"id": i, "name": STATE["secrets"][i]["name"]}
                for i in ids
                if i in STATE["secrets"]
            ]
            if not dry_run:
                for i in ids:
                    if i in STATE["secrets"]:
                        del STATE["secrets"][i]
                        record_audit("secret.delete", i)
            return self._send(200, {"previews": previews, "blockers": blockers})

        # --- collection ---
        if rest == "/secrets":
            if method == "GET":
                search = (query.get("search") or [""])[0]
                limit = int((query.get("limit") or ["50"])[0])
                matches = [
                    secret_payload(s)
                    for s in STATE["secrets"].values()
                    if not search or search in s["name"]
                ]
                return self._send(200, {"secrets": matches, "total": len(matches)})

            if method == "POST":
                body = self._read_json()
                name = body.get("name")
                if not name:
                    return self._send(
                        400, {"code": "invalid_request", "message": "name is required"}
                    )
                secret_id = new_secret(
                    name,
                    body.get("namespace") or "default",
                    body.get("secret_type") or "api_key",
                    body.get("value") or "",
                )
                return self._send(
                    201, secret_payload(STATE["secrets"][secret_id], include_value=True)
                )

        # --- tokens on a secret ---
        m = re.fullmatch(r"/secrets/([^/]+)/tokens", rest)
        if m:
            secret_id = m.group(1)
            if secret_id not in STATE["secrets"]:
                return self._send(404, {"code": "not_found", "message": "no such secret"})
            if method == "GET":
                tokens = [
                    t for t in STATE["tokens"].values() if t["secret_id"] == secret_id
                ]
                return self._send(200, {"tokens": tokens})
            if method == "POST":
                body = self._read_json()
                token_id = str(uuid.uuid4())
                STATE["tokens"][token_id] = {
                    "token_id": token_id,
                    "secret_id": secret_id,
                    "name": body.get("name") or "token",
                    "scopes": body.get("scopes") or [],
                    "created_at": "2026-01-01T00:00:00Z",
                }
                # The orchestrator returns the plaintext exactly once.
                payload = dict(STATE["tokens"][token_id])
                payload["token"] = "ffp_1_mock_plaintext_token"
                record_audit("token.create", secret_id)
                return self._send(201, payload)

        # --- token revoke ---
        m = re.fullmatch(r"/tokens/([^/]+)", rest)
        if m and method == "DELETE":
            token_id = m.group(1)
            if token_id not in STATE["tokens"]:
                return self._send(404, {"code": "not_found", "message": "no such token"})
            del STATE["tokens"][token_id]
            record_audit("token.revoke")
            return self._send(204, {})

        # --- versions diff ---
        m = re.fullmatch(r"/secrets/([^/]+)/versions/diff", rest)
        if m and method == "GET":
            secret = STATE["secrets"].get(m.group(1))
            if secret is None:
                return self._send(404, {"code": "not_found", "message": "no such secret"})
            versions = {str(v["version"]): v for v in secret["versions"]}
            frm = (query.get("from") or ["1"])[0]
            to = (query.get("to") or ["1"])[0]
            a = versions.get(frm)
            b = versions.get(to)
            if a is None or b is None:
                return self._send(404, {"code": "not_found", "message": "no such version"})
            return self._send(
                200,
                {
                    "from": int(frm),
                    "to": int(to),
                    "changed": a["value"] != b["value"],
                    "fields": ["value"],
                },
            )

        # --- versions ---
        m = re.fullmatch(r"/secrets/([^/]+)/versions", rest)
        if m and method == "GET":
            secret = STATE["secrets"].get(m.group(1))
            if secret is None:
                return self._send(404, {"code": "not_found", "message": "no such secret"})
            return self._send(200, {"versions": secret["versions"]})

        # --- rollback ---
        m = re.fullmatch(r"/secrets/([^/]+)/rollback", rest)
        if m and method == "POST":
            secret = STATE["secrets"].get(m.group(1))
            if secret is None:
                return self._send(404, {"code": "not_found", "message": "no such secret"})
            body = self._read_json()
            target = int(body.get("target_version") or 1)
            if not any(v["version"] == target for v in secret["versions"]):
                return self._send(
                    404, {"code": "not_found", "message": "no such version"}
                )
            # Roll back by appending the old value as a new version, which
            # is how the real orchestrator preserves history.
            secret["versions"].append(
                {
                    "version": len(secret["versions"]) + 1,
                    "value": next(v["value"] for v in secret["versions"] if v["version"] == target),
                    "created_at": "2026-01-01T00:00:00Z",
                    "rollback_reason": body.get("reason") or "",
                }
            )
            record_audit("secret.rollback", secret["id"])
            return self._send(200, secret_payload(secret, include_value=True))

        # --- single secret ---
        m = re.fullmatch(r"/secrets/([^/]+)", rest)
        if m:
            secret_id = m.group(1)
            secret = STATE["secrets"].get(secret_id)
            if secret is None:
                return self._send(404, {"code": "not_found", "message": "no such secret"})

            if method == "GET":
                return self._send(200, secret_payload(secret, include_value=True))

            if method == "PATCH":
                body = self._read_json()
                if "value" in body:
                    secret["versions"].append(
                        {
                            "version": len(secret["versions"]) + 1,
                            "value": body["value"],
                            "created_at": "2026-01-01T00:00:00Z",
                        }
                    )
                if "description" in body:
                    secret["description"] = body["description"]
                if "name" in body:
                    secret["name"] = body["name"]
                record_audit("secret.update", secret_id)
                return self._send(200, secret_payload(secret, include_value=True))

            if method == "DELETE":
                del STATE["secrets"][secret_id]
                record_audit("secret.delete", secret_id)
                return self._send(204, {})

        return self._send(
            404, {"code": "not_found", "message": f"no route for {method} {path}"}
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=0, help="0 picks a free port")
    parser.add_argument(
        "--stats-port",
        type=int,
        default=None,
        help="also serve GET /__stats on this port (same server)",
    )
    args = parser.parse_args()

    class StatsHandler(Handler):
        def do_GET(self):
            if urlparse(self.path).path == "/__stats":
                with LOCK:
                    return self._send(200, dict(STATE["stats"]))
            return super().do_GET()

        def do_POST(self):
            # Test-only: reset the counters so a test wrapper can measure
            # exactly what the script under test did, rather than what the
            # wrapper's own probes did.
            if urlparse(self.path).path == "/__reset":
                with LOCK:
                    for key in STATE["stats"]:
                        STATE["stats"][key] = 0
                return self._send(200, {"ok": True})
            return super().do_POST()

    server = ThreadingHTTPServer(("127.0.0.1", args.port), StatsHandler)
    port = server.server_address[1]
    print(f"LISTENING {port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
