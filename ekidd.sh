#!/usr/bin/env bash

exec docker run --rm -ti \
	-v $PWD:/home/rust/src \
	-v $PWD/ekidd-target:/home/rust/src/target \
	-v $PWD/ekidd-cache:/home/rust/.cargo/registry \
	ekidd/rust-musl-builder bash -c '
		sudo chown rust:rust target ~/.cargo/registry \
		&& cargo build --release --locked --all
	'
