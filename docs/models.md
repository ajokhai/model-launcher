# Models and the ~/Models folder

Every model lives in one place, `~/Models`, with one folder per model:

```
~/Models/
  Chat/
    qwen3-vl-8b/
      Qwen3VL-8B-Instruct-Q4_K_M.gguf
      mmproj-Qwen3VL-8B-Instruct-Q8_0.gguf
  Voice/
    whisper-base-en/ggml-base.en.bin
    outetts-0.2/OuteTTS-0.2-500M-Q8_0.gguf, WavTokenizer-Large-75-F16.gguf
  Image/
    sd-turbo/sd_turbo-f16-q8_0.gguf
```

Settings, logs, engines and generated files are kept apart in `~/Library/Application Support/Model Launcher`.

## Adding your own model

Make a folder for it and put its files inside. Model Launcher reads it by file name:

| Folder | Files | Becomes |
|---|---|---|
| `Chat/<name>/` | one `.gguf` plus an optional `mmproj-*.gguf` | a chat model (with vision if there's an mmproj) |
| `Voice/<name>/` | a `ggml-*.bin` | a Whisper speech-to-text model |
| `Voice/<name>/` | a TTS `.gguf` plus a `WavTokenizer*.gguf` | a text-to-speech model |
| `Image/<name>/` | a `.gguf` or `.safetensors` | an image model |

The folder name is the model's id in the API and MCP.

Model files dropped loose in `~/Models` (or straight into `Chat`, `Voice` or `Image`) are moved into a folder of their own the next time the app starts.

## Removing a model

Press **Delete** on its card, or delete its folder in Finder. Deleting is permanent, but every catalog model can be downloaded again.

## The catalog

Discover lists models picked for Macs, all as direct Hugging Face downloads. The list is in [`crates/ml-core/src/catalog.rs`](../crates/ml-core/src/catalog.rs), and adding one is a small pull request. Before a download starts, Model Launcher checks there's room on the disk (plus 1 GB to spare).
