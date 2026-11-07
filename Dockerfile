# Web app
FROM --platform=$BUILDPLATFORM node:22-alpine AS web
WORKDIR /app/web
RUN npm install -g pnpm@12
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build

# Server: a fully static musl binary, compiled natively on the build host
# with zig as the C compiler and linker for both architectures, instead of
# running cargo under QEMU emulation. Static linking lets the runtime image
# carry no OS packages at all (no libc, zlib, util-linux, ...), which is
# what keeps image vulnerability scans clean.
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS server
ARG TARGETARCH
ARG ZIG_VERSION=0.17.1
ARG CARGO_ZIGBUILD_VERSION=0.24.1
RUN ARCH="$(uname -m)" \
    && curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-${ARCH}-linux-${ZIG_VERSION}.tar.xz" \
       | tar -xJ -C /usr/local \
    && ln -s "/usr/local/zig-${ARCH}-linux-${ZIG_VERSION}/zig" /usr/local/bin/zig \
    && cargo install --locked "cargo-zigbuild@${CARGO_ZIGBUILD_VERSION}"
WORKDIR /app
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl > /target-triple ;; \
      arm64) echo aarch64-unknown-linux-musl > /target-triple ;; \
      *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && rustup target add "$(cat /target-triple)"
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo zigbuild --release --target "$(cat /target-triple)" \
    && cp "target/$(cat /target-triple)/release/packrat" /packrat \
    && mkdir /data

# Runtime: distroless static ships only CA certificates, tzdata, and a
# non-root user; there is no shell or package manager in the image.
FROM gcr.io/distroless/static-debian12:nonroot
WORKDIR /app
COPY --from=server /packrat /usr/local/bin/packrat
COPY --from=server --chown=nonroot:nonroot /data /data
COPY --from=web /app/web/dist ./web/dist
ENV PORT=4100 \
    PACKRAT_DATA_DIR=/data \
    PACKRAT_STATIC_DIR=/app/web/dist
VOLUME ["/data"]
EXPOSE 4100
ENTRYPOINT ["packrat"]
CMD ["serve"]
