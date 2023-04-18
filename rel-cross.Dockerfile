FROM --platform=$BUILDPLATFORM rust:1.68.2-bookworm AS builder

RUN for a in armhf arm64 amd64; do dpkg --add-architecture $a; done \
  && apt update && apt install -y \
    binutils-mingw-w64 \
    build-essential \
    clang \
    cmake \
    crossbuild-essential-amd64 \
    crossbuild-essential-arm64 \
    crossbuild-essential-armhf \
    g++-mingw-w64 \
    gcc-mingw-w64 \
    lld \
  && true  
RUN rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl x86_64-pc-windows-gnu i686-pc-windows-gnu armv7-unknown-linux-musleabihf
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
ENV CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABI_LINKER=/usr/bin/clang
ENV CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABI_RUSTFLAGS="-C link-arg=--ld-path=/usr/bin/ld.lld -C link-arg=--target=armv7-unknown-linux-musleabihf"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=/usr/bin/clang
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=--ld-path=/usr/bin/ld.lld -C link-arg=--target=aarch64-unknown-linux-musl"
ENV CARGO_TARGET_X86_64_UNKOWN_LINUX_MUSL_LINKER=/usr/bin/clang
ENV CARGO_TARGET_X86_64_UNKOWN_LINUX_MUSL_RUSTFLAGS="-C link-arg=--ld-path=/usr/bin/ld.lld -C link-arg=--target=x86_64-unknown-linux-musl"
ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=/usr/bin/x86_64-w64-mingw32-gcc
ENV CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER=/usr/bin/i686-w64-mingw32-gcc

WORKDIR /usr/src/totop
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch --locked
COPY . .
RUN cargo add rdkafka --git https://github.com/jcaesar/fork-rust-rdkafka.git
ARG CARGO_BUILD_TARGET
RUN \  
  if echo $CARGO_BUILD_TARGET | grep -q musl; then \
    export CC=/usr/bin/clang; \
    export CXX=/usr/bin/true; \
    export LDFLAGS=--ld-path=/usr/bin/ld.lld; \
  fi; \
  cargo build --release --bin totop --features cmake-build
RUN ln -s $CARGO_BUILD_TARGET target/cross
RUN if echo $CARGO_BUILD_TARGET | grep -q linux; then file target/cross/release/totop | tee /dev/stderr | grep -qE 'static(ally|-pie) linked'; fi

FROM alpine as final
ARG EXE_SUFFIX=
COPY --from=builder /usr/src/totop/target/cross/release/totop$EXE_SUFFIX /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/totop"]
