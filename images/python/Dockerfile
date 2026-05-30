# Minimal Python eval base. arm64-native on Apple Silicon; tiny footprint.
# Built once via `harness build-base --lang python`; tasks inject their files
# at run time (no per-task build).
FROM python:3.12-alpine

RUN apk add --no-cache ripgrep
RUN pip install --no-cache-dir "pytest==8.*"

WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
