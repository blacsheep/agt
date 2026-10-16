#!/usr/bin/env python3
"""Deterministic Claude Code stream-json fixture; never contacts a model.

Input words choose the scenario: TOOL calls the first agt tool, PERMISSION asks
to run Bash, HANG waits until interrupted, FAIL ends the turn with an error.
"""
import json, sys, pathlib, time

root = pathlib.Path.cwd()
args = sys.argv[1:]
(root / "args.json").write_text(json.dumps(args))
session = args[args.index("--resume") + 1] if "--resume" in args else "session-new"
counter = 0


def emit(value):
    print(json.dumps(value), flush=True)


def read():
    line = sys.stdin.readline()
    if not line:
        sys.exit(0)
    value = json.loads(line)
    with (root / "protocol.jsonl").open("a") as log:
        log.write(json.dumps(value) + "\n")
    return value


def ask(request):
    """Send a control request and return the host's response to it."""
    global counter
    counter += 1
    rid = f"cli-{counter}"
    emit({"type": "control_request", "request_id": rid, "request": request})
    while True:
        value = read()
        if value.get("type") == "control_response" and value["response"]["request_id"] == rid:
            return value["response"]
        if value.get("type") == "control_request" and value["request"]["subtype"] == "interrupt":
            interrupted(value)


def mcp(message):
    return ask({"subtype": "mcp_message", "server_name": "agt", "message": message})["response"]["mcp_response"]


def interrupted(value):
    emit({"type": "control_response", "response": {"subtype": "success", "request_id": value["request_id"], "response": {}}})
    emit({"type": "result", "subtype": "error_during_execution", "is_error": True, "session_id": session})
    sys.exit(0)


init = read()
assert init["type"] == "control_request" and init["request"]["subtype"] == "initialize", init
mcp({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}}})
mcp({"jsonrpc": "2.0", "method": "notifications/initialized"})
tools = mcp({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})["result"]["tools"]
(root / "tools.json").write_text(json.dumps(tools))
emit({"type": "control_response", "response": {"subtype": "success", "request_id": init["request_id"], "response": {"commands": []}}})

user = read()
assert user["type"] == "user", user
text = user["message"]["content"][0]["text"]
emit({"type": "system", "subtype": "init", "session_id": session, "model": "claude-fixture"})
emit({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "working "}}})
notes = []
if "TOOL" in text:
    result = mcp({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": tools[0]["name"], "arguments": {"kind": "agent"}}})
    notes.append("tool:" + result["result"]["content"][0]["text"])
if "PERMISSION" in text:
    emit({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "toolu_bash", "name": "Bash", "input": {"command": "ls"}}]}})
    answer = ask({"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "ls"}, "permission_suggestions": []})
    behavior = answer["response"]["behavior"]
    notes.append("permission:" + behavior)
    if answer["response"].get("interrupt"):
        emit({"type": "result", "subtype": "error_during_execution", "is_error": True, "session_id": session})
        sys.exit(0)
    emit({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_bash", "is_error": behavior != "allow"}]}})
if "HANG" in text:
    (root / "hanging").write_text("1")
    while True:
        value = read()
        if value.get("type") == "control_request" and value["request"]["subtype"] == "interrupt":
            interrupted(value)
if "FAIL" in text:
    emit({"type": "result", "subtype": "error_max_turns", "is_error": True, "session_id": session})
    sys.exit(0)
final = "done " + " ".join(notes) + f" session:{session}"
emit({"type": "assistant", "message": {"content": [{"type": "text", "text": final}]}})
emit({"type": "result", "subtype": "success", "is_error": False, "result": final, "session_id": session})
while sys.stdin.readline():
    pass
