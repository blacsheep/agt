"""Local contract fixture: count side effects; expose a single shared input schema."""
import json
import pathlib
import sys

schema = {
    "type": "object",
    "properties": {"count": {"type": "integer", "minimum": 1, "maximum": 7}},
    "required": ["count"],
    "additionalProperties": False,
}
mode, log = sys.argv[1:]
if mode == "bad-schema":
    schema["properties"]["count"]["type"] = "invalid-type"
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if "id" not in request:
        continue
    if method == "initialize":
        with pathlib.Path(log + ".starts").open("a") as f:
            f.write("start\n")
        result = {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
                  "serverInfo": {"name": "contract", "version": "1"}}
    elif method == "tools/list":
        tool = {"name": "count", "inputSchema": schema}
        if mode == "read-only":
            tool["annotations"] = {"readOnlyHint": True}
        result = {"tools": [tool]}
    elif method == "tools/call":
        args = request["params"]["arguments"]
        assert type(args["count"]) is int and 1 <= args["count"] <= 7
        with pathlib.Path(log).open("a") as f:
            f.write(json.dumps(args) + "\n")
        result = {"content": [{"type": "text", "text": "counted"}]}
    else:
        raise AssertionError(method)
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
