#!/usr/bin/env python3
"""Deterministic JSON-RPC peer for connection/ownership failure tests only."""
import json
import sys

if "--version" in sys.argv:
    print("codex-cli 0.162.0-alpha.17.2")
    sys.exit(0)

threads = {}
deferred = None
pending = None


def send(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    params = message.get("params", {})
    if method is None:
        if pending and message["id"] == pending[0]:
            send({"method": "serverRequest/resolved", "params": {"threadId": pending[1], "requestId": message["id"]}})
            pending = None
        continue
    if "id" not in message:
        continue
    if method == "test/disconnect":
        sys.exit(0)
    if method == "test/malformed":
        print("malformed protocol frame", flush=True)
        continue
    if method == "test/deferred":
        deferred = message
        continue
    if method == "test/reverse":
        send({"id": message["id"], "result": {"method": method}})
        send({"id": deferred["id"], "result": {"method": deferred["method"]}})
        continue
    result = {}
    if method == "initialize":
        result = {"userAgent": "agent_router_codex/0.162.0-alpha.17.2"}
    elif method == "thread/start":
        identity = "thread-" + str(len(threads) + 1)
        threads[identity] = {"id": identity, "cwd": params["cwd"], "model": params["model"], "parentThreadId": None, "status": {"type": "idle"}}
        result = {"thread": threads[identity]}
    elif method in ("thread/read", "thread/resume"):
        result = {"thread": threads[params["threadId"]]}
    elif method == "thread/loaded/list":
        result = {"data": list(threads), "nextCursor": None}
    elif method == "thread/settings/update" and params.get("model") == "refuse":
        send({"id": message["id"], "error": {"code": -32602, "message": "model refused by native peer"}})
        continue
    elif method in ("test/approval", "test/foreign-approval", "test/question"):
        identity = params["threadId"]
        pending = (90, identity)
        request = {"threadId": identity, "itemId": "tool-1", "turnId": "turn-1", "command": "touch fixture", "availableDecisions": ["accept", "decline"]}
        if method == "test/question":
            request["questions"] = [{"id": "question-1", "question": "Choose a color", "header": "Color", "isOther": False, "isSecret": False, "options": [{"label": "Blue", "description": "A blue fixture"}]}]
        send({"id": 90, "method": "item/tool/requestUserInput" if method == "test/question" else "item/commandExecution/requestApproval", "params": request})
    send({"id": message["id"], "result": result})
