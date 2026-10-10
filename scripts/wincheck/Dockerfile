FROM rust:1-trixie
RUN apt-get update && apt-get install -y --no-install-recommends mingw-w64 cmake pkg-config wine wine64 xvfb xauth && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu && rustup component add clippy
RUN xvfb-run -a wineboot -i && wineserver -w
ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine WINEDEBUG=-all
