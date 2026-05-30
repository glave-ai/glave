# Minimal Go eval base. arm64-native on Apple Silicon. The toolchain is needed
# at run time (compile-after-edit); stdlib is prebuilt in the base.
FROM golang:alpine

RUN apk add --no-cache ripgrep
ENV GOPROXY=off
WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
