#!/bin/sh
# THIRD-PARTY-NOTICES, which every release archive carries (dist-workspace.toml, `include`):
# the licences of the Rust crates glave links for one target (cargo-about, with about.toml and
# about.hbs), then those of the npm packages in its web app, which the app's build writes
# beside it (`npm run build` in web/ comes first). It fails on a crate whose licence is missing
# or not one about.toml accepts, and on an npm package that ships no licence text.
#
#   sh .github/third-party-notices.sh <target>   the notices for that target's archive
#   sh .github/third-party-notices.sh            the same for every target the release builds
#
# The release build runs it for the target each job builds (.github/build-setup.yml); CI runs
# it for every target (ci.yml, web-checks), so a missing licence fails a pull request rather
# than a release.

set -eu

cd "$(dirname "$0")/.."

# cargo-about at a fixed version, built from crates.io with its Cargo.lock: it publishes no
# binary for Intel Macs. Installing the version already there is skipped.
ABOUT=0.9.2
WEB=web/dist/app/3rdpartylicenses.txt

say() { printf 'third-party notices: %s\n' "$*" >&2; }

# The targets the release builds (dist-workspace.toml).
targets() {
	sed -n '/^targets = \[/,/\]/p' dist-workspace.toml | grep -o '"[^"]*"' | tr -d '"'
}

# Fails unless every npm package in the web app's notices ships its licence text: Angular's
# build writes a package with none as its name alone.
npm_texts() {
	awk '
		function done() { if (package != "" && !text) { missing = missing "\n  " package } }
		/^Package: / { done(); package = substr($0, 10); text = 0; named = 1; next }
		/^License: / && named { named = 0; next }
		/^-+$/ && length($0) == 80 { done(); package = ""; next }
		package != "" && NF > 0 { text = 1 }
		END {
			done()
			if (missing != "") { print "these npm packages ship no licence text:" missing > "/dev/stderr"; exit 1 }
		}
	' "$WEB"
}

# Writes THIRD-PARTY-NOTICES for the archive of target $1.
write() {
	cargo about generate --locked --fail --target "$1" --manifest-path crates/cli/Cargo.toml \
		--features web --config about.toml --output-file THIRD-PARTY-NOTICES about.hbs
	cat "$WEB" >>THIRD-PARTY-NOTICES
	say "wrote THIRD-PARTY-NOTICES for $1"
}

[ $# -le 1 ] || {
	say "usage: sh .github/third-party-notices.sh [<target>]"
	exit 2
}
if [ "$(cargo about --version 2>/dev/null || true)" != "cargo-about $ABOUT" ]; then
	cargo install --locked --features cli "cargo-about@$ABOUT"
fi
[ -f "$WEB" ] || {
	say "$WEB is missing: build the web app first (npm run build in web/)"
	exit 1
}
npm_texts
if [ $# -eq 1 ]; then
	[ -n "$1" ] || {
		say "the target is empty"
		exit 2
	}
	write "$1"
else
	all="$(targets)"
	[ -n "$all" ] || {
		say "dist-workspace.toml names no targets"
		exit 1
	}
	for target in $all; do write "$target"; done
fi
