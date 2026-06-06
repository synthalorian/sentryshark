# SentryShark — Development Plan

Self-hosted AI code review bot. Rust + Axum. No cloud, no API keys.

---

## v0.1.0 — Wire the Core (Complete)

- [x] Connect `github.rs` webhook handler to `review.rs` + `llm.rs`
- [x] Implement `ReviewEngine::clone_and_diff` with real git operations
- [x] Add GitHub webhook HMAC-SHA256 signature verification
- [x] Add GitLab webhook secret token verification
- [x] Wire LLM client to send diffs and post review comments
- [x] Write integration tests with mock webhook payloads

## v0.2.0 — Smart Reviews (Complete)

- [x] Filter out generated/lockfile noise from diffs
- [x] Configurable review rules (security, style, performance)
- [x] Batch multiple commits into single review
- [x] Inline comment placement on specific lines
- [x] Review summary comment (approve/request changes)

## v0.3.0 — Production Ready (Complete)

- [x] GitHub App authentication (JWT + installation tokens)
- [x] GitLab CI/CD integration (MR discussions API)
- [x] SQLite database for review history
- [x] Web dashboard for review stats
- [x] Docker compose with llama.cpp sidecar

## v0.4.0 — Observability & Polish (Complete)

- [x] Metrics endpoint (/metrics) with Prometheus-compatible output
- [x] Webhook rate limiting middleware
- [x] Config validation on startup with helpful error messages
- [x] Structured review metrics (latency, error rates, per-repo stats)
- [x] Code quality: remove AppConfig duplication in handlers
- [x] Expanded integration tests for batch processing and metrics
- [x] Health check endpoint returns detailed status (DB, LLM connectivity)

## v0.5.0 — Advanced Review Features

- [x] Review rule engine with custom rule definitions (YAML/TOML)
- [x] Severity levels for findings (critical, warning, info)
- [x] Auto-approve trivial PRs (docs, lockfile-only, whitespace)
- [x] Review templates for different project types (Rust, Python, JS)
- [x] Diff hunk filtering by file type and path patterns
- [x] Review history search and filtering via dashboard
- [x] Webhook retry logic with exponential backoff

## v0.6.0 — Scale & Reliability

- [x] Async review queue with worker pool (not inline)
- [x] Review job persistence in SQLite (survive restarts)
- [x] Concurrent review limits per repository
- [x] Review result caching (skip re-reviewing identical diffs)
- [x] Graceful shutdown with in-flight review completion
- [x] Structured logging with tracing (spans for each review)
- [x] Benchmark suite for review latency under load

## v1.0.0 — Ship It

- [ ] All tests pass, CI green
- [ ] Binary releases for Linux x86_64/ARM64
- [ ] Docker image on GHCR
- [ ] Documentation complete
- [ ] Show HN launch

---

## Architecture

```
GitHub/GitLab webhook → Axum → Verify sig → Rate limit → Clone repo → Extract diff
                                                                  ↓
Post review ← Format response ← LLM response ← Send to local LLM
      ↓
SQLite DB ← Metrics endpoint ← Dashboard
```

## Key Files

| File | Responsibility |
|------|---------------|
| `src/main.rs` | Server setup, routing, middleware |
| `src/github.rs` | GitHub webhook handler |
| `src/gitlab.rs` | GitLab webhook handler |
| `src/llm.rs` | Local LLM client |
| `src/review.rs` | Diff extraction, review formatting, batching |
| `src/config.rs` | TOML configuration, validation |
| `src/db.rs` | SQLite review history |
| `src/dashboard.rs` | Web dashboard HTML + stats API |
| `src/metrics.rs` | Prometheus-compatible metrics |
| `src/rate_limit.rs` | Webhook rate limiting |

## Dependencies

- `tokio` — Async runtime
- `axum` — Web framework
- `reqwest` — HTTP client (LLM API)
- `sha2` + `hmac` — Webhook verification
- `serde` + `toml` — Config parsing
- `rusqlite` — Review history database
- `jsonwebtoken` + `rsa` — GitHub App auth

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

## Docker

```bash
docker-compose up -d
# Access dashboard at http://localhost:3000/dashboard
# Access metrics at http://localhost:3000/metrics
```

---

*This is the wave.* \u{1f988}
