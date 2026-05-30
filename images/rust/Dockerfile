# Minimal Rust eval base. The heavy leg: toolchain + linker needed at run time
# (compile-after-edit). musl-dev provides the linker on alpine.
FROM rust:alpine

ENV CARGO_NET_OFFLINE=true
RUN apk add --no-cache musl-dev
WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
