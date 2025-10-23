# ---- builder (nightly เพื่อรองรับ edition2024)
FROM rustlang/rust:nightly-bullseye AS builder
WORKDIR /app

# ใส่ manifest ก่อน + warm cache
COPY Cargo.toml ./
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    mkdir -p src && echo "fn main() {}" > src/main.rs && \
    cargo fetch

# ใส่ซอร์สจริงแล้ว build
COPY src ./src
ARG APP_NAME=shopimage
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo build --release --bin ${APP_NAME}

# ---- runtime (ใช้ bullseye ที่ยังมี libssl1.1)
FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates tzdata libssl1.1 poppler-utils \
    python3 python3-pip && \
    pip3 install --no-cache-dir pypdf rembg[cpu] onnxruntime && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /app

ARG APP_NAME=shopimage
COPY --from=builder /app/target/release/${APP_NAME} /app/app
COPY merge_pdf.py /app/merge_pdf.py
COPY remove_bg.py /app/remove_bg.py

ENV PORT=8080
ENV RUST_LOG=info
EXPOSE 8080

CMD ["/app/app"]