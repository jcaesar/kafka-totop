FROM rust:1.61.0-alpine AS chef
RUN apk add --no-cache musl-dev alpine-sdk cmake
RUN cargo install cargo-chef --locked
WORKDIR /usr/src/totop

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder 
COPY --from=planner /usr/src/totop/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin totop

FROM alpine as final
COPY --from=builder /usr/src/totop/target/release/totop /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/totop"]
