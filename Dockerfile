# Builds the fly.io game server image. Build context is the repo root (the server
# crate compiles `shared/`). Deploy with:
#   fly deploy . --config server/fly.toml -a <app>
# Locally:  docker build -t trickshot-server .

# ---- build stage ----
FROM rust:1-bookworm AS build
WORKDIR /app

# The whole workspace is copied so Cargo can resolve it. `-p server` builds ONLY
# the server package and its deps — the client crate (full Bevy: wgpu, alsa, …)
# is never compiled, so no extra system libraries are needed here.
COPY . .
RUN cargo build --release --locked -p server

# ---- runtime stage ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /app/target/release/server /usr/local/bin/server

ENV PORT=5000
ENV RUST_LOG=info
EXPOSE 5000/udp

ENTRYPOINT ["server"]
