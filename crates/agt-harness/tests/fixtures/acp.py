#!/usr/bin/env python3
"""Deterministic ACP v1 agent fixture shaped after DeepSeek Harness; never contacts a model.

Prompt words choose the scenario: TOOL calls the first agt tool over the mounted
HTTP MCP server, PERMISSION asks to run a command, HANG waits for session/cancel,
FAIL stops with a refusal.
"""
import json, sys, pathlib, urllib.request

root = pathlib.Path.cwd()
(root / "args.json").write_text(json.dumps(sys.argv[1:]))
MODELS = [["deepseek-official", "deepseek-v4-flash"], ["deepseek-official", "deepseek-v4-pro"]]
state = {"model": json.dumps(MODELS[0]), "effort": "high"}
server = None


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


def options():
    return [
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": state["model"],
         "options": [{"group": "deepseek-official", "name": "DeepSeek", "options": [
             {"value": json.dumps(m), "name": m[1]} for m in MODELS]}]},
        {"id": "reasoning_effort", "name": "Reasoning effort", "category": "thought_level", "type": "select",
         "currentValue": state["effort"],
         "options": [{"value": v, "name": v.title()} for v in ["off", "low", "high", "max"]]},
    ]


def mcp(message):
    request = urllib.request.Request(server["url"], data=json.dumps(message).encode(), method="POST",
                                     headers={"content-type": "application/json",
                                              "authorization": dict((h["name"], h["value"]) for h in server["headers"])["authorization"]})
    with urllib.request.urlopen(request, timeout=10) as response:
        body = response.read()
        return json.loads(body) if body else None


def mount(servers):
    global server
    server = next(s for s in servers if s.get("type") == "http")
    mcp({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}}})
    mcp({"jsonrpc": "2.0", "method": "notifications/initialized"})
    tools = mcp({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})["result"]["tools"]
    (root / "tools.json").write_text(json.dumps(tools))
    return tools


def update(session, value):
    emit({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session, "update": value}})


tools = []
while True:
    v = read()
    method, params, rid = v.get("method"), v.get("params", {}), v.get("id")
    if method == "initialize":
        emit({"jsonrpc": "2.0", "id": rid, "result": {"protocolVersion": 1,
              "agentInfo": {"name": "deepseek-harness-acp", "version": "0.0.1"},
              "agentCapabilities": {"mcpCapabilities": {"http": True},
                                    "sessionCapabilities": {"close": {}, "list": {}, "resume": {}}},
              "authMethods": []}})
    elif method == "session/new":
        tools = mount(params["mcpServers"])
        emit({"jsonrpc": "2.0", "id": rid, "result": {"sessionId": "acp-session-1", "configOptions": options()}})
    elif method == "session/resume":
        tools = mount(params["mcpServers"])
        emit({"jsonrpc": "2.0", "id": rid, "result": {"configOptions": options()}})
    elif method == "session/set_config_option":
        state["model" if params["configId"] == "model" else "effort"] = params["value"]
        emit({"jsonrpc": "2.0", "id": rid, "result": {"configOptions": options()}})
    elif method == "session/close":
        emit({"jsonrpc": "2.0", "id": rid, "result": {}})
    elif method == "session/prompt":
        session = params["sessionId"]
        text = params["prompt"][0]["text"]
        update(session, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "working "}})
        notes = []
        if "TOOL" in text:
            result = mcp({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                          "params": {"name": tools[0]["name"], "arguments": {"kind": "agent"}}})
            notes.append("tool:" + result["result"]["content"][0]["text"])
        if "PERMISSION" in text:
            update(session, {"sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "bash", "kind": "execute", "status": "pending"})
            emit({"jsonrpc": "2.0", "id": "perm-1", "method": "session/request_permission", "params": {
                "sessionId": session, "toolCall": {"toolCallId": "call-1", "title": "bash"},
                "options": [{"optionId": "allow", "name": "Allow once", "kind": "allow_once"},
                            {"optionId": "reject", "name": "Reject", "kind": "reject_once"}]}})
            answer = read()
            outcome = answer["result"]["outcome"]
            notes.append("permission:" + outcome.get("optionId", outcome["outcome"]))
            update(session, {"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed"})
        if "HANG" in text:
            (root / "hanging").write_text("1")
            while True:
                cancel = read()
                if cancel.get("method") == "session/cancel":
                    emit({"jsonrpc": "2.0", "id": rid, "result": {"stopReason": "cancelled"}})
                    break
            continue
        if "FAIL" in text:
            emit({"jsonrpc": "2.0", "id": rid, "result": {"stopReason": "refusal"}})
            continue
        model = json.loads(state["model"])[1]
        update(session, {"sessionUpdate": "agent_message_chunk", "content": {"type": "text",
               "text": "done " + " ".join(notes) + f" model:{model} session:{session}"}})
        emit({"jsonrpc": "2.0", "id": rid, "result": {"stopReason": "end_turn"}})
    elif rid is not None:
        emit({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "unknown"}})
