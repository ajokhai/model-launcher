# Building from source

You need Rust (stable), Node.js (only for the Tauri CLI) and Xcode's command line tools.

```bash
git clone https://github.com/ajokhai/model-launcher
cd model-launcher
npm install                  # Tauri CLI
cargo run -p model-launcher  # the app, in debug mode
```

`./packaging/build.sh` builds a release `.app` with the MCP server inside, writes `dist/Model Launcher.dmg`, and installs the app to `/Applications` when given `--install`.

## Layout

| Path | What's there |
|---|---|
| `crates/ml-core/src/catalog.rs` | Downloadable models |
| `crates/ml-core/src/library.rs` | Reading and tidying `~/Models` |
| `crates/ml-core/src/engines.rs` | Starting models, hot swap, the memory watchdog |
| `crates/ml-core/src/gateway.rs` | The local API |
| `crates/ml-core/src/download.rs` | Model downloads and engine installs |
| `crates/ml-core/src/system.rs` | Memory, disk and process measurements (macOS calls) |
| `crates/ml-mcp/src/main.rs` | The MCP server |
| `app/src-tauri/src/main.rs` | The window, chat windows, connecting MCP clients |
| `ui/` | The interface (HTML, CSS, JS, no build step) |

## Principles

- **Keep it fast and small.** Rust for everything that runs, plain JS for the UI, and engines in separate processes.
- **Never surprise the user's memory.** Anything new that runs a model goes through `Engines`, so it gets the budget check, the watchdog and the user's priority and thread settings.
- **One folder.** Models only ever live under the models folder; nothing else writes there.
- **Plain words.** Errors say what happened and what to do next.
