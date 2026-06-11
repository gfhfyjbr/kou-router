#!/usr/bin/env python3
"""Live demo traffic for the kou-router logs page.

Starts a mock OpenAI-compatible upstream on :20999, creates demo providers
("Claude Code (demo)" / "Codex (demo)"), accounts and API keys in the router,
then fires randomized traffic (streams, big prompts, occasional 4xx/5xx)
until Ctrl-C — so /#logs fills up and updates live.

Usage:
    python3 scripts/demo-traffic.py            # router at http://127.0.0.1:20128
    KOU_ROUTER=http://127.0.0.1:20444 python3 scripts/demo-traffic.py
"""
import json
import os
import random
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROUTER = os.environ.get("KOU_ROUTER", "http://127.0.0.1:20128")
MOCK_PORT = int(os.environ.get("KOU_MOCK_PORT", "20999"))
KEYS_CACHE = "/tmp/kou-demo-keys.json"

# ── mock upstream ────────────────────────────────────────────────────


class MockHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        model = body.get("model", "kou-demo")
        stream = bool(body.get("stream"))

        # occasional upstream failures keep the error rows realistic
        roll = random.random()
        if roll < 0.06:
            self._json(500, {"error": {"message": "mock upstream exploded under load", "type": "server_error"}})
            return
        if roll < 0.10:
            self._json(429, {"error": {"message": "mock rate limit exceeded, slow down", "type": "rate_limit_error"}})
            return

        ti = random.randint(900, 52_000)
        to = random.randint(2, 1_900)

        if stream:
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("transfer-encoding", "chunked")
            self.end_headers()
            words = "The switchyard hums softly tonight as cars roll between the lines .".split()
            chunks = random.randint(4, 14)
            for i in range(chunks):
                self._sse({"id": "chatcmpl-demo", "object": "chat.completion.chunk", "model": model,
                           "choices": [{"index": 0, "delta": {"content": " " + words[i % len(words)]}, "finish_reason": None}]})
                time.sleep(random.uniform(0.05, 0.45))
            self._sse({"id": "chatcmpl-demo", "object": "chat.completion.chunk", "model": model,
                       "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                       "usage": {"prompt_tokens": ti, "completion_tokens": to, "total_tokens": ti + to}})
            done = b"data: [DONE]\n\n"
            self.wfile.write(f"{len(done):x}\r\n".encode() + done + b"\r\n0\r\n\r\n")
            self.wfile.flush()
            return

        time.sleep(random.uniform(0.15, 2.8))
        self._json(200, {
            "id": "chatcmpl-demo", "object": "chat.completion", "model": model,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Kou routes the request across the yard."}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": ti, "completion_tokens": to, "total_tokens": ti + to},
        })

    def _json(self, status, obj):
        payload = json.dumps(obj).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _sse(self, obj):
        data = f"data: {json.dumps(obj)}\n\n".encode()
        self.wfile.write(f"{len(data):x}\r\n".encode() + data + b"\r\n")
        self.wfile.flush()


# ── router api helpers ───────────────────────────────────────────────


def api(path, payload=None, method=None, key=None, timeout=90):
    req = urllib.request.Request(
        ROUTER + path,
        data=json.dumps(payload).encode() if payload is not None else None,
        method=method or ("POST" if payload is not None else "GET"),
        headers={"content-type": "application/json", **({"authorization": "Bearer " + key} if key else {})},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read() or b"{}")


def ensure_provider(existing, name, prefix, accounts):
    for p in existing:
        if p.get("name") == name:
            return p["id"]
    created = api("/api/providers", {
        "provider": prefix, "base_url": f"http://127.0.0.1:{MOCK_PORT}/v1",
        "api_key": "sk-demo", "model_prefix": prefix, "name": name,
        "priority": 1, "protocol_format": "openai",
    })
    pid = created["id"]
    for label in accounts:
        api("/api/provider-accounts", {
            "provider_connection_id": pid, "label": label,
            "auth_mode": "api_key", "api_key": "sk-demo-" + label.split("@")[0],
            "remote_email": label if "@" in label else None,
        })
    return pid


def ensure_keys():
    cached = {}
    if os.path.exists(KEYS_CACHE):
        with open(KEYS_CACHE) as fh:
            cached = json.load(fh)
    names = {k["name"] for k in api("/api/keys")}
    for name in ("demo-cli", "demo-ci-bot"):
        if name in cached and name in names:
            continue
        if name in names:  # key exists but secret lost — recreate under a fresh run
            continue
        created = api("/api/keys", {"name": name, "allowed_models": ["*"]})
        cached[name] = created["key"]
    with open(KEYS_CACHE, "w") as fh:
        json.dump(cached, fh)
    return [v for v in cached.values()]


# ── traffic loop ─────────────────────────────────────────────────────

MODELS = [
    ("claude/claude-opus-4-8", 4),
    ("claude/claude-sonnet-4-6", 5),
    ("claude/claude-haiku-4-5", 2),
    ("codex/gpt-5.5", 4),
    ("codex/gpt-5-codex", 3),
    ("codex/codex-mini-latest", 1),
]

PROMPTS = [
    "Explain the switchyard routing in one paragraph.",
    "Refactor this function to avoid the extra clone.",
    "Summarize today's dispatch report.",
    "Write a haiku about freight trains at night.",
    "Why does the circuit breaker open after repeated 500s?",
]


def fire(keys):
    model = random.choices([m for m, _ in MODELS], weights=[w for _, w in MODELS])[0]
    if random.random() < 0.05:
        model = "ghost/unknown-9"  # unresolved prefix -> 404 row
    payload = {
        "model": model,
        "stream": random.random() < 0.5,
        "messages": [{"role": "user", "content": random.choice(PROMPTS)}],
    }
    key = random.choice(keys) if random.random() < 0.9 else None
    try:
        req = urllib.request.Request(
            ROUTER + "/v1/chat/completions",
            data=json.dumps(payload).encode(),
            headers={"content-type": "application/json", **({"authorization": "Bearer " + key} if key else {})},
        )
        with urllib.request.urlopen(req, timeout=90) as resp:
            resp.read()
        status = resp.status
    except urllib.error.HTTPError as err:
        err.read()
        status = err.code
    except Exception as err:  # noqa: BLE001
        status = f"?? ({err})"
    print(f"  -> {status} {model} stream={payload['stream']} key={'yes' if key else 'no'}")


def main():
    threading.Thread(
        target=lambda: ThreadingHTTPServer(("127.0.0.1", MOCK_PORT), MockHandler).serve_forever(),
        daemon=True,
    ).start()
    print(f"mock upstream on :{MOCK_PORT}, router at {ROUTER}")

    try:
        providers = api("/api/providers")
    except Exception as err:  # noqa: BLE001
        sys.exit(f"router not reachable at {ROUTER}: {err}")

    ensure_provider(providers, "Claude Code (demo)", "claude", ["kou@anthropic.dev", "backup@anthropic.dev"])
    ensure_provider(providers, "Codex (demo)", "codex", ["kou@openai.dev"])
    keys = ensure_keys()
    print(f"demo providers ready, {len(keys)} api keys — open {ROUTER}/#logs and watch. Ctrl-C to stop.")

    while True:
        threading.Thread(target=fire, args=(keys,), daemon=True).start()
        time.sleep(random.uniform(0.7, 2.4))


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\nbye")
