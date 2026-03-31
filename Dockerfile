# syntax=docker/dockerfile:1.7

# ----------------
FROM node:22-bookworm-slim AS admin-assets

WORKDIR /app

ENV PNPM_HOME="/pnpm"
ENV PATH="${PNPM_HOME}:${PATH}"

RUN corepack enable

COPY package.json pnpm-lock.yaml /app/
RUN --mount=type=cache,id=pnpm-store,target=/pnpm/store \
    pnpm install --frozen-lockfile

COPY rspack.config.ts tsconfig.json /app/
COPY assets /app/assets
COPY templates /app/templates
COPY postcss.config.mjs /app/postcss.config.mjs

RUN pnpm run build:admin

# ----------------
FROM debian:12 AS builder

WORKDIR /app

ENV CARGO_HOME="/usr/local/cargo"
ENV RUSTUP_HOME="/usr/local/rustup"
ENV PATH="${CARGO_HOME}/bin:${PATH}"
ENV CC="/usr/bin/clang"
ENV CARGO_TARGET_DIR="/tmp/target"

RUN apt-get update && apt-get install -y \
    curl \
    clang \
    git \
    build-essential \
    pkg-config \
    mold \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal

COPY Cargo.toml Cargo.lock /app/
COPY vendor /app/vendor
COPY src/lib.rs src/main.rs /app/src/
COPY src/bin/session_seed.rs /app/src/bin/session_seed.rs

RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    cargo fetch --locked

COPY src /app/src
COPY templates /app/templates
COPY openapi.json /app/openapi.json

RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=cargo-target,target=/tmp/target \
    cargo build --release --quiet --locked \
    && cp /tmp/target/release/websites /app/websites

# ----------------
FROM gcr.io/distroless/cc-debian12 AS final

WORKDIR /
COPY --from=builder /app/websites /websites
COPY --from=admin-assets /app/admin-ui-assets /admin-ui-assets
COPY site_templates /site_templates

ENV WEBSITES_LISTEN_ADDR="0.0.0.0:9000"
ENV WEBSITES_DB_PATH="/data/websites.sqlite"
ENV WEBSITES_UPLOAD_ROOT="/data/uploads"
ENV WEBSITES_RENDERED_DIR="/data/rendered"
ENV WEBSITES_SITE_TEMPLATES_DIR="/data/site_templates"
ENV WEBSITES_TLS_CERT_PATH="/certs/tls.crt"
ENV WEBSITES_TLS_KEY_PATH="/certs/tls.key"
ENV WEBSITES_ADMIN_ASSETS_DIR="/admin-ui-assets"

EXPOSE 9000
USER nonroot
ENTRYPOINT ["/websites"]
