# Getting started

1. **Install.** Open `Model Launcher.dmg` and drag the app to Applications. You need an Apple Silicon Mac with macOS 13 or later.
2. **Get a model.** Open **Discover**. Each model has a badge that says how well it fits your memory budget. Start with one marked **Runs great**: Qwen3-VL 4B or Qwen3-VL 8B for chat, Whisper Base for speech to text, OuteTTS for speech, SD Turbo for images. Press **Download**.
3. **Chat.** In **Library**, press **Chat** on a chat model. It loads in a few seconds and its chat window opens. Vision models (marked **Reads images**) accept pictures in the chat.
4. **Voice and images.** Open **Studio** to type something and hear it spoken, transcribe an audio file, or describe an image to make.
5. **Use it from other apps.** **Connect** has the API address and one-click MCP setup for Claude, Claude Code, Cursor and Windsurf.

## Engines

Models run on open-source engines. Model Launcher installs each one the first time it's needed. You can also install them in **Settings › Engines**.

| Engine | Used for | Installed from |
|---|---|---|
| llama.cpp | Chat, text to speech | Homebrew (`brew install llama.cpp`) |
| whisper.cpp | Speech to text | Homebrew (`brew install whisper-cpp`) |
| stable-diffusion.cpp | Images | Its official macOS release, into `~/Library/Application Support/Model Launcher/engines` |

Homebrew engines need [Homebrew](https://brew.sh).

## Closing and quitting

Closing the window keeps Model Launcher (and a loaded model) running in the Dock, so the API stays available. **Quit** (Cmd+Q) stops every model and frees its memory.
