# Memory and resource limits

Local models are memory hungry. Model Launcher keeps them inside limits you set in **Settings**, so a model never takes over your Mac.

## Settings

| Setting | Default | What it does |
|---|---|---|
| Memory budget | half your RAM | The most memory all running models may use together. |
| Context size | 8,192 tokens | How much of a conversation a chat model keeps in memory. Each doubling costs more memory. |
| Unload when idle | 10 minutes | Unloads a chat model nobody has used for this long. It reloads on the next message. |
| CPU threads | Auto (all cores but two) | Threads each engine may use. Fewer leaves more for your other apps. |
| Use the GPU | On | Runs models on the GPU with Metal. Off is slower and leaves the GPU free. |
| Low priority | On | Runs engines at a lower CPU priority so whatever you're doing comes first. |

## How it's enforced

1. **Before loading**, Model Launcher estimates what the model needs: its files plus working memory (the KV cache for chat, which grows with context size). It refuses the launch if that's over your budget, or more than is free right now, and says why.
2. **One chat model at a time.** Loading another one unloads the current one first (hot swap).
3. **Voice and image jobs run one at a time.** If a job plus the loaded chat model would exceed the budget, the chat model is unloaded first. It reloads on the next chat.
4. **A watchdog** measures each model's real memory every two seconds (the larger of its footprint and its resident memory, so GPU buffers and mapped weights both count). Anything that grows past the budget, plus 10% grace for loading spikes, is stopped, and Settings › Activity says so.
5. **Idle unload** frees a chat model after the idle time you picked.
6. **Quitting frees everything**, including after a crash: on the next start, a model server left behind is stopped.

Chat models also run with flash attention and an 8-bit KV cache, which roughly halves the memory a long conversation takes.

## The fit badges

| Badge | Meaning |
|---|---|
| Runs great | Needs under 75% of your budget. |
| Fits | Within your budget. |
| Over budget | Your Mac could run it if you raise the budget. |
| Too big | Needs more memory than this Mac has. |
