# Stage 1: Build
FROM rust:1.82-alpine AS builder
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig cmake make g++
WORKDIR /app
COPY . .
RUN cargo build --release -p api-anything-platform-api

# Stage 2: Runtime (< 20MB)
FROM scratch
COPY --from=builder /app/target/release/api-anything-platform-api /app
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
EXPOSE 8080
ENTRYPOINT ["/app"]
