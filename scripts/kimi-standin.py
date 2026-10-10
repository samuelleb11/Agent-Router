#!/usr/bin/env python3
"""A stand-in for an Anthropic-compatible model server, for running Kimi Code without
spending tokens: the fixtures under fixtures/kimi/session/ were recorded against it, and
scripts/check-kimi-hosting.sh drives a real `kimi web` with it.

    scripts/kimi-standin.py <port> [requests.jsonl]

POST */v1/messages, streamed or not:
  - a request with tools, whose last message is not a tool result: a call of the first tool
    that takes a `command` (Kimi Code's Bash), running `echo hello-from-standin`;
  - otherwise: a short text answer.
Each request is appended to the log when one is named, its credentials left out.
"""
import json
import sys
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
LOG = sys.argv[2] if len(sys.argv) > 2 else None
SECRET = {"authorization", "x-api-key", "cookie", "proxy-authorization"}
lock = threading.Lock()


def log(entry):
    if LOG:
        with lock, open(LOG, "a") as f:
            f.write(json.dumps(entry) + "\n")


def shell_call(tools):
    for tool in tools:
        schema = tool.get("input_schema") or {}
        props = schema.get("properties") or {}
        if "command" not in props:
            continue
        args = {"command": "echo hello-from-standin"}
        for name in schema.get("required") or []:
            if name not in args:
                kind = (props.get(name) or {}).get("type")
                args[name] = {"string": "x", "boolean": False, "integer": 10, "number": 10}.get(kind, "x")
        return tool["name"], args
    return None, None


def answered_a_tool(messages):
    content = messages[-1].get("content") if messages else None
    return isinstance(content, list) and any(isinstance(b, dict) and b.get("type") == "tool_result" for b in content)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def send_json(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self.send_json(404, {"type": "error", "error": {"type": "not_found_error", "message": "stand-in: no such path"}})

    def do_POST(self):
        size = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(size) if size else b""
        try:
            req = json.loads(raw)
        except ValueError:
            req = {}
        headers = {k: v for k, v in self.headers.items() if k.lower() not in SECRET}
        log({"method": self.command, "path": self.path, "headers": headers, "body": req})
        if not self.path.split("?")[0].endswith("/messages"):
            return self.send_json(404, {"type": "error", "error": {"type": "not_found_error", "message": "stand-in: no such path"}})

        messages = req.get("messages") or []
        name, args = (None, None)
        if req.get("tools") and not answered_a_tool(messages):
            name, args = shell_call(req["tools"])
        if name:
            block = {"type": "tool_use", "id": "toolu_standin_" + uuid.uuid4().hex[:10], "name": name, "input": args}
            stop = "tool_use"
        else:
            block = {"type": "text", "text": "The command said hello." if answered_a_tool(messages) else "Stand-in answer"}
            stop = "end_turn"
        model = req.get("model", "k3")
        mid = "msg_standin_" + uuid.uuid4().hex[:10]
        usage = {"input_tokens": 1000, "output_tokens": 20, "cache_creation_input_tokens": 0, "cache_read_input_tokens": 500}
        if not req.get("stream"):
            return self.send_json(200, {"id": mid, "type": "message", "role": "assistant", "model": model,
                                        "content": [block], "stop_reason": stop, "stop_sequence": None, "usage": usage})

        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        self.close_connection = True

        def event(name, data):
            self.wfile.write(("event: %s\ndata: %s\n\n" % (name, json.dumps(data))).encode())
            self.wfile.flush()

        event("message_start", {"type": "message_start", "message": {
            "id": mid, "type": "message", "role": "assistant", "model": model, "content": [],
            "stop_reason": None, "stop_sequence": None, "usage": dict(usage, output_tokens=1)}})
        if block["type"] == "text":
            event("content_block_start", {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})
            event("content_block_delta", {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": block["text"]}})
        else:
            start = {"type": "tool_use", "id": block["id"], "name": block["name"], "input": {}}
            event("content_block_start", {"type": "content_block_start", "index": 0, "content_block": start})
            event("content_block_delta", {"type": "content_block_delta", "index": 0,
                                          "delta": {"type": "input_json_delta", "partial_json": json.dumps(block["input"])}})
        event("content_block_stop", {"type": "content_block_stop", "index": 0})
        event("message_delta", {"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": None},
                                "usage": {"output_tokens": 20}})
        event("message_stop", {"type": "message_stop"})


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
