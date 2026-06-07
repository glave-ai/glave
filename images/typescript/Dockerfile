# Minimal TypeScript/JS eval base — slim (glibc) so prebuilt native node addons
# install cleanly (alpine/musl often lacks prebuilds). git: the oracle applies
# the held test patch. grep/glob are native (ripgrep crates in the harness), so
# no rg binary. The project's test runner (jest/vitest) comes from node_modules.
FROM node:22-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
