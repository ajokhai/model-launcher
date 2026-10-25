<p align="center"><img src="packaging/icon.png" width="128" alt=""></p>

<h1 align="center">Model Launcher</h1>

<p align="center">Launch open-source chat, voice and image models on your Mac as easily as opening an app.<br>
Fast Rust core · memory limits you control · OpenAI-compatible API · MCP server</p>

---

Model Launcher keeps your local models in one folder, shows which ones your Mac can run well, and starts one with a click. Any app that speaks the OpenAI API or MCP (Claude, Cursor, VS Code and others) can use the models it runs.

- **Chat** with Qwen, Gemma, Llama, gpt-oss and any other `.gguf` model ([llama.cpp](https://github.com/ggml-org/llama.cpp)), including vision models that read images.
- **Voice**: speech to text with Whisper ([whisper.cpp](https://github.com/ggml-org/whisper.cpp)) and text to speech with OuteTTS (llama.cpp).
- **Images** with SD Turbo and Stable Diffusion ([stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp)).
- **Hot swap.** Pick another model, in the app or by naming it in an API call, and it replaces the loaded one.
- **It won't run away with your memory.** You set a memory budget. Launches that don't fit are refused, a watchdog stops anything that grows past the budget, idle models unload, and you choose CPU threads, GPU use and priority.
- **One tidy folder.** Every model lives in `~/Models/{Chat,Voice,Image}/<model>/`.

## Install

Download the latest `Model Launcher.dmg` from [Releases](https://github.com/ajokhai/model-launcher/releases), open it and drag the app to Applications. Apple Silicon, macOS 13 or later.

The first time a model needs an engine, Model Launcher installs it: llama.cpp and whisper.cpp through [Homebrew](https://brew.sh), and stable-diffusion.cpp from its official release.

## Use it from other apps

**API** (OpenAI-compatible, on this Mac only):

```bash
curl http://127.0.0.1:11500/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "qwen3-vl-8b", "messages": [{"role": "user", "content": "Hello!"}]}'
```

```python
from openai import OpenAI
client = OpenAI(base_url="http://127.0.0.1:11500/v1", api_key="local")
```

Endpoints: `/v1/models`, `/v1/chat/completions` (streaming too), `/v1/audio/transcriptions`, `/v1/audio/speech`, `/v1/images/generations`.

**MCP**: in the app, open **Connect** and press **Connect** next to Claude, Claude Code, Cursor or Windsurf. Or add it to any MCP client by hand:

```json
{ "mcpServers": { "model-launcher": { "command": "/Applications/Model Launcher.app/Contents/MacOS/model-launcher-mcp" } } }
```

Tools: `list_models`, `chat`, `transcribe`, `speak`, `generate_image`, `load_model`, `unload_model`.

## Docs

- [Getting started](docs/getting-started.md)
- [Models and the ~/Models folder](docs/models.md)
- [Memory and resource limits](docs/resources.md)
- [API reference](docs/api.md)
- [MCP](docs/mcp.md)
- [Building from source](docs/development.md)

## How it's built

| Part | What it does |
|---|---|
| `crates/ml-core` | Rust. Model library, downloads, process supervisor with the memory watchdog, and the local API (axum). |
| `crates/ml-mcp` | Rust. The MCP server (stdio). Its only dependency is serde_json. |
| `app/src-tauri` | Rust + [Tauri](https://tauri.app). The native window, chat windows, MCP client setup. |
| `ui/` | The window's interface: plain HTML, CSS and JS. No build step. |

Models run in their own processes (llama.cpp, whisper.cpp, stable-diffusion.cpp), so a model can never crash the app, and quitting the app always frees their memory.

## Contributing

Issues and pull requests are welcome: new catalog models, Windows and Linux support, more engines (MLX, Kokoro TTS, FLUX) and UI polish. See [docs/development.md](docs/development.md).

## License

MIT. Models have their own licenses; check each one before commercial use.
