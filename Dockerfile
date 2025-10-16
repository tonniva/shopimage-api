# ---- builder (nightly เพื่อรองรับ edition2024)
FROM rustlang/rust:nightly-bullseye AS builder
WORKDIR /app

# ใส่ manifest ก่อน + warm cache
COPY Cargo.toml ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    mkdir -p src && echo "fn main() {}" > src/main.rs && \
    cargo fetch

# ใส่ซอร์สจริงแล้ว build
COPY src ./src
ARG APP_NAME=shopimage
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --bin ${APP_NAME}

# ---- runtime (ใช้ bullseye ที่ยังมี libssl1.1)
FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tzdata libssl1.1 && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /app

ARG APP_NAME=shopimage
COPY --from=builder /app/target/release/${APP_NAME} /app/app

ENV PORT=8080
ENV RUST_LOG=info
EXPOSE 8080

CMD ["/app/app"]