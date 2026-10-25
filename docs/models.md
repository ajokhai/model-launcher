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

## Where the catalog's models come from

Every model in Discover downloads straight from [Hugging Face](https://huggingface.co), from the lab that made it or a well-known converter. Each card's **Source** line links the original model, the repo the files come from, and the license. The full list is in the README's [Credits and model licenses](../README.md#credits-and-model-licenses). Licenses differ: Apache 2.0 and MIT models can be used for anything, while Gemma, Llama 3.2, Stability AI and OuteTTS (non-commercial) have their own terms.

## Bring your own model

You're not limited to Discover. Any open model in a format these engines read works:

| Kind | Format | Engine |
|---|---|---|
| Chat (and vision) | `.gguf`, plus `mmproj-*.gguf` for vision | llama.cpp: Llama, Qwen, Mistral, Phi, Gemma, DeepSeek, gpt-oss and many more |
| Speech to text | Whisper `ggml-*.bin` | whisper.cpp |
| Text to speech | OuteTTS `.gguf` plus `WavTokenizer*.gguf` | llama.cpp |
| Images | `.gguf` or `.safetensors` (SD 1.x, 2.x, SDXL, Turbo) | stable-diffusion.cpp |
| Video | Wan: the model `.gguf`, a `umt5` text encoder and a `vae` file in one folder | stable-diffusion.cpp |

In Library, press **+ Add a model** and pick one of three ways.

### 1. From a link (walkthrough)

Here's how to add Mistral 7B, which isn't in Discover:

1. Go to [huggingface.co/models](https://huggingface.co/models?library=gguf&sort=trending) and search for `Mistral 7B Instruct GGUF`. Prefer the lab's own GGUF repo, or a trusted converter such as [bartowski](https://huggingface.co/bartowski), [unsloth](https://huggingface.co/unsloth) or [ggml-org](https://huggingface.co/ggml-org).
2. Open the repo's **Files and versions** tab. You'll see the same model at several sizes. Pick **Q4_K_M**: it's the usual balance of size and quality. As a rule of thumb, the file should be well under your memory budget (Settings › Memory).
3. Click the file and copy the page's address, for example `https://huggingface.co/bartowski/Mistral-7B-Instruct-v0.3-GGUF/blob/main/Mistral-7B-Instruct-v0.3-Q4_K_M.gguf`. The page link and the download (`/resolve/`) link both work.
4. In **+ Add a model › From a link**, paste it. For a vision model, paste its `mmproj-*.gguf` link on a second line.
5. Check the name (it's filled in from the file) and the kind, then press **Add**. A progress bar shows in Library; when it finishes the model appears with a **Chat** button.
6. Read the model's license on its page before using it commercially.

Model Launcher checks for disk space before downloading, saves the files to `~/Models/Chat/<name>/`, and records the link in a `source.json` next to them, so the card credits where the model came from.

### 2. From files already on this Mac

Downloaded a model already, or have one from LM Studio or another app? Choose **From this Mac › Choose files…** and pick the files. They're **moved** into `~/Models/<Kind>/<name>/` rather than copied, so they don't take up the space twice. (Files on another disk are copied.)

### 3. By hand

Make a folder for the model and put its files inside. Model Launcher reads it by file name:

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
