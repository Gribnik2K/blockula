# Blockula node image. Fly builds it remotely (x86_64), so nothing is cross-compiled locally.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY node/ node/
ARG GIT_SHA=unknown
ENV BLOCKULA_GIT_SHA=$GIT_SHA
RUN cargo build --release --manifest-path node/Cargo.toml && cp node/target/release/blockula /blockula

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /data --shell /usr/sbin/nologin blockula
COPY --from=build /blockula /usr/local/bin/blockula
COPY deploy/entrypoint.sh /entrypoint.sh
# The entrypoint starts as root only to fix volume ownership, then drops to the `blockula` user.
ENTRYPOINT ["/entrypoint.sh"]
