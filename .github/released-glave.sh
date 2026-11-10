#!/bin/sh
# glave of the latest release for each processor, for the images that carry it
# (.github/workflows/computer-image.yml, server-image.yml): dist's static build of each, checked
# against the provenance the release made for its archives, signed by the Release workflow alone,
# put at $1/<amd64|arm64>/glave. Its version goes in the step's outputs (`version`). Before the
# first release there is none, and nothing goes in them. After a release (`workflow_run`), the
# image $IMAGE already published with the latest version means the release was a dry run, which
# published nothing: `skip`. Each archive's download tries again while GitHub fails for a moment,
# as .github/curl/.curlrc says.
set -eu

folder="$1"
tag="$(gh release list --repo "$GITHUB_REPOSITORY" --exclude-drafts --exclude-pre-releases --limit 1 --json tagName --jq '.[0].tagName // empty')"
if [ -z "$tag" ]; then
	echo "no release of glave yet"
	exit 0
fi
version="${tag#v}"
if [ "$GITHUB_EVENT_NAME" = workflow_run ] && docker buildx imagetools inspect "$IMAGE:$version" >/dev/null 2>&1; then
	echo "$IMAGE:$version is published already: no new release"
	echo "skip=true" >>"$GITHUB_OUTPUT"
	exit 0
fi
for pair in amd64:x86_64 arm64:aarch64; do
	arch="${pair%%:*}"
	archive="glave-${pair#*:}-unknown-linux-musl.tar.xz"
	curl -fsSL --retry 8 --retry-all-errors --retry-max-time 300 --connect-timeout 20 \
		-o "$RUNNER_TEMP/$archive" "$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/releases/download/$tag/$archive"
	gh attestation verify "$RUNNER_TEMP/$archive" --repo "$GITHUB_REPOSITORY" \
		--signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml"
	mkdir -p "$RUNNER_TEMP/$arch" "$folder/$arch"
	tar -xJf "$RUNNER_TEMP/$archive" -C "$RUNNER_TEMP/$arch"
	install -m 0755 "$RUNNER_TEMP/$arch/${archive%.tar.xz}/glave" "$folder/$arch/glave"
done
echo "version=$version" >>"$GITHUB_OUTPUT"
