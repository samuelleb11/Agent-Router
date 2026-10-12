"""Official SDK contract test. Only the loopback fixture URL is accepted."""
import json
import os
from urllib.parse import urlparse

import httpx2 as httpx
import openai
from openai import OpenAI

url = os.environ["AR_SDK_URL"]
assert urlparse(url).hostname == "127.0.0.1"
assert openai.__version__ == "3.28.0"
transport = httpx.Client(trust_env=False)
client = OpenAI(base_url=url, api_key=os.environ["AR_SDK_TOKEN"], max_retries=0,
                timeout=10, http_client=transport)
models = client.models.list()
assert models.object == "list"
assert {"claude-sdk-test", "gpt-sdk-test", "kimi-for-coding"} <= {m.id for m in models}
for model in models:
    assert model.object == "model" and isinstance(model.created, int)
    assert model.owned_by in ("anthropic", "openai", "kimi")

old_token = os.environ.get("AR_SDK_OLD_TOKEN")
if old_token:
    old = OpenAI(base_url=url, api_key=old_token, max_retries=0, timeout=10,
                 http_client=httpx.Client(trust_env=False))
    for call in (lambda: old.models.list(),
                 lambda: old.responses.create(model="claude-sdk-test", input="hello"),
                 lambda: old.chat.completions.create(model="claude-sdk-test", messages=[{"role":"user","content":"hello"}])):
        try:
            call()
            raise AssertionError("Revoked token succeeded")
        except openai.AuthenticationError:
            pass
    assert client.responses.create(model="gpt-sdk-test", input="hello").output_text == "SDK_OK"
    print("SDK_ROTATION_OK")
    raise SystemExit(0)

schema = {"type":"object", "properties":{"ok":{"type":"boolean"}},
          "required":["ok"], "additionalProperties":False}
function = {"name":"read", "description":"Read a file", "parameters":{
    "type":"object", "properties":{"path":{"type":"string"}}, "required":["path"],
    "additionalProperties":False}, "strict":True}
cases = 0
for model in ("claude-sdk-test", "gpt-sdk-test", "kimi-for-coding"):
    for streaming in (False, True):
        for kind in ("text", "tool", "structured"):
            prompt = "TOOL_REQUEST" if kind == "tool" else "hello"
            source_history = [{"role":"developer", "content":"SDK_BASE_CONTEXT"},
                              {"role":"user", "content":prompt},
                              {"role":"developer", "content":"SDK_LATE_CONTEXT"}]
            options = {"model":model, "input":source_history, "stream":streaming, "store":False}
            if kind == "tool":
                options["tools"] = [{"type":"function", **function}]
            if kind == "structured":
                options["text"] = {"format":{"type":"json_schema", "name":"answer", "schema":schema, "strict":True}}
            response = client.responses.create(**options)
            if streaming:
                events = list(response)
                completed = [e for e in events if e.type == "response.completed"]
                assert len(completed) == 1
                response = completed[0].response
                if kind == "tool":
                    assert any(e.type == "response.function_call_arguments.delta" for e in events)
                else:
                    assert "".join(e.delta for e in events if e.type == "response.output_text.delta") == ("{\"ok\":true}" if kind == "structured" else "SDK_OK")
            assert response.status == "completed" and response.model == model
            assert response.usage.input_tokens == 37 and response.usage.output_tokens == 11
            if kind == "tool":
                call = response.output[0]
                assert call.type == "function_call" and call.name == "read"
                assert json.loads(call.arguments) == {"path":"fixture"}
                # Stateless continuation: send the original input, call and output.
                history = source_history + [call.model_dump(exclude_none=True),
                           {"type":"function_call_output", "call_id":call.call_id, "output":"SDK_TOOL_RESULT"},
                           {"role":"developer", "content":"SDK_LATE_TOOL_POLICY"}]
                continued = client.responses.create(model=model, input=history, tools=[{"type":"function", **function}], store=False)
                assert continued.output_text == "SDK_OK"
            else:
                assert response.output_text == ("{\"ok\":true}" if kind == "structured" else "SDK_OK")

            options = {"model":model, "messages":[{"role":"user", "content":prompt}], "stream":streaming}
            if streaming:
                options["stream_options"] = {"include_usage":True}
            if kind == "tool":
                options["tools"] = [{"type":"function", "function":function}]
            if kind == "structured":
                options["response_format"] = {"type":"json_schema", "json_schema":{"name":"answer", "schema":schema, "strict":True}}
            chat = client.chat.completions.create(**options)
            if streaming:
                chunks = list(chat)
                choices = [c.choices[0] for c in chunks if c.choices]
                assert choices[-1].finish_reason == ("tool_calls" if kind == "tool" else "stop")
                usage = [c.usage for c in chunks if c.usage]
                assert len(usage) == 1 and usage[0].prompt_tokens == 37 and usage[0].completion_tokens == 11
                if kind == "tool":
                    calls = [t for c in choices for t in (c.delta.tool_calls or [])]
                    assert any(t.function.name == "read" for t in calls)
                    args = "".join(t.function.arguments or "" for t in calls)
                    assert json.loads(args) == {"path":"fixture"}
                else:
                    assert "".join(c.delta.content or "" for c in choices) == ("{\"ok\":true}" if kind == "structured" else "SDK_OK")
            else:
                assert chat.model == model
                assert chat.usage.prompt_tokens == 37 and chat.usage.completion_tokens == 11
                message = chat.choices[0].message
                if kind == "tool":
                    call = message.tool_calls[0]
                    assert call.function.name == "read"
                    assert json.loads(call.function.arguments) == {"path":"fixture"}
                    history = options["messages"] + [message.model_dump(exclude_none=True),
                        {"role":"tool", "tool_call_id":call.id, "content":"SDK_TOOL_RESULT"}]
                    assert client.chat.completions.create(model=model, messages=history, tools=options["tools"]).choices[0].message.content == "SDK_OK"
                else:
                    assert message.content == ("{\"ok\":true}" if kind == "structured" else "SDK_OK")
            cases += 2

    for api in ("responses", "chat"):
        try:
            if api == "responses":
                client.responses.create(model=model, input="UPSTREAM_ERROR")
            else:
                client.chat.completions.create(model=model, messages=[{"role":"user", "content":"UPSTREAM_ERROR"}])
            raise AssertionError("Provider error succeeded")
        except openai.RateLimitError as error:
            assert "Fixture capacity exhausted" in str(error)
        cases += 1
    if model != "gpt-sdk-test":
        for extra in ({"previous_response_id":"resp_stored"},
                      {"tools":[{"type":"web_search_preview"}]},
                      {"input":[{"type":"reasoning","encrypted_content":"opaque","summary":[]}]}):
            try:
                options = {"model":model,"input":"hello", **extra}
                client.responses.create(**options)
                raise AssertionError("Unsupported cross-provider feature succeeded")
            except openai.BadRequestError:
                pass

assert transport.get(url.removesuffix("/v1") + "/api/accounts", headers={"Authorization":"Bearer " + os.environ["AR_SDK_TOKEN"]}).status_code == 403
assert cases == 42, cases
print(f"SDK_MATRIX_OK cases={cases}; three providers, both APIs, streams, tool history, JSON schema, usage and errors")
