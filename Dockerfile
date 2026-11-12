# syntax=docker/dockerfile:1
# Multi-stage build: static musl binary, small Alpine runtime, non-root user.
# Expected image size: about 20 MB (limit of the specification: 50 MB). Check it with
# scripts/check-image.sh.

FROM rust:1.99-alpine AS build
# C toolchain for the bundled SQLite and the TLS crypto backends.
RUN apk add --no-cache build-base cmake perl linux-headers
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# The last command runs the binary: the build fails, not the deployment, if it is broken.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p parfumo-mcp-server \
 && cp target/release/parfumo-mcp-server /parfumo-mcp-server \
 && /parfumo-mcp-server --version

FROM alpine:3
# `source` links the published image to the repository: GitHub then lists it on the
# repository page.
LABEL org.opencontainers.image.title="parfumo-mcp-server" \
      org.opencontainers.image.description="Remote MCP server for Parfumo (Streamable HTTP)" \
      org.opencontainers.image.source="https://github.com/hessbey/parfumo-mcp" \
      org.opencontainers.image.licenses="MIT"
RUN apk add --no-cache ca-certificates \
 && addgroup -S -g 10001 parfumo \
 && adduser -S -D -H -u 10001 -G parfumo parfumo \
 && mkdir /data && chown parfumo:parfumo /data && chmod 700 /data
COPY --from=build /parfumo-mcp-server /usr/local/bin/parfumo-mcp-server
USER 10001:10001
ENV PARFUMO_MCP_BIND=0.0.0.0:8080 \
    PARFUMO_MCP_DB=/data/parfumo-mcp.db
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["parfumo-mcp-server", "healthcheck"]
ENTRYPOINT ["parfumo-mcp-server"]
CMD ["serve"]
