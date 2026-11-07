# Web app
FROM --platform=$BUILDPLATFORM node:22-alpine AS web
WORKDIR /app/web
RUN npm install -g pnpm@12
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build

# Server: always compile natively on the build host and cross-compile for
# arm64 when needed, instead of running cargo under QEMU emulation.
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS server
ARG TARGETARCH
WORKDIR /app
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-gnu > /target-triple ;; \
      arm64) echo aarch64-unknown-linux-gnu > /target-triple \
             && apt-get update \
             && apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu \
             && rm -rf /var/lib/apt/lists/* ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && rustup target add "$(cat /target-triple)"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release --target "$(cat /target-triple)" \
    && cp "target/$(cat /target-triple)/release/packrat" /packrat

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=server /packrat /usr/local/bin/packrat
COPY --from=web /app/web/dist ./web/dist
ENV PORT=4100 \
    PACKRAT_DATA_DIR=/data \
    PACKRAT_STATIC_DIR=/app/web/dist
VOLUME ["/data"]
EXPOSE 4100
CMD ["packrat", "serve"]
