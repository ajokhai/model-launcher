# MCP

Model Launcher works with MCP in both directions.

## Your local models as tools for other AI apps

The app ships an MCP server (stdio) at:

```
/Applications/Model Launcher.app/Contents/MacOS/model-launcher-mcp
```

The easiest way to add it is **Connect › Connect** next to Claude, Claude Code, Cursor or Windsurf. That adds a `model-launcher` entry to the app's MCP config and leaves the rest of the file as it was. Restart the app, or start a new chat, to see the tools.

To add it by hand to any MCP client:

```json
{
  "mcpServers": {
    "model-launcher": {
      "command": "/Applications/Model Launcher.app/Contents/MacOS/model-launcher-mcp"
    }
  }
}
```

| Tool | What it does |
|---|---|
| `list_models` | Installed models, their kind and fit, and the loaded chat model |
| `chat` | Ask a local chat model (`prompt`, optional `model`, `system`, `max_tokens`) |
| `transcribe` | Speech to text from an audio file `path` |
| `speak` | Text to speech; writes a WAV and returns its path (`play: true` also plays it) |
| `generate_image` | Makes an image and returns the PNG's path (`prompt`, optional `size`, `steps`) |
| `load_model` / `unload_model` | Swap the chat model in or out ahead of time |

If Model Launcher isn't running, the MCP server opens it in the background.

## MCP servers for your local models

A model's chat window is llama.cpp's web UI, which has an **MCP** panel. Add an MCP server there (for example [computer-use-lite](https://github.com/ajokhai/computer-use-lite)) and the local model can call its tools. Tool use works best with models trained for it, such as Qwen3 and Qwen3-VL.
