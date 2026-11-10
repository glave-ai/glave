# glave: glave as a server (`glave serve`), for a cluster or any container runtime
# (specs/deployments.md). glave of a release, static (musl) with the web app inside, for this
# image's processor, put at glave/<amd64|arm64>/glave by the build: the release's own archives,
# checked against their provenance (.github/workflows/server-image.yml), or on a pull request
# glave built as the release builds it. Published after each release, and by hand.
#
# Nothing but glave: no shell, package manager, git or sandbox. People's work runs on their
# computers (`[computers]`, specs/computers.md), never in the server's container; a server that
# is also a person's computer runs where their tools are, as the installer sets one up
# (install/server.sh), or on glave's computer image (images/computer). Chainguard's static base,
# from Wolfi as that image is: the CA certificates glave checks TLS against, time zones and the
# nonroot account, under a megabyte, pinned by digest and moved by hand.
ARG BASE=cgr.dev/chainguard/static:latest@sha256:fc42af33e643f20d08baf03cddcc1c02ba57052dfb02edb0d79f3fc47bcea8d9

FROM $BASE

# Owned by root: the account glave runs as cannot change it.
ARG TARGETARCH
COPY glave/$TARGETARCH/glave /usr/local/bin/glave

# nonroot (65532), never root, as people's computers run. Its home is the image's one volume,
# with glave's folder in it, named so glave keeps its root key in a key file there rather than
# looking for a keychain the container has none of. glave writes nowhere else, so the root
# filesystem can be read-only, with a /tmp of its own. The deployment puts glave's settings in
# the volume first (`.glave/config.toml`): at least the origin people reach it at (`[serve]
# origin`), through the proxy or tunnel that serves HTTPS in front of it, without which glave
# refuses to listen beyond loopback (specs/one-server.md, "On a server").
USER 65532:65532
ENV HOME=/home/nonroot \
    GLAVE_HOME=/home/nonroot/.glave
VOLUME /home/nonroot
WORKDIR /home/nonroot

EXPOSE 4180
ENTRYPOINT ["/usr/local/bin/glave"]
CMD ["serve", "--http", "0.0.0.0:4180"]
