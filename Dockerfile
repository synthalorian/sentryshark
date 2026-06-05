FROM rust:1.75-slim-bookworm as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y git ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/code-review-bot /usr/local/bin/
COPY config.example.toml ./config.toml

EXPOSE 3000
CMD ["sentryshark"]
