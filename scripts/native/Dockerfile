FROM rust:1.98.1-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang lld llvm cmake pkg-config python3-pip ca-certificates git xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN ln -s /usr/bin/clang /usr/local/bin/clang-cl
RUN pip3 install --break-system-packages --no-cache-dir ziglang==0.15.2
RUN cargo install --locked cargo-zigbuild --version 0.23.4 \
    && cargo install --locked cargo-xwin --version 0.23.1
RUN rustup target add \
    aarch64-unknown-linux-musl x86_64-unknown-linux-musl \
    aarch64-pc-windows-msvc x86_64-pc-windows-msvc \
    aarch64-apple-darwin x86_64-apple-darwin
WORKDIR /work
