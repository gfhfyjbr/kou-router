#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
import webbrowser
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any

DEFAULT_BASE_URL = "http://127.0.0.1:20128"
DEFAULT_PROVIDER_NAME = "Claude Code OAuth Test"
DEFAULT_LISTEN_HOST = "localhost"
DEFAULT_LISTEN_PORT = 1455
DEFAULT_CALLBACK_PATH = "/auth/callback"
DEFAULT_MANUAL_REDIRECT_URI = "https://platform.claude.com/oauth/code/callback"
REQUEST_TIMEOUT_SECONDS = 15
CALLBACK_TIMEOUT_SECONDS = 300


class ScriptError(RuntimeError):
    pass


@dataclass
class CallbackResult:
    code: str
    state: str


@dataclass
class CallbackCapture:
    event: threading.Event
    result: CallbackResult | None = None
    error: str | None = None


class CallbackServer:
    def __init__(self, host: str, port: int, callback_path: str) -> None:
        self.capture = CallbackCapture(event=threading.Event())
        self._callback_path = callback_path
        self._server = self._build_server(host, port)
        self._thread = threading.Thread(
            target=self._serve,
            name="claudecode-oauth-callback",
            daemon=True,
        )

    @property
    def redirect_uri(self) -> str:
        _, port = self._server.server_address[:2]
        return f"http://localhost:{port}{self._callback_path}"

    def start(self) -> None:
        self._thread.start()

    def wait_for_callback(self, timeout_seconds: int) -> CallbackResult:
        if not self.capture.event.wait(timeout_seconds):
            raise ScriptError(
                "timed out waiting for OAuth callback; rerun and complete browser login within "
                f"{timeout_seconds} seconds"
            )
        if self.capture.error:
            raise ScriptError(self.capture.error)
        if self.capture.result is None:
            raise ScriptError("callback completed without usable OAuth parameters")
        return self.capture.result

    def close(self) -> None:
        self.capture.event.set()
        self._server.server_close()
        self._thread.join(timeout=1)

    def _serve(self) -> None:
        while not self.capture.event.is_set():
            self._server.handle_request()

    def _build_server(self, host: str, port: int) -> HTTPServer:
        capture = self.capture
        callback_path = self._callback_path

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802
                parsed = urllib.parse.urlsplit(self.path)
                params = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
                if parsed.path != callback_path:
                    if any(params.get(key, [""])[0].strip() for key in ("code", "state", "error")):
                        capture.error = (
                            "oauth callback arrived on unexpected path: "
                            f"got {parsed.path!r}, expected {callback_path!r}"
                        )
                        self._send_response(404, f"Unexpected callback path. Expected {callback_path}.\n")
                        capture.event.set()
                    else:
                        self._send_response(404, f"Unexpected path. Expected {callback_path}.\n")
                    return

                code = params.get("code", [""])[0].strip()
                state = params.get("state", [""])[0].strip()
                error = params.get("error", [""])[0].strip()
                error_description = params.get("error_description", [""])[0].strip()

                if error:
                    detail = error_description or error
                    capture.error = f"oauth provider redirected with error: {detail}"
                    self._send_response(400, "OAuth login failed. Return to the terminal for details.\n")
                elif not code or not state:
                    missing = []
                    if not code:
                        missing.append("code")
                    if not state:
                        missing.append("state")
                    capture.error = "malformed OAuth callback: missing " + ", ".join(missing)
                    self._send_response(400, "Malformed OAuth callback. Return to the terminal for details.\n")
                else:
                    capture.result = CallbackResult(code=code, state=state)
                    self._send_response(200, "OAuth callback received. You can return to the terminal.\n")

                capture.event.set()

            def log_message(self, format: str, *args: Any) -> None:  # noqa: A003
                return

            def _send_response(self, status: int, body: str) -> None:
                payload = body.encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

        try:
            server = HTTPServer((host, port), Handler)
        except OSError as err:
            raise ScriptError(f"failed to bind local callback server on {host}:{port}: {err}") from err

        server.timeout = 0.5
        return server


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Test the Claude Code OAuth flow against a running kou-router instance by importing the "
            "Claude OAuth preset, starting OAuth, and completing the flow either via manual callback "
            "URL paste (default) or a localhost callback server."
        )
    )
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL, help=f"Router base URL (default: {DEFAULT_BASE_URL})")
    parser.add_argument(
        "--cookie",
        help=(
            "Management auth cookie value. Accepts either a raw JWT token or a cookie header value such as "
            "'kou_auth=...'."
        ),
    )
    parser.add_argument(
        "--provider-connection-id",
        help="Reuse an existing provider connection instead of importing the Claude OAuth preset",
    )
    parser.add_argument(
        "--provider-name",
        default=DEFAULT_PROVIDER_NAME,
        help=f"Provider name used when importing the Claude OAuth preset (default: {DEFAULT_PROVIDER_NAME})",
    )
    parser.add_argument(
        "--redirect-mode",
        choices=("manual", "localhost"),
        default="manual",
        help="OAuth redirect mode: manual Claude-hosted callback URL paste, or localhost listener (default: manual)",
    )
    parser.add_argument(
        "--manual-redirect-uri",
        default=DEFAULT_MANUAL_REDIRECT_URI,
        help="Redirect URI used for manual mode (default: Claude Code production callback)",
    )
    parser.add_argument(
        "--listen-host",
        default=DEFAULT_LISTEN_HOST,
        help=f"Local host for the OAuth callback server (default: {DEFAULT_LISTEN_HOST})",
    )
    parser.add_argument(
        "--listen-port",
        type=int,
        default=DEFAULT_LISTEN_PORT,
        help=f"Local port for the OAuth callback server (default: {DEFAULT_LISTEN_PORT})",
    )
    parser.add_argument(
        "--callback-path",
        default=DEFAULT_CALLBACK_PATH,
        help=f"Local callback path for the OAuth redirect URI (default: {DEFAULT_CALLBACK_PATH})",
    )
    parser.add_argument(
        "--no-browser",
        action="store_true",
        help="Do not try to open the authorization URL automatically; print it instead",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    base_url = args.base_url.rstrip("/")
    cookie_header = normalize_cookie(args.cookie)
    callback_path = normalize_callback_path(args.callback_path)

    callback_server: CallbackServer | None = None
    if args.redirect_mode == "localhost":
        callback_server = CallbackServer(args.listen_host, args.listen_port, callback_path)
        callback_server.start()

    try:
        provider_connection_id = args.provider_connection_id or import_claudecode_provider(
            base_url=base_url,
            cookie_header=cookie_header,
            provider_name=args.provider_name,
        )

        redirect_uri = (
            callback_server.redirect_uri
            if callback_server is not None
            else normalize_manual_redirect_uri(args.manual_redirect_uri)
        )

        start_response = request_json(
            base_url=base_url,
            path="/api/provider-accounts/oauth/start",
            payload={
                "provider_connection_id": provider_connection_id,
                "redirect_uri": redirect_uri,
            },
            cookie_header=cookie_header,
        )
        authorization_url = require_string(start_response, "authorization_url", "oauth start response")
        print_launch_instructions(authorization_url, no_browser=args.no_browser)

        callback = (
            callback_server.wait_for_callback(CALLBACK_TIMEOUT_SECONDS)
            if callback_server is not None
            else prompt_for_manual_callback(CALLBACK_TIMEOUT_SECONDS)
        )
        account = request_json(
            base_url=base_url,
            path="/api/provider-accounts/oauth/callback",
            payload={"state": callback.state, "code": callback.code},
            cookie_header=cookie_header,
        )

        provider_account_id = require_string(account, "id", "oauth callback response")
        has_access_token = require_bool(account, "has_access_token", "oauth callback response")
        has_refresh_token = require_bool(account, "has_refresh_token", "oauth callback response")

        print(
            "success "
            f"provider_connection_id={provider_connection_id} "
            f"provider_account_id={provider_account_id} "
            f"has_access_token={str(has_access_token).lower()} "
            f"has_refresh_token={str(has_refresh_token).lower()}"
        )
        return 0
    except KeyboardInterrupt:
        print_error("interrupted")
        return 130
    except ScriptError as err:
        print_error(str(err))
        return 1
    finally:
        if callback_server is not None:
            callback_server.close()


def normalize_callback_path(value: str) -> str:
    if not value.startswith("/"):
        raise ScriptError("--callback-path must start with '/'")

    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme or parsed.netloc or parsed.query or parsed.fragment:
        raise ScriptError("--callback-path must be a path like /auth/callback")

    return parsed.path


def normalize_manual_redirect_uri(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme != "https" or not parsed.netloc:
        raise ScriptError("--manual-redirect-uri must be an absolute https:// URL")
    if not parsed.path:
        raise ScriptError("--manual-redirect-uri must include a path")
    if parsed.query or parsed.fragment:
        raise ScriptError("--manual-redirect-uri must not include query or fragment")
    return value

def prompt_for_manual_callback(timeout_seconds: int) -> CallbackResult:
    print(
        "After completing login, copy the full callback URL from the browser address bar "
        f"and paste it here within {timeout_seconds} seconds.",
        file=sys.stderr,
    )
    print("Callback URL:", file=sys.stderr)
    try:
        callback_url = input().strip()
    except EOFError as err:
        raise ScriptError("expected a pasted callback URL on stdin") from err
    if not callback_url:
        raise ScriptError("empty callback URL")
    return parse_callback_url(callback_url)


def parse_callback_url(callback_url: str) -> CallbackResult:
    parsed = urllib.parse.urlsplit(callback_url)
    params = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
    code = params.get("code", [""])[0].strip()
    state = params.get("state", [""])[0].strip()
    error = params.get("error", [""])[0].strip()
    error_description = params.get("error_description", [""])[0].strip()
    if error:
        detail = error_description or error
        raise ScriptError(f"oauth provider redirected with error: {detail}")
    if not code or not state:
        missing: list[str] = []
        if not code:
            missing.append("code")
        if not state:
            missing.append("state")
        raise ScriptError("malformed OAuth callback URL: missing " + ", ".join(missing))
    return CallbackResult(code=code, state=state)


def import_claudecode_provider(*, base_url: str, cookie_header: str | None, provider_name: str) -> str:
    response = request_json(
        base_url=base_url,
        path="/api/providers/import",
        payload={"preset_id": "claude-oauth", "name": provider_name},
        cookie_header=cookie_header,
    )
    return require_string(response, "id", "provider import response")


def request_json(
    *,
    base_url: str,
    path: str,
    payload: dict[str, Any],
    cookie_header: str | None,
) -> dict[str, Any]:
    url = f"{base_url}{path}"
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(url=url, data=body, method="POST")
    request.add_header("Content-Type", "application/json")
    request.add_header("Accept", "application/json")
    if cookie_header:
        request.add_header("Cookie", cookie_header)

    try:
        with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
            raw_body = response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as err:
        raw_body = err.read().decode("utf-8", errors="replace")
        message = extract_error_message(raw_body) or f"HTTP {err.code}"
        if err.code in (400, 401, 403) and "unauthorized" in message.lower():
            raise ScriptError(
                "management auth required or rejected; rerun with --cookie containing a valid kou_auth token"
            ) from err
        raise ScriptError(f"request to {path} failed with HTTP {err.code}: {message}") from err
    except urllib.error.URLError as err:
        reason = err.reason
        detail = str(reason) if reason else str(err)
        raise ScriptError(
            f"failed to reach kou-router at {base_url}; ensure the server is running and reachable ({detail})"
        ) from err

    try:
        data = json.loads(raw_body)
    except json.JSONDecodeError as err:
        raise ScriptError(f"request to {path} returned non-JSON response") from err
    if not isinstance(data, dict):
        raise ScriptError(f"request to {path} returned unexpected JSON payload")
    return data


def extract_error_message(raw_body: str) -> str | None:
    try:
        payload = json.loads(raw_body)
    except json.JSONDecodeError:
        return raw_body.strip() or None

    if not isinstance(payload, dict):
        return raw_body.strip() or None
    error = payload.get("error")
    if isinstance(error, dict):
        message = error.get("message")
        if isinstance(message, str) and message.strip():
            return message.strip()
    if isinstance(error, str) and error.strip():
        return error.strip()
    message = payload.get("message")
    if isinstance(message, str) and message.strip():
        return message.strip()
    return raw_body.strip() or None


def normalize_cookie(raw_cookie: str | None) -> str | None:
    if raw_cookie is None:
        return None
    cookie = raw_cookie.strip()
    if not cookie:
        return None
    if cookie.lower().startswith("cookie:"):
        cookie = cookie.split(":", 1)[1].strip()
    if "=" not in cookie:
        return f"kou_auth={cookie}"
    return cookie


def print_launch_instructions(authorization_url: str, *, no_browser: bool) -> None:
    if no_browser:
        print(f"Open this URL in your browser:\n{authorization_url}")
        return

    opened = webbrowser.open(authorization_url, new=1, autoraise=True)
    if opened:
        print("Opened browser for OAuth login.")
        return

    print("Browser was not opened automatically. Open this URL manually:")
    print(authorization_url)


def require_string(payload: dict[str, Any], key: str, context: str) -> str:
    value = payload.get(key)
    if isinstance(value, str) and value:
        return value
    raise ScriptError(f"{context} did not include a valid '{key}' field")


def require_bool(payload: dict[str, Any], key: str, context: str) -> bool:
    value = payload.get(key)
    if isinstance(value, bool):
        return value
    raise ScriptError(f"{context} did not include a valid '{key}' field")


def print_error(message: str) -> None:
    print(f"error: {message}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
