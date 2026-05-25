FROM rust:1.86-alpine AS build
WORKDIR /app
RUN apk add --no-cache musl-dev
COPY Cargo.toml Cargo.lock* ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release

FROM alpine:3.21
RUN addgroup -S app && adduser -S app -G app && mkdir -p /data && chown -R app:app /data
USER app
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
