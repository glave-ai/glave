# Minimal Rust eval base. The heavy leg: toolchain + linker needed at run time
# (compile-after-edit). musl-dev provides the linker on alpine.
FROM rust:alpine

ENV CARGO_NET_OFFLINE=true
# grep/glob are native (ripgrep crates in the harness) — no rg binary needed.
RUN apk add --no-cache musl-dev git
WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
