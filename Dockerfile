# ----------------
FROM node:22-bookworm-slim AS admin-assets

WORKDIR /app

ENV PNPM_HOME="/pnpm"
ENV PATH="${PNPM_HOME}:${PATH}"

RUN corepack enable

COPY package.json pnpm-lock.yaml rspack.config.ts tsconfig.json /app/
COPY assets /app/assets
COPY templates /app/templates
COPY postcss.config.mjs /app/postcss.config.mjs

RUN pnpm install --frozen-lockfile
RUN pnpm run build:admin

# ----------------
FROM debian:12 AS builder

ARG GITHUB_SHA="$(git rev-parse HEAD)"

# fixing the issue with getting OOMKilled in BuildKit
# ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN mkdir /app
COPY . /app/

WORKDIR /app
# install the dependencies
RUN apt-get update && apt-get install -y \
    curl \
    clang \
    git \
    build-essential \
    pkg-config \
    mold
# install rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
RUN mv /root/.cargo/bin/* /usr/local/bin/
# do the build bits
ENV CC="/usr/bin/clang"
RUN cargo build --release --quiet --locked

# ----------------
FROM gcr.io/distroless/cc-debian12 AS final

WORKDIR /
COPY --from=builder /app/target/release/websites /websites
COPY --from=admin-assets /app/admin-ui-assets /admin-ui-assets
COPY site_templates /site_templates

ENV WEBSITES_LISTEN_ADDR="0.0.0.0:9000"
ENV WEBSITES_DB_PATH="/data/websites.sqlite"
ENV WEBSITES_RENDERED_DIR="/data/rendered"
ENV WEBSITES_TLS_CERT_PATH="/certs/tls.crt"
ENV WEBSITES_TLS_KEY_PATH="/certs/tls.key"
ENV WEBSITES_ADMIN_ASSETS_DIR="/admin-ui-assets"
ENV WEBSITES_SITE_TEMPLATES_DIR="/site_templates"

EXPOSE 9000
USER nonroot
ENTRYPOINT ["/websites"]
