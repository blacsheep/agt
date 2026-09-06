import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if "id" not in request:
        continue
    if method == "initialize":
        result = {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "runtime-test", "version": "1"},
        }
    elif method == "tools/list":
        result = {"tools": [{"name": "读取/a__b", "inputSchema": {"type": "object"}}]}
    elif method == "tools/call":
        assert request["params"]["name"] == "读取/a__b"
        assert request["params"]["arguments"] == {"query": "test"}
        result = {
            "content": [{"type": "text", "text": "pong", "_meta": {"secret": "client-secret"}}],
            "_meta": {"secret": "client-secret"},
        }
    else:
        raise AssertionError(method)
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
