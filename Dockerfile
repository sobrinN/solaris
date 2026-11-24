# Stage 1: Build frontend packages & studio-web bundle
FROM node:22-bookworm-slim AS web-builder
WORKDIR /app
COPY package.json package-lock.json ./
COPY packages/engine-client/package.json packages/engine-client/
COPY packages/gfx-runtime/package.json packages/gfx-runtime/
COPY apps/studio-web/package.json apps/studio-web/
RUN npm ci
COPY packages/ packages/
COPY apps/studio-web/ apps/studio-web/
RUN npm run build

# Stage 2: Build Rust server & MCP CLI
FROM rust:1-bookworm AS rust-builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY skills/ skills/
RUN cargo build --release -p solaris-server -p solaris-mcp-server

# Stage 3: Minimal runtime image
FROM debian:bookworm-slim AS runtime
ARG INSTALL_CHROMIUM=false
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    ffmpeg \
    && if [ "$INSTALL_CHROMIUM" = "true" ]; then \
        apt-get install -y --no-install-recommends \
            chromium \
            fonts-liberation \
            fonts-dejavu-core; \
    fi \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --home-dir /var/lib/solaris --shell /usr/sbin/nologin solaris
WORKDIR /var/lib/solaris

COPY --from=rust-builder /app/target/release/solaris-server /usr/local/bin/solaris-server
COPY --from=rust-builder /app/target/release/solaris-mcp /usr/local/bin/solaris-mcp
COPY --from=web-builder /app/apps/studio-web/dist /usr/local/share/solaris/studio-web

RUN mkdir -p /var/lib/solaris/data && chown -R solaris:solaris /var/lib/solaris
USER solaris

ENV SOLARIS_HOST=0.0.0.0 \
    SOLARIS_PORT=41731 \
    SOLARIS_DATA_DIR=/var/lib/solaris/data \
    SOLARIS_STATIC_DIR=/usr/local/share/solaris/studio-web

EXPOSE 41731

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${SOLARIS_PORT:-41731}/api/v1/health" || exit 1

ENTRYPOINT ["/usr/local/bin/solaris-server"]
