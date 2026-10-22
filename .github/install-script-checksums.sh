#!/bin/sh
# dist's install script (glave-installer.sh, the `curl … | sh` line) checks each archive's
# sha256 with sha256sum, and where that command is missing, as on macOS before 15, it installs
# the archive unchecked. This makes it check with shasum there, which every macOS has, and
# refuse to install when it has neither. dist has no setting for it (0.33.0 and 0.33.1, and
# its main branch, all skip), so the release runs this on the script dist writes, before it is
# uploaded (`cargo xtask release-workflow` puts the step in release.yml), and so does the test
# of the installers (release-installers.yml); CI runs it on what dist writes today (ci.yml). It
# fails unless the script holds, once, exactly the lines it replaces.
#
#   sh .github/install-script-checksums.sh <glave-installer.sh>

set -eu

if [ $# -ne 1 ] || [ ! -f "$1" ]; then
	echo "usage: sh .github/install-script-checksums.sh <glave-installer.sh>" >&2
	exit 2
fi

# The lines dist 0.33 writes, and the lines that take their place: text, printed by functions so
# that no shell reads them as commands.
old() {
	cat <<'EOF'
        sha256)
            if ! check_cmd sha256sum; then
                say "skipping sha256 checksum verification (it requires the 'sha256sum' command)"
                return 0
            fi
            _calculated_checksum="$(sha256sum -b "$_file" | awk '{printf $1}')"
            ;;
EOF
}
new() {
	cat <<'EOF'
        sha256)
            if check_cmd sha256sum; then
                _calculated_checksum="$(sha256sum -b "$_file" | awk '{printf $1}')"
            elif check_cmd shasum; then
                _calculated_checksum="$(shasum -a 256 -b "$_file" | awk '{printf $1}')"
            else
                err "cannot check the download against its checksum: install sha256sum or shasum, then run this again"
            fi
            ;;
EOF
}

patched="$(mktemp)"
trap 'rm -f "$patched"' EXIT
OLD="$(old)" NEW="$(new)" awk '
	{ all = all $0 "\n" }
	END {
		old = ENVIRON["OLD"] "\n"
		at = index(all, old)
		if (at == 0 || index(substr(all, at + length(old)), old) != 0) {
			print "the install script does not hold, once, the sha256 check this replaces: has dist changed it?" > "/dev/stderr"
			exit 1
		}
		printf "%s%s\n%s", substr(all, 1, at - 1), ENVIRON["NEW"], substr(all, at + length(old))
	}
' "$1" >"$patched"
cat "$patched" >"$1"
echo "$1 checks every archive it downloads, with sha256sum or shasum" >&2
