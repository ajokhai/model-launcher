# Memory and resource limits

Local models are memory hungry. Model Launcher keeps them inside limits you set in **Settings**, so a model never takes over your Mac.

## Settings

| Setting | Default | What it does |
|---|---|---|
| RAM budget | half your RAM | The most memory (RAM, not disk) all running models may use together. Disk space is checked separately, before each download. |
| Chat models at once | Auto | How many chat models can stay loaded together, still within the RAM budget. |
| Context size | 8,192 tokens | How much of a conversation a chat model keeps in memory. Each doubling costs more memory. |
| Unload when idle | 10 minutes | Unloads a chat model nobody has used for this long. It reloads on the next message. |
| CPU threads | Auto (all cores but two) | Threads each engine may use. Fewer leaves more for your other apps. |
| Use the GPU | On | Runs models on the GPU with Metal. Off is slower and leaves the GPU free. |
| Low priority | On | Runs engines at a lower CPU priority so whatever you're doing comes first. |
| Use disk as extra memory | Off | Models bigger than the RAM budget load anyway and read their weights from the drive as needed (llama.cpp maps the model file into memory). Slower; a fast external SSD is best. |
| Internet access | On | Lets chat models search the web (DuckDuckGo, no account) and read pages, through the `web_search` and `web_open_page` tools. Off keeps them fully offline; loaded models reload to apply the change. |

## How it's enforced

1. **Before loading**, Model Launcher estimates what the model needs: its files plus working memory (the KV cache for chat, which grows with context size). It refuses the launch if that's over your budget, or more than is free right now, and says why.
2. **Several chat models at once.** Models stay loaded together while they fit the budget and the **Chat models at once** limit (Auto: 1 under 24 GB of RAM, 2 at 24 GB, 3 at 32 GB, 4 at 48 GB or more). When a new one doesn't fit, the least recently used one is unloaded first.
3. **Voice and image jobs run one at a time.** If a job plus the loaded chat model would exceed the budget, the chat model is unloaded first. It reloads on the next chat.
4. **A watchdog** measures each model's real memory every two seconds (the larger of its footprint and its resident memory, so GPU buffers and mapped weights both count). Anything that grows past the budget, plus 10% grace for loading spikes, is stopped, and Settings › Activity says so.
5. **Idle unload** frees a chat model after the idle time you picked.
6. **Quitting frees everything**, including after a crash: on the next start, a model server left behind is stopped.

Chat models also run with flash attention and an 8-bit KV cache, which roughly halves the memory a long conversation takes, and llama.cpp's prompt cache is capped at 512 MB (it defaults to 8 GB, enough on its own to push a model past a 16 GB Mac's budget).

## The fit badges

| Badge | Meaning |
|---|---|
| Runs great | Needs under 75% of your budget. |
| Fits | Within your budget. |
| Over budget | Your Mac could run it if you raise the budget. |
| Too big | Needs more memory than this Mac has. |

## Models on an external SSD

Settings › Storage lists this Mac and every connected drive. **Move models here** copies every model to `<drive>/Models`, checks each file, then removes the originals; `~/Models` becomes a link to the drive so it's still easy to find. Move them back the same way. While the drive is unplugged, Model Launcher won't download or import (so nothing lands on this Mac by mistake) and says the drive isn't connected.

macOS doesn't let apps put swap on an external drive, so a drive can't literally become RAM. What it can do is hold the model files: with **Use disk as extra memory** on, a model past the RAM budget loads anyway, and macOS reads its weights from the drive as they're needed instead of keeping them all in RAM. Expect slower replies, and keep the model on the fastest drive you have.
