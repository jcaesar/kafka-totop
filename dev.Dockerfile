FROM rust:1.62.0 AS chef
RUN apt update && apt install -y build-essential cmake musl musl-dev musl-tools
RUN cargo install cargo-chef --locked
RUN rustup target add $(uname -m)-unknown-linux-musl
RUN mkdir -p ~/.cargo && echo [build]\\ntarget = "$(uname -m)-unknown-linux-musl" >>~/.cargo/config
WORKDIR /usr/src/totop

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
COPY --from=planner /usr/src/totop/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin totop
RUN file target/release/totop; ldd target/release/totop; false

FROM alpine as final
COPY --from=builder /usr/src/totop/target/release/totop /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/totop"]
