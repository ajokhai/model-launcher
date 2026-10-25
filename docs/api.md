# API reference

Model Launcher serves an OpenAI-compatible API at `http://127.0.0.1:11500/v1` while it's running. Only this Mac can reach it, and web pages can't use it (requests from a browser origin are refused). Any API key works.

Naming a model in `model` loads it, swapping out the loaded one. Leave `model` out (or send `"default"`) to use the loaded chat model, or the first one installed. Model ids are folder names in `~/Models`; `GET /v1/models` lists them.

## Chat

`POST /v1/chat/completions` takes the standard OpenAI chat request, streaming included. It's passed through to llama.cpp, so tools, JSON mode and images (for vision models, as `image_url` content parts) work as they do there.

```bash
curl http://127.0.0.1:11500/v1/chat/completions -H "Content-Type: application/json" \
  -d '{"model": "qwen3-vl-8b", "messages": [{"role": "user", "content": "Write a haiku about RAM."}], "stream": true}'
```

## Speech to text

`POST /v1/audio/transcriptions` is a multipart form with `file` (wav, mp3, m4a, aiff and other formats macOS can read) and an optional `model`. It returns `{"text": "..."}`.

```bash
curl http://127.0.0.1:11500/v1/audio/transcriptions -F file=@memo.m4a -F model=whisper-base-en
```

## Text to speech

`POST /v1/audio/speech` takes `{"input": "...", "model": "outetts-0.2"}` and returns `audio/wav`. The `x-output-path` header says where the file was saved.

```bash
curl http://127.0.0.1:11500/v1/audio/speech -H "Content-Type: application/json" \
  -d '{"input": "Hello from my Mac"}' -o hello.wav
```

## Images

`POST /v1/images/generations` takes `{"prompt": "...", "model": "sd-turbo", "size": "512x512"}`, plus optional `steps`, `seed` and `negative_prompt`. It returns `{"data": [{"b64_json": "...", "path": "..."}]}`. Sizes run from 256 to 1024 in steps of 64. Turbo models default to 4 steps, others to 20.

## Video

`POST /v1/videos/generations` takes `{"prompt": "...", "model": "wan2.1-t2v-1.3b"}`, plus optional `seconds` (1 to 5, default 2), `size` (default `480x272`), `steps`, `seed` and `negative_prompt`. It returns `{"data": [{"b64_webm": "...", "path": "...", "frames": 33, "size": "480x272"}]}`. This endpoint isn't part of OpenAI's API. Video is slow on a laptop: expect minutes per clip.

## Model Launcher's own endpoints

These are what the app's window and the MCP server use.

| Endpoint | What it does |
|---|---|
| `GET /api/status` | System memory and disk, settings, the loaded model and its memory, the running job, recent activity, engines, downloads |
| `GET /api/library` | Installed models, with estimated memory need and fit |
| `GET /api/catalog` | Downloadable models, with fit and whether they're installed |
| `POST /api/launch {"id","force"?}` | Load a chat model; returns its chat page URL. Without enough RAM it answers 507 (`insufficient_memory`); `"force": true` loads it anyway, past the budget |
| `POST /api/stop {"id"?}` | Unload one chat model, or all of them |
| `POST /api/import/url {"name","kind","urls"}` | Download a model from any links into `~/Models` |
| `POST /api/import/file {"name","kind","paths"}` | Move model files already on this Mac into `~/Models` |
| `POST /api/download {"id"}`, `/api/download/cancel` | Download a catalog model, or cancel it |
| `POST /api/delete {"id"}` | Delete a model's folder |
| `POST /api/settings {...}` | Change settings (any subset of the fields in `/api/status`) |
| `POST /api/engines/install {"engine"}` | Install `llama_server`, `llama_tts`, `whisper_cli` or `sd_cli` |
| `POST /api/reveal {"id"}` | Show a model's folder in Finder |
| `POST /api/web/search {"query","max_results"?}` | Web search results (title, url, snippet). Refused when Internet access is off |
| `POST /api/web/fetch {"url"}` | A web page as plain text. Local and private addresses are refused |

Errors look like OpenAI's: `{"error": {"message": "..."}}`, with the reason in plain words (for example, that a model needs more memory than your budget allows).
