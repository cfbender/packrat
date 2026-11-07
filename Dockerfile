# Web app
FROM node:22-alpine AS web
WORKDIR /app/web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build

# Server
FROM rust:1-bookworm AS server
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /app/target/release/packrat /usr/local/bin/packrat
COPY --from=web /app/web/dist ./web/dist
ENV PORT=4100 \
    PACKRAT_DATA_DIR=/data \
    PACKRAT_STATIC_DIR=/app/web/dist
VOLUME ["/data"]
EXPOSE 4100
CMD ["packrat", "serve"]
