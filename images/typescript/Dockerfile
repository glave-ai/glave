# Minimal TypeScript/JS eval base — slim (glibc) so prebuilt native node addons
# install cleanly (alpine/musl often lacks prebuilds). git: the oracle applies
# the held test patch. ripgrep: the agent's grep/glob tools. The project's own
# test runner (jest/vitest) comes from its node_modules at vendor time.
FROM node:22-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends git ripgrep \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
CMD ["tail", "-f", "/dev/null"]
