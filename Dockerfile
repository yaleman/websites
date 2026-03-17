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

ADD ./admin-ui-assets /admin-ui-assets
ADD ./site_templates /site_templates

ENV WEBSITES_LISTEN_ADDR="0.0.0.0:9000"
ENV WEBSITES_DB_PATH="/data/websites.sqlite"
ENV WEBSITES_TLS_CERT_PATH="/certs/tls.crt"
ENV WEBSITES_TLS_KEY_PATH="/certs/tls.key"

EXPOSE 9000
USER nonroot
ENTRYPOINT ["/websites"]
