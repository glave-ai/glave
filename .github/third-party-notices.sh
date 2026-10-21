#!/bin/sh
# THIRD-PARTY-NOTICES, which every release archive carries (dist-workspace.toml, `include`):
# the licences of the Rust crates glave links for a target (cargo-about, with about.toml and
# about.hbs), then those of the npm packages in its web app, which the app's build writes
# beside it (`npm run build` in web/ comes first). It fails on a crate whose licence cargo-about
# can't find or about.toml doesn't accept, and on an npm package that ships no licence text.
#
#   sh .github/third-party-notices.sh <target>   writes the notices for that target's archive
#   sh .github/third-party-notices.sh            checks every target the release builds at once,
#                                                writing the notices of all of them together
#
# The release build runs it for the target each job builds (.github/build-setup.yml), and CI
# for every target (ci.yml, web-checks), so a missing licence fails a pull request rather than
# a release.

set -eu

CDPATH='' cd -- "$(dirname -- "$0")/.."

# cargo-about at a fixed version, built from crates.io with its Cargo.lock when it isn't already
# there: it publishes no binary for Intel Macs.
ABOUT=0.9.2
# Where the web app's build writes the licences of the npm packages in it.
WEB=web/dist/app/3rdpartylicenses.txt

say() { printf 'third-party notices: %s\n' "$*" >&2; }

# The targets the release builds: dist-workspace.toml's `targets`, without comments.
targets() {
	sed 's/#.*//' dist-workspace.toml |
		awk '/^targets *= *\[/ { on = 1 } on { print } on && /\]/ { exit }' |
		grep -o '"[^"]*"' | tr -d '"'
}

# Fails unless every npm package in the web app's licences ships its licence text. Angular's
# build writes each package as `Package:`, then its `License:` (JSON, over lines when it is an
# object), a blank line and the text, which is empty when the package has none.
npm_texts() {
	awk '
		function done() { if (package != "" && !text) { missing = missing "\n  " package } }
		/^Package: / { done(); package = substr($0, 10); packages++; text = 0; head = 1; next }
		/^-+$/ && length($0) == 80 { done(); package = ""; next }
		head { if (NF == 0) { head = 0 }; next }
		package != "" && NF > 0 { text = 1 }
		END {
			done()
			if (!packages) { print "no npm package found: has the web build changed how it writes them?" > "/dev/stderr"; exit 1 }
			if (missing != "") { print "these npm packages ship no licence text:" missing > "/dev/stderr"; exit 1 }
		}
	' "$WEB"
}

# Writes THIRD-PARTY-NOTICES for the crates glave links on any of the targets given.
notices() {
	set --
	for target in $wanted; do set -- "$@" --target "$target"; done
	cargo about generate --locked --fail "$@" --manifest-path crates/cli/Cargo.toml \
		--features web --config about.toml --output-file THIRD-PARTY-NOTICES about.hbs
	cat "$WEB" >>THIRD-PARTY-NOTICES
}

case $# in
0) wanted="$(targets | paste -s -d " " -)" ;;
1) wanted="$1" ;;
*)
	say "usage: sh .github/third-party-notices.sh [<target>]"
	exit 2
	;;
esac
[ -n "$wanted" ] || {
	say "no target: name one, or list them in dist-workspace.toml"
	exit 2
}
[ -f "$WEB" ] || {
	say "$WEB is missing: build the web app first (npm run build in web/)"
	exit 1
}
npm_texts
if [ "$(cargo about --version 2>/dev/null || true)" != "cargo-about $ABOUT" ]; then
	say "installing cargo-about $ABOUT"
	cargo install --locked --features cli "cargo-about@$ABOUT"
fi
notices
say "wrote THIRD-PARTY-NOTICES for $wanted"
