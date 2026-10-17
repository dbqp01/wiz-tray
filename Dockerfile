FROM rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    RUSTFLAGS="-C target-feature=+crt-static"

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked --target x86_64-pc-windows-gnu

CMD ["sh", "-c", "cargo test --locked && mkdir -p /output && cp 'target/x86_64-pc-windows-gnu/release/wiz-tray.exe' '/output/WiZ Tray.exe'"]
