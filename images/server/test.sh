#!/bin/sh
# Builds glave's server image from this folder for this machine's processor, with the glave the
# build put at glave/<processor>/glave, and tests it as Docker and Kubernetes run it: on a new
# volume, as its own account with no capability, nothing else writable but /tmp, and glave the
# container's first process, with no init. It holds no program but glave, serves the web app at /
# and answers server/discover with its own version, stops cleanly within seconds when told to
# (SIGTERM), and serves again from its volume when started again. Run by
# .github/workflows/server-image.yml on the image it built, before anything is published.
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
image=glave-server:test
name=glave-server-test
url=http://127.0.0.1:4180/
work="$(mktemp -d)"
cleanup() {
	docker rm -f "$name" >/dev/null 2>&1 || true
	docker volume rm -f "$name" >/dev/null 2>&1 || true
	rm -rf "$work"
}
trap cleanup EXIT

fail() {
	docker logs "$name" >&2 2>&1 || true
	echo "server image: $*" >&2
	exit 1
}

docker buildx build --load --provenance=false --tag "$image" "$here"

user="$(docker image inspect --format '{{.Config.User}}' "$image")"
[ "$user" = 65532:65532 ] || fail "it runs as '$user', not nonroot (65532:65532)"
version="$(docker run --rm "$image" --version)"

# glave's settings, in its folder in the volume and owned by its account: the origin people
# reach it at, without which glave refuses to listen beyond loopback.
mkdir "$work/.glave"
printf '[serve]\norigin = "https://glave.example.test"\n' >"$work/.glave/config.toml"
docker create --name "$name" --read-only --tmpfs /tmp --cap-drop ALL \
	--security-opt no-new-privileges --volume "$name:/home/nonroot" \
	--publish 127.0.0.1:4180:4180 "$image" >/dev/null
tar -C "$work" --owner=65532 --group=65532 --numeric-owner -cf - .glave |
	docker cp --archive - "$name:/home/nonroot"

# A program is a file that can run and holds something: Docker's own empty files (.dockerenv,
# etc/hostname) are made runnable.
docker export "$name" | tar -tvf - >"$work/files"
programs="$(awk '$1 ~ /^-.*x/ && $3 > 0 { sub(/^\.?\//, "", $NF); print $NF }' "$work/files")"
[ "$programs" = usr/local/bin/glave ] || fail "it holds programs besides glave: $programs"
size="$(docker image inspect --format '{{.Size}}' "$image")"
echo "$version in $image: $((size / 1000000)) MB, $(wc -l <"$work/files") files, no program but glave"

# Started, it serves the app at / and answers server/discover within a minute, as its account.
serves() {
	i=0
	until curl -fs --compressed --max-time 5 -o "$work/page" "$url"; do
		i=$((i + 1))
		[ "$i" -lt 60 ] || fail "glave did not serve $url within a minute"
		[ "$(docker inspect --format '{{.State.Running}}' "$name")" = true ] || fail "glave ended as it started"
		sleep 1
	done
	grep -q '<app-root' "$work/page" || fail "$url is not the web app"
	discovered="$(curl -fsS --max-time 5 -H 'content-type: application/json' \
		--data '{"jsonrpc":"2.0","id":1,"method":"server/discover"}' "$url" | jq -r .result.version)"
	[ "glave $discovered" = "$version" ] || fail "server/discover says version $discovered, not that of $version"
	others="$(docker top "$name" -eo pid,uid,args | awk 'NR > 1 && $2 != 65532')"
	[ -z "$others" ] || fail "processes run as another account than nonroot: $others"
}

# Told to stop, it stops on its own within the 10 seconds Docker and Compose give before they
# kill it, and says it ended well.
stops() {
	started="$(date +%s)"
	docker stop --time 30 "$name" >/dev/null
	took=$(($(date +%s) - started))
	code="$(docker inspect --format '{{.State.ExitCode}}' "$name")"
	[ "$code" = 0 ] || fail "glave ended with $code when told to stop, not 0"
	[ "$took" -lt 10 ] || fail "glave took $took seconds to stop"
	echo "stopped cleanly in $took s"
}

docker start "$name" >/dev/null
serves
echo "serves the web app and server/discover at $url, as nonroot"
stops
docker start "$name" >/dev/null
serves
echo "serves again from its volume"
stops
echo "what glave said:"
docker logs "$name" 2>&1
