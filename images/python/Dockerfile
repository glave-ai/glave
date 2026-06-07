# Minimal Python eval base — slim (glibc) so manylinux wheels install cleanly
# (alpine/musl forces source builds for C-extension deps). git: the oracle
# applies the held test patch. ripgrep: the agent's grep/glob tools. Built once
# via `glave build-base --lang python`; tasks inject their files at run time.
FROM python:3.12-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends git ripgrep \
    && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir "pytest==8.*"

WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
