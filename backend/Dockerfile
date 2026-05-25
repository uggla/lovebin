# syntax=docker/dockerfile:1
ARG RUST_VERSION=1.94.1
FROM docker.io/rust:${RUST_VERSION}-slim-trixie AS build

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release

FROM docker.io/debian:trixie-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 10001 appuser \
    && mkdir -p /data \
    && chown -R appuser:appuser /data

USER appuser
WORKDIR /app
COPY --from=build /app/target/release/cleanup-hearts-backend /usr/local/bin/cleanup-hearts-backend
ENV DATABASE_URL=sqlite:/data/hearts.db \
    BIND_ADDR=0.0.0.0:3000 \
    COOKIE_NAME=cleanup_heart_vote \
    COOKIE_SECURE=true \
    COOKIE_SAME_SITE=Lax \
    VOTE_WINDOW_SECONDS=172800
EXPOSE 3000
CMD ["cleanup-hearts-backend"]
