import json
import os
import sys
import time

mode = os.environ.get("AGT_MCP_TEST_MODE", "normal")
pid_file = os.environ.get("AGT_MCP_PID_FILE")
if pid_file:
    with open(pid_file, "w", encoding="utf-8") as handle:
        handle.write(str(os.getpid()))
page = 0

for raw_line in sys.stdin:
    message = json.loads(raw_line)
    method = message.get("method", "")
    if method == "initialize":
        if mode == "stderr-secret":
            sys.stderr.write("TOP_SECRET=do-not-leak\n" * 4)
            sys.stderr.flush()
            time.sleep(0.05)
        version = (
            "2024-11-05"
            if mode in ("bad-version", "stderr-secret")
            else "2025-11-25"
        )
        capabilities = (
            {}
            if mode == "missing-tools"
            else {
                "tools": {"listChanged": True},
                "vendor": {"enabled": True},
            }
        )
        response = {
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "protocolVersion": version,
                "capabilities": capabilities,
                "serverInfo": {
                    "name": "helper",
                    "version": "0.1.0",
                    "title": "Helper",
                    "icons": [
                        {
                            "src": "data:image/png;base64,AA==",
                            "mimeType": "image/png",
                        }
                    ],
                    "vendor": "kept",
                },
                "instructions": "Use carefully",
            },
        }
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        page += 1
        if mode == "repeat-cursor":
            next_cursor = "same"
        elif mode == "pages":
            next_cursor = f"page-{page}"
        elif page == 1 and mode in ("normal", "tools-total"):
            next_cursor = "page-2"
        else:
            next_cursor = None
        description = "x" * 64 if mode == "large-description" else "Lookup text"
        schema = (
            {"type": "object", "description": "x" * 64}
            if mode == "large-schema"
            else {"type": "object"}
        )
        output_schema = (
            {"type": "object", "description": "x" * 64}
            if mode == "large-output-schema"
            else {"type": "object"}
        )
        tool = {
            "name": (
                "x" * 64
                if mode == "large-name"
                else ("lookup" if page == 1 else "second-page")
            ),
            "description": description,
            "inputSchema": schema,
            "outputSchema": output_schema,
            "annotations": {"readOnlyHint": True, "vendor": "kept"},
            "icons": [{"src": "data:image/png;base64,AA=="}],
            "_meta": {"vendor": "kept"},
        }
        tools = [tool, dict(tool, name="extra")] if mode == "many-tools" else [tool]
        notification_count = 2 if mode == "event-flood" else 1
        if mode in ("normal", "event-flood") and page == 1:
            for _ in range(notification_count):
                sys.stdout.write(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "method": "notifications/tools/list_changed",
                        }
                    )
                    + "\n"
                )
        result = {"tools": tools}
        if next_cursor is not None:
            result["nextCursor"] = next_cursor
        response = {"jsonrpc": "2.0", "id": message["id"], "result": result}
    elif method == "tools/call":
        text = "x" * 128 if mode == "large-result" else "pong"
        query = ((message.get("params") or {}).get("arguments") or {}).get(
            "query", ""
        )
        response = {
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "content": [{"type": "text", "text": text}],
                "structuredContent": {"echo": query},
                "isError": False,
            },
        }
    else:
        response = {
            "jsonrpc": "2.0",
            "id": message.get("id"),
            "error": {"code": -32601, "message": "secret omitted"},
        }
    sys.stdout.write(json.dumps(response) + "\n")
    sys.stdout.flush()
