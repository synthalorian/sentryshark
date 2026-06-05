# SentryShark — Development Plan

Self-hosted AI code review bot. Rust + Axum. No cloud, no API keys.

---

## v0.1.0 — Wire the Core (Now)

- [ ] Connect `github.rs` webhook handler to `review.rs` + `llm.rs`
- [ ] Implement `ReviewEngine::clone_and_diff` with real git operations
- [ ] Add GitHub webhook HMAC-SHA256 signature verification
- [ ] Add GitLab webhook secret token verification
- [ ] Wire LLM client to send diffs and post review comments
- [ ] Write integration tests with mock webhook payloads

## v0.2.0 — Smart Reviews

- [ ] Filter out generated/lockfile noise from diffs
- [ ] Configurable review rules (security, style, performance)
- [ ] Batch multiple commits into single review
- [ ] Inline comment placement on specific lines
- [ ] Review summary comment (approve/request changes)

## v0.3.0 — Production Ready

- [ ] GitHub App authentication (JWT + installation tokens)
- [ ] GitLab CI/CD integration (MR discussions API)
- [ ] SQLite database for review history
- [ ] Web dashboard for review stats
- [ ] Docker compose with llama.cpp sidecar

## v1.0.0 — Ship It

- [ ] All tests pass, CI green
- [ ] Binary releases for Linux x86_64/ARM64
- [ ] Docker image on GHCR
- [ ] Documentation complete
- [ ] Show HN launch

---

## Architecture

```
GitHub/GitLab webhook → Axum → Verify sig → Clone repo → Extract diff
                                                            ↓
Post review ← Format response ← LLM response ← Send to local LLM
```

## Key Files

| File | Responsibility |
|------|---------------|
| `src/main.rs` | Server setup, routing |
| `src/github.rs` | GitHub webhook handler |
| `src/gitlab.rs` | GitLab webhook handler |
| `src/llm.rs` | Local LLM client |
| `src/review.rs` | Diff extraction, review formatting |
| `src/config.rs` | TOML configuration |

## Dependencies

- `tokio` — Async runtime
- `axum` — Web framework
- `reqwest` — HTTP client (LLM API)
- `sha2` + `hmac` — Webhook verification
- `serde` + `toml` — Config parsing

## Local Dev

```bash
cargo run
# In another terminal:
curl -X POST http://localhost:3000/webhook/github \
  -H "Content-Type: application/json" \
  -d '{"action":"opened","pull_request":{"number":1,"title":"test"},"repository":{"full_name":"test/repo"}}'
```

## Testing

```bash
cargo test
cargo clippy -- -D warnings
cargo build --release
```

---

*This is the wave.* 🦈
