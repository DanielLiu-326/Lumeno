# syntax=docker/dockerfile:1.7

ARG UBUNTU_VERSION=24.04

# ── frontend ─────────────────────────────────────────────────────────────────
FROM ubuntu:${UBUNTU_VERSION} AS frontend
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    gnupg \
    && curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY frontend/package.json frontend/package-lock.json ./frontend/
RUN cd frontend && npm ci

COPY frontend/ ./frontend/
RUN cd frontend && npm run build

# ── Rust builder ─────────────────────────────────────────────────────────────
FROM ubuntu:${UBUNTU_VERSION} AS builder
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    clang \
    cmake \
    curl \
    git \
    libaom-dev \
    libclang-dev \
    libde265-dev \
    libheif-dev \
    libjpeg-dev \
    libnuma-dev \
    libpng-dev \
    libssl-dev \
    libtiff-dev \
    libx265-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain nightly
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY vendor ./vendor
COPY --from=frontend /src/dist/resources ./dist/resources

# dist/models/onnx is intentionally absent in the image context. Local AI
# remains fail-closed; the rest of the application does not need the models.
RUN cargo build --release

# ── runtime ─────────────────────────────────────────────────────────────────
FROM ubuntu:${UBUNTU_VERSION} AS runtime
ARG INSTALL_DARKTABLE=false
ENV DEBIAN_FRONTEND=noninteractive
ENV LISTEN_ADDR=0.0.0.0:3000
ENV PATH=/usr/local/bin:/usr/local/sbin:/usr/bin:/sbin:/bin

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    ffmpeg \
    libimage-exiftool-perl \
    libheif1 \
    libheif-examples \
    libheif-plugin-aomenc \
    libvulkan1 \
    mesa-vulkan-drivers \
    && if [ "${INSTALL_DARKTABLE}" = "true" ]; then \
         apt-get install -y --no-install-recommends darktable; \
       fi \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 10001 lumeno \
    && useradd --uid 10001 --gid lumeno --create-home \
        --shell /usr/sbin/nologin lumeno

WORKDIR /app
COPY --from=builder /src/target/release/lumeno /app/lumeno
COPY --from=builder /src/dist/resources /app/dist/resources
RUN mkdir -p /app/dist/data \
    && chown -R lumeno:lumeno /app \
    && chmod 0755 /app/lumeno

USER lumeno
VOLUME ["/app/dist/data"]
EXPOSE 3000

ENTRYPOINT ["/app/lumeno"]
