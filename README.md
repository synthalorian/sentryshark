# 🦈 SentryShark

Self-hosted AI code review for GitHub/GitLab PRs. Runs entirely on your infrastructure — no API keys, no data leaves your network.

SentryShark patrols your code like an apex predator. It doesn't miss a thing.

## Features

- **Fully offline** — Local LLMs via llama.cpp, Ollama, or vLLM
- **GitHub & GitLab** — Native webhook support for both
- **Pluggable backends** — Swap models without touching review logic
- **Rust-powered** — Fast, safe, minimal resource footprint

## Quick Start

```bash
# 1. Start your local LLM server
llama-server -m codellama-34b.Q4_K_M.gguf -c 4096 --port 8080

# 2. Configure the bot
cp config.example.toml config.toml
# Edit config.toml with your webhook secrets

# 3. Run
 cargo run --release

# 4. Point webhooks to http://your-host:3000/webhook/github
```

## Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   GitHub    │────▶│  Webhook    │────▶│  Diff       │
│   GitLab    │     │  Server     │     │  Extractor  │
└─────────────┘     └─────────────┘     └──────┬──────┘
                                                │
                                       ┌────────▼────────┐
                                       │  Local LLM      │
                                       │  (llama.cpp)    │
                                       └────────┬────────┘
                                                │
                                       ┌────────▼────────┐
                                       │  Review Poster  │
                                       └─────────────────┘
```

## Environment Variables

| Variable | Description |
|----------|-------------|
| `CONFIG_PATH` | Path to config.toml |
| `RUST_LOG` | Log level (info/debug/trace) |

## License

MIT — This is the wave. 🎹🦈
