# Builds the fly.io game server image. Build context is the repo root (the server
# crate compiles `shared/`). Deploy with:  cd server && fly deploy ..
# Locally:  docker build -t trickshot-server .

# ---- build stage ----
FROM rust:1-bookworm AS build
WORKDIR /app

# The whole workspace is copied so Cargo can resolve it, but only the `server`
# package (and its dependency `shared`) is actually compiled. Big client assets
# are kept out of the build context by server/.dockerignore.
COPY . .
RUN cargo build --release --bin server

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
