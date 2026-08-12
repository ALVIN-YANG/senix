FROM node:24-bookworm-slim AS admin-builder

WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --ignore-scripts
COPY web/ ./
RUN npm run build

FROM rust:1.88-slim-bookworm AS builder

RUN apt-get update \
    && apt-get install --yes --no-install-recommends build-essential cmake clang pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .
COPY --from=admin-builder /src/crates/senixd/assets/admin.html /src/crates/senixd/assets/admin.html
COPY --from=admin-builder /src/crates/senixd/assets/admin.js /src/crates/senixd/assets/admin.js
RUN cargo build --release --locked -p senixd

FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/ALVIN-YANG/senix" \
      org.opencontainers.image.description="A self-contained Rust gateway with safe traffic control and scoped MCP" \
      org.opencontainers.image.licenses="Apache-2.0"

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 senix \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /var/lib/senix --shell /usr/sbin/nologin senix \
    && mkdir -p /etc/senix /var/lib/senix \
    && chown senix:senix /var/lib/senix

COPY --from=builder /src/target/release/senixd /usr/local/bin/senixd

USER senix
WORKDIR /var/lib/senix
EXPOSE 8080 9080
ENTRYPOINT ["/usr/local/bin/senixd"]
CMD ["--listen", "0.0.0.0:8080", "--admin-listen", "0.0.0.0:9080", "--db", "/var/lib/senix/senix.db", "--config", "/etc/senix/gateway.json"]
