FROM rust:1-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-pc-windows-gnu

# Los flags van por target: con RUSTFLAGS global cargo los aplica a los
# proc-macro del host y la compilación nativa de las pruebas falla.
ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C target-feature=+crt-static"

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY src-tauri ./src-tauri
COPY ui ./ui

# Solo la biblioteca: el ejecutable que se entrega es el de la app Tauri.
RUN cargo build --release --locked --target x86_64-pc-windows-gnu --lib \
    && cd src-tauri \
    && cargo build --release --locked --target x86_64-pc-windows-gnu

CMD ["sh", "-c", "cargo test --locked && mkdir -p /output && cp 'src-tauri/target/x86_64-pc-windows-gnu/release/wiz-tray-app.exe' '/output/WiZ Tray.exe' && cp 'src-tauri/target/x86_64-pc-windows-gnu/release/WebView2Loader.dll' '/output/'"]
