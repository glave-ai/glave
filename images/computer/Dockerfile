# glave-computer: the machine a person's computer in the cloud runs (specs/computers.md,
# decision 12). Wolfi, for glibc and packages rebuilt as soon as a fix ships; the tools a
# person's work needs first, and version managers that keep what a person adds in their home,
# which is their disk. glave itself is copied in at each wake, never baked in. Published weekly
# (.github/workflows/computer-image.yml).
#
# The base pinned by digest, moved by hand: the keys and repository it holds check every package
# installed below. Every package, the base's own too, is the latest at each build (`apk
# upgrade`), so fixes land at once.
FROM cgr.dev/chainguard/wolfi-base:latest@sha256:88a20e2d017bd20cdc0cb3910de603e7bea9512a6c351a8dd3c9df212ab03d73

# bubblewrap: glave's sandbox, which commands on a person's computer run in
# (specs/sandbox.md, "On a person's computer").
RUN apk upgrade --no-cache && apk add --no-cache \
      bash bubblewrap build-base ca-certificates-bundle coreutils curl findutils git gnutar grep gzip jq \
      less nodejs-24 npm openssh-client procps python-3.13 ripgrep rustup sed unzip uv zip

# fnm has no Wolfi package: its release, checked against the SHA-256 GitHub records for it.
ARG TARGETARCH
ARG FNM_VERSION=1.39.0
ARG FNM_SHA256_AMD64=7807664f39d39fc518da1c35ba0181e4b3267603c4b1dedeb4b5fc6ae440a224
ARG FNM_SHA256_ARM64=4eaff58b2c5bf30d0934027572dd0b5bbb60d2a1af309230b53662d4b1d45599
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) asset=fnm-linux.zip; sum="$FNM_SHA256_AMD64" ;; \
      arm64) asset=fnm-arm64.zip; sum="$FNM_SHA256_ARM64" ;; \
      *) echo "no fnm for $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/fnm.zip "https://github.com/Schniz/fnm/releases/download/v$FNM_VERSION/$asset"; \
    echo "$sum  /tmp/fnm.zip" | sha256sum -c -; \
    unzip -q /tmp/fnm.zip -d /usr/local/bin; \
    chmod 0755 /usr/local/bin/fnm; \
    rm -f /tmp/fnm.zip

# rustup's proxies, so cargo and rustc name the toolchain a project pins, or the default the
# person sets (`rustup default stable`), installed in their home.
RUN set -eu; \
    for tool in cargo cargo-clippy cargo-fmt clippy-driver rust-analyzer rustc rustdoc rustfmt; do \
      ln -s /usr/bin/rustup "/usr/local/bin/$tool"; \
    done

# nonroot's home is the person's disk, for what reads it from the user database (ssh, git).
RUN sed -i 's#:/home/nonroot:#:/home/glave:#' /etc/passwd

# What a person adds lands in their home: npm's global packages, cargo's and rustup's
# toolchains, and the Node versions fnm installs, its default (`fnm default 22`) on PATH.
# GLAVE_SANDBOX_TOOLCHAINS: Rust's homes, in glave's sandbox, are in its cache, so commands build
# there without asking (specs/sandbox.md, "On a person's computer"). This image holds no toolchain
# to hide: rustup fetches the one chosen, once.
ENV HOME=/home/glave \
    GLAVE_SANDBOX_TOOLCHAINS=1 \
    FNM_DIR=/home/glave/.local/share/fnm \
    NPM_CONFIG_PREFIX=/home/glave/.local \
    PATH=/home/glave/.local/bin:/home/glave/.cargo/bin:/home/glave/.local/share/fnm/aliases/default/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

USER 65532:65532
WORKDIR /home/glave
