# ── web UI ─────────────────────────────────────────────────────────
FROM oven/bun:1 AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/bun.lock ./
RUN bun install --frozen-lockfile
COPY frontend/ ./
RUN bun run build

# ── backend ────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src/ src/
COPY --from=frontend /app/frontend/dist frontend/dist
RUN cargo build --release

# ── runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=backend /app/target/release/kou-router /usr/local/bin/kou-router

ENV KOU_ROUTER_BIND=0.0.0.0:20128
ENV KOU_ROUTER_DATABASE_URL=sqlite:///data/kou-router.db
# There is no TTY for the first-run password prompt, so the admin password
# must be supplied via env: docker run -e KOU_ROUTER_ADMIN_PASSWORD=...
VOLUME /data
EXPOSE 20128
# codex oauth callback listener (spawned on demand during authorization)
EXPOSE 1455

ENTRYPOINT ["kou-router"]
CMD ["serve"]
