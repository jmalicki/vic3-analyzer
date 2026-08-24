#!/usr/bin/env python3
"""NDJSON stdio MCP smoke client for `vic3-analyzer mcp` (docs/mcp.md, #46).

Speaks JSON-RPC over newline-delimited stdin/stdout:
  initialize → notifications/initialized → tools/list → use_save → …
Optional advisor tools (`campaign_brief`, `preview_delta`) are called only when
present in tools/list so this script works against main today and against
stacked advisor branches without requiring them.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any


PROTOCOL_VERSION = "2025-11-25"
CLIENT_INFO = {"name": "vic3-mcp-smoke", "version": "0.1.0"}

# Domestic-scoped shortage peek used when campaign_brief is unavailable.
FALLBACK_SQL = (
    "SELECT s.state_name, g.good, g.shortage, g.price "
    "FROM states s JOIN goods_by_state g USING (state_id) "
    "WHERE g.shortage > 0 "
    "ORDER BY g.shortage DESC LIMIT 5"
)


class McpSmokeError(RuntimeError):
    """Protocol or tool failure that should exit non-zero."""


class McpClient:
    def __init__(self, proc: subprocess.Popen[str]) -> None:
        self._proc = proc
        self._stdin = proc.stdin
        self._stdout = proc.stdout
        assert self._stdin is not None and self._stdout is not None
        self._next_id = 1
        self._lock = threading.Lock()
        self._stderr_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self._stderr_thread.start()

    def _drain_stderr(self) -> None:
        err = self._proc.stderr
        if err is None:
            return
        for line in err:
            sys.stderr.write(line)

    def close(self) -> None:
        if self._stdin and not self._stdin.closed:
            self._stdin.close()
        try:
            self._proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self._proc.kill()
            self._proc.wait(timeout=5)

    def _send(self, msg: dict[str, Any]) -> None:
        line = json.dumps(msg, separators=(",", ":"))
        self._stdin.write(line + "\n")
        self._stdin.flush()

    def _read_message(self) -> dict[str, Any]:
        while True:
            raw = self._stdout.readline()
            if raw == "":
                code = self._proc.poll()
                raise McpSmokeError(
                    f"MCP server closed stdout unexpectedly (exit={code})"
                )
            line = raw.strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except json.JSONDecodeError as exc:
                raise McpSmokeError(f"invalid NDJSON from server: {line!r}") from exc
            if not isinstance(msg, dict):
                raise McpSmokeError(f"expected JSON object, got {type(msg).__name__}")
            # Skip server notifications / requests; only return responses/errors.
            if "id" in msg and ("result" in msg or "error" in msg):
                return msg
            # notifications have method, no result — ignore

    def request(self, method: str, params: dict[str, Any] | None = None) -> Any:
        with self._lock:
            req_id = self._next_id
            self._next_id += 1
            msg: dict[str, Any] = {
                "jsonrpc": "2.0",
                "id": req_id,
                "method": method,
            }
            if params is not None:
                msg["params"] = params
            self._send(msg)
            while True:
                resp = self._read_message()
                if resp.get("id") != req_id:
                    # Out-of-order / unrelated response — keep waiting.
                    continue
                if "error" in resp:
                    raise McpSmokeError(
                        f"{method} error: {json.dumps(resp['error'], indent=2)}"
                    )
                return resp.get("result")

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        msg: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        self._send(msg)

    def call_tool(self, name: str, arguments: dict[str, Any] | None = None) -> Any:
        result = self.request(
            "tools/call",
            {"name": name, "arguments": arguments or {}},
        )
        if isinstance(result, dict) and result.get("isError"):
            raise McpSmokeError(
                f"tool {name} returned isError: {json.dumps(result, indent=2)}"
            )
        return result


def resolve_bin(positional: str | None) -> list[str]:
    """Return argv to launch the MCP server (binary + `mcp`)."""
    if positional:
        path = Path(positional)
        if path.exists() or "/" in positional or positional.endswith("vic3-analyzer"):
            return [str(path), "mcp"]
        # Allow a full command string only if it already includes mcp? Prefer path.
        return [positional, "mcp"]

    env = os.environ.get("VIC3_MCP_BIN") or os.environ.get("VIC3_ANALYZER_BIN")
    if env:
        return [env, "mcp"]

    found = shutil.which("vic3-analyzer")
    if found:
        return [found, "mcp"]

    raise McpSmokeError(
        "no MCP binary: pass path as first arg, set VIC3_MCP_BIN, or put "
        "vic3-analyzer on PATH"
    )


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="NDJSON MCP smoke: initialize → use_save → campaign_brief|query "
        "(+ optional preview_delta)."
    )
    parser.add_argument(
        "bin",
        nargs="?",
        help="Path to vic3-analyzer (or set VIC3_MCP_BIN). Invoked as `<bin> mcp`.",
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--name", help="use_save stub name (e.g. autosave)")
    group.add_argument(
        "--selector",
        choices=("latest", "latest_autosave", "latest_named"),
        help="use_save selector",
    )
    parser.add_argument(
        "--location",
        choices=("local", "steam_cloud"),
        help="optional use_save location disambiguator",
    )
    parser.add_argument(
        "--preview-rye",
        action="store_true",
        help="if preview_delta exists, sugar building_rye_farm extra_levels=1",
    )
    args = parser.parse_args(argv)
    if not args.name and not args.selector:
        parser.error("one of --name or --selector is required")
    return args


def tool_names(list_result: Any) -> set[str]:
    tools = []
    if isinstance(list_result, dict):
        tools = list_result.get("tools") or []
    names: set[str] = set()
    for t in tools:
        if isinstance(t, dict) and "name" in t:
            names.add(str(t["name"]))
    return names


def extract_text_payload(tool_result: Any) -> Any:
    """Prefer parsed JSON from text content blocks; else return the raw result."""
    if not isinstance(tool_result, dict):
        return tool_result
    content = tool_result.get("content")
    if not isinstance(content, list) or not content:
        return tool_result
    texts: list[str] = []
    for block in content:
        if isinstance(block, dict) and block.get("type") == "text":
            texts.append(str(block.get("text", "")))
    if len(texts) == 1:
        try:
            return json.loads(texts[0])
        except json.JSONDecodeError:
            return texts[0]
    if texts:
        return texts
    return tool_result


def run(args: argparse.Namespace) -> dict[str, Any]:
    cmd = resolve_bin(args.bin)
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    client = McpClient(proc)
    out: dict[str, Any] = {"bin": cmd, "steps": {}}

    try:
        init = client.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": CLIENT_INFO,
            },
        )
        out["steps"]["initialize"] = init
        client.notify("notifications/initialized")

        listed = client.request("tools/list", {})
        names = tool_names(listed)
        out["steps"]["tools_list"] = sorted(names)

        use_args: dict[str, Any] = {}
        if args.name:
            use_args["name"] = args.name
        if args.selector:
            use_args["selector"] = args.selector
        if args.location:
            use_args["location"] = args.location
        use_result = client.call_tool("use_save", use_args)
        out["steps"]["use_save"] = extract_text_payload(use_result)

        if "campaign_brief" in names:
            brief = client.call_tool("campaign_brief", {})
            out["steps"]["campaign_brief"] = extract_text_payload(brief)
        else:
            query = client.call_tool("query", {"sql": FALLBACK_SQL, "format": "json"})
            out["steps"]["query"] = extract_text_payload(query)

        if args.preview_rye and "preview_delta" in names:
            preview = client.call_tool(
                "preview_delta",
                {"building": "building_rye_farm", "extra_levels": 1},
            )
            out["steps"]["preview_delta"] = extract_text_payload(preview)
        elif args.preview_rye:
            out["steps"]["preview_delta"] = {
                "skipped": True,
                "reason": "preview_delta not in tools/list",
            }

        return out
    finally:
        client.close()


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv if argv is not None else sys.argv[1:])
    try:
        result = run(args)
    except McpSmokeError as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, indent=2))
        return 1
    except OSError as exc:
        print(json.dumps({"ok": False, "error": f"failed to spawn MCP: {exc}"}, indent=2))
        return 1
    print(json.dumps({"ok": True, **result}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
