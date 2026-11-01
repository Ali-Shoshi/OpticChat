"""Loopback-only, tool-free inference through the installed Hermes runtime."""
import sys
from pathlib import Path

if "--launch" in sys.argv:
    import json, os, subprocess
    command = json.loads(subprocess.check_output([str(Path.home()/".local/bin/hermes"), "--print-runtime-command"], text=True))
    command[-1] = command[-1].rsplit("runpy.run_module(", 1)[0] + f"runpy.run_path({str(Path(__file__).resolve())!r}, run_name='__main__')"
    os.execv(command[0], command)

import asyncio
import hmac
import json
import secrets
import threading
from aiohttp import web

KEY_PATH = Path.home()/".config/opticchat/hermes.key"
KEY_PATH.parent.mkdir(parents=True, exist_ok=True)
if not KEY_PATH.exists():
    import os
    descriptor = os.open(KEY_PATH, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        output.write(secrets.token_urlsafe(32))
KEY = KEY_PATH.read_text().strip()

def model_name():
    from hermes_cli.config import load_config
    return load_config().get("model", {}).get("default", "gpt-6.1-sol")

@web.middleware
async def authenticate(request, handler):
    if not hmac.compare_digest(request.headers.get("Authorization", ""), "Bearer " + KEY):
        raise web.HTTPUnauthorized(text="Local Hermes bridge authentication required")
    return await handler(request)

async def models(request):
    return web.json_response({"object":"list", "data":[{"id":model_name(), "object":"model"}]})

async def respond(request):
    body = await request.json()
    messages = []
    for item in body.get("input", []):
        content = item.get("content", "")
        if isinstance(content, list):
            content = [({"type":"image_url", "image_url":{"url":part["image_url"]}} if part.get("type")=="input_image" else {"type":"text", "text":part.get("text", "")}) for part in content]
        messages.append({"role":item.get("role", "user"), "content":content})
    if not messages or messages[-1]["role"] != "user":
        raise web.HTTPBadRequest(text="A user question is required")
    stream = web.StreamResponse(headers={"Content-Type":"text/event-stream", "Cache-Control":"no-cache"})
    await stream.prepare(request)
    loop = asyncio.get_running_loop()
    queue = asyncio.Queue()
    cancelled = threading.Event()
    active = []
    def emit(event):
        loop.call_soon_threadsafe(queue.put_nowait, event)
    def run():
        agent = None
        try:
            from hermes_cli.runtime_provider import resolve_runtime_provider
            from run_agent import AIAgent
            selected = body.get("model") or model_name()
            runtime = resolve_runtime_provider(requested="openai-codex", target_model=selected)
            def delta(text):
                if cancelled.is_set():
                    raise RuntimeError("Optic Chat disconnected")
                if text:
                    emit({"type":"response.output_text.delta", "delta":text})
            agent = AIAgent(model=selected, provider=runtime["provider"], api_key=runtime.get("api_key"), base_url=runtime.get("base_url"), api_mode=runtime.get("api_mode"), credential_pool=runtime.get("credential_pool"), enabled_toolsets=[], max_iterations=1, max_tokens=1000, quiet_mode=True, skip_context_files=True, skip_memory=True, skip_background_review=True, stream_delta_callback=delta, save_trajectories=False)
            active.append(agent)
            result = agent.run_conversation(messages[-1]["content"], system_message=body.get("instructions"), conversation_history=messages[:-1])
            if result.get("error"):
                raise RuntimeError(result["error"])
            emit({"type":"response.completed"})
        except Exception as error:
            emit({"type":"error", "message":str(error)})
        finally:
            if agent is not None:
                agent.close()
            emit(None)
    async with request.app["inference"]:
        task = asyncio.create_task(asyncio.to_thread(run))
        try:
            while True:
                event = await queue.get()
                if event is None:
                    break
                await stream.write(("data: " + json.dumps(event) + "\n\n").encode())
        finally:
            cancelled.set()
            if not task.done() and active:
                active[0].interrupt()
            await task
    return stream

app = web.Application(middlewares=[authenticate], client_max_size=12*1024*1024)
app["inference"] = asyncio.Semaphore(1)
app.router.add_get("/v1/models", models)
app.router.add_post("/v1/responses", respond)
web.run_app(app, host="127.0.0.1", port=8642, print=None, access_log=None)
