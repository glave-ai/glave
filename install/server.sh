#!/bin/sh
# glave on a Linux server, as a service that starts when the server does (specs/one-server.md,
# "On a server"). One line, and one answer: how the person reaches it.
#
#   curl -fsSL https://raw.githubusercontent.com/glave-ai/glave/main/install/server.sh | sh -s -- --private
#   curl -fsSL https://raw.githubusercontent.com/glave-ai/glave/main/install/server.sh | sh -s -- --public glave.example.com
#
# Without --private, --public or --origin it asks, when someone is at the terminal. It runs
# glave under an account of its own, with its home and data in /var/lib/glave, and ends with
# the link that makes the first passkey. Run it again to update glave: it keeps glave's
# settings and data, and glave never updates itself.

set -eu

REPOSITORY="https://github.com/glave-ai/glave"
PROGRAM=/usr/local/bin/glave
ACCOUNT=glave
STATE=/var/lib/glave
FOLDER="$STATE/.glave"
CONFIG="$FOLDER/config.toml"
UNIT=/etc/systemd/system/glave.service
LISTEN=127.0.0.1:4180
RECOVER="sudo -u $ACCOUNT -H glave server link"

USAGE="Installs glave on this Linux server as a service, or updates it.

  --private          reach glave through Tailscale, with no port open here (recommended)
  --public <domain>  reach glave at your domain, which points at this server: glave gets
                     its HTTPS certificate itself and serves on port 443
  --origin <url>     reach glave through a tunnel or proxy of yours that serves HTTPS at
                     <url> and forwards to $LISTEN
  --version <tag>    install this release of glave rather than the latest
  --binary <file>    install this glave program rather than downloading one
  --acme <url>       with --public, the ACME directory the certificate comes from, when it
                     is not Let's Encrypt's

Run it again to update glave: it keeps glave's settings and data."

REACH=""
DOMAIN=""
ORIGIN=""
VERSION=""
BINARY=""
ACME=""

say() { printf 'glave: %s\n' "$*" >&2; }
fail() {
	say "$*"
	exit 1
}

# The commands that need root: run as they are by root, else through sudo.
if [ "$(id -u)" = 0 ]; then
	SUDO=""
elif command -v sudo >/dev/null 2>&1; then
	SUDO="sudo"
else
	fail "run this as root, or install sudo: it sets glave up as a service"
fi

# Runs a command as glave's account, in its home. The inner shell expands its own $HOME.
# shellcheck disable=SC2016
as_glave() {
	if [ "$(id -u)" = 0 ] && command -v runuser >/dev/null 2>&1; then
		runuser -u "$ACCOUNT" -- env HOME="$STATE" sh -c 'cd "$HOME" && exec "$@"' sh "$@"
	elif command -v sudo >/dev/null 2>&1; then
		sudo -u "$ACCOUNT" -H sh -c 'cd "$HOME" && exec "$@"' sh "$@"
	else
		fail "install sudo, then run this again: it runs glave's commands as glave's account"
	fi
}

parse() {
	while [ $# -gt 0 ]; do
		case "$1" in
		--private) REACH=private ;;
		--public)
			[ $# -ge 2 ] || fail "--public needs your domain, such as glave.example.com"
			REACH=public
			DOMAIN="$2"
			shift
			;;
		--origin)
			[ $# -ge 2 ] || fail "--origin needs the address people reach glave at, such as https://glave.example.com"
			REACH=origin
			ORIGIN="$2"
			shift
			;;
		--version | --binary | --acme)
			[ $# -ge 2 ] || fail "$1 needs a value"
			case "$1" in
			--version) VERSION="$2" ;;
			--binary) BINARY="$2" ;;
			--acme) ACME="$2" ;;
			esac
			shift
			;;
		-h | --help)
			printf '%s\n' "$USAGE"
			exit 0
			;;
		*) fail "$1 is not something this installer takes; --help says what it does" ;;
		esac
		shift
	done
	case "$DOMAIN" in
	*[!A-Za-z0-9.-]* | .* | *. | *..*) fail "$DOMAIN is not a domain glave can get a certificate for, such as glave.example.com" ;;
	esac
	case "$ORIGIN" in
	"" | https://*) ;;
	*) fail "$ORIGIN is not an address people reach glave at: https:// and its name, such as https://glave.example.com" ;;
	esac
	if [ -n "$ACME" ] && [ "$REACH" != public ]; then
		fail "--acme is for --public: where its certificate comes from"
	fi
}

# What this server must have: Linux with systemd, on a processor glave is built for.
check_system() {
	[ "$(uname -s)" = Linux ] || fail "this installer is for Linux servers; on a Mac, install glave as the README says"
	[ -d /run/systemd/system ] || fail "this server does not run systemd, which starts glave and keeps it running"
	command -v curl >/dev/null 2>&1 || fail "install curl, then run this again"
}

# How the person reaches glave, asked when no flag said and someone is at the terminal.
ask() {
	if ! (exec </dev/tty) 2>/dev/null; then
		fail "say how you will reach glave: --private (through Tailscale, recommended) or --public <your domain>"
	fi
	printf '%s' "How will you reach glave from your devices?
  1) Privately, through Tailscale: no port open on this server (recommended)
  2) Publicly, at a domain of yours that points at this server
Choose 1 or 2: " >/dev/tty
	read -r answer </dev/tty
	case "$answer" in
	1) REACH=private ;;
	2)
		printf 'Your domain (such as glave.example.com): ' >/dev/tty
		read -r answer </dev/tty
		parse --public "$answer"
		;;
	*) fail "$answer is neither 1 nor 2: run this again and choose one" ;;
	esac
}

# Installs packages through the server's package manager.
packages() {
	if command -v apt-get >/dev/null 2>&1; then
		$SUDO env DEBIAN_FRONTEND=noninteractive apt-get update -q >/dev/null
		$SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y -q "$@" >/dev/null
	elif command -v dnf >/dev/null 2>&1; then
		$SUDO dnf install -y -q "$@" >/dev/null
	elif command -v zypper >/dev/null 2>&1; then
		$SUDO zypper --non-interactive install "$@" >/dev/null
	elif command -v pacman >/dev/null 2>&1; then
		$SUDO pacman -S --noconfirm --needed "$@" >/dev/null
	else
		fail "install $* with this server's package manager, then run this again"
	fi
}

# glave's program: the one --binary names, else the release for this processor, checked
# against its checksum.
fetch() {
	if [ -n "$BINARY" ]; then
		[ -f "$BINARY" ] || fail "$BINARY is not a file"
		return 0
	fi
	case "$(uname -m)" in
	x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
	aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
	*) fail "glave is built for x86-64 and ARM64 servers, not $(uname -m)" ;;
	esac
	if ! command -v xz >/dev/null 2>&1; then
		if command -v apt-get >/dev/null 2>&1; then packages xz-utils; else packages xz; fi
	fi
	if [ -n "$VERSION" ]; then
		from="$REPOSITORY/releases/download/$VERSION"
	else
		from="$REPOSITORY/releases/latest/download"
	fi
	archive="glave-$target.tar.xz"
	work="$(mktemp -d)"
	trap 'rm -rf "$work"' EXIT
	say "downloading glave from $from"
	for file in "$archive" "$archive.sha256"; do
		curl --proto '=https' --tlsv1.2 -fsSL -o "$work/$file" "$from/$file" ||
			fail "could not download $from/$file: is there a release of glave there?"
	done
	expected="$(cut -d ' ' -f 1 <"$work/$archive.sha256")"
	actual="$(sha256sum "$work/$archive" | cut -d ' ' -f 1)"
	if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
		fail "the download does not match its checksum, so nothing was installed: run this again"
	fi
	tar -xJf "$work/$archive" -C "$work"
	BINARY="$work/glave-$target/glave"
}

# bubblewrap, the sandbox glave runs commands in, and on Ubuntu 23.10 and later the AppArmor
# profile that lets it make user namespaces (specs/sandbox.md, "Linux").
sandbox() {
	command -v bwrap >/dev/null 2>&1 || packages bubblewrap
	restricted=/proc/sys/kernel/apparmor_restrict_unprivileged_userns
	if [ -r "$restricted" ] && [ "$(cat "$restricted")" = 1 ] && [ ! -e /etc/apparmor.d/bwrap ]; then
		printf 'abi <abi/4.0>,\ninclude <tunables/global>\nprofile bwrap %s flags=(unconfined) {\n  userns,\n  include if exists <local/bwrap>\n}\n' \
			"$(command -v bwrap)" | $SUDO tee /etc/apparmor.d/bwrap >/dev/null
		$SUDO apparmor_parser -r /etc/apparmor.d/bwrap
	fi
}

# glave's account: no password and no login shell; its home holds glave's folder.
account() {
	id "$ACCOUNT" >/dev/null 2>&1 && return 0
	shell="$(command -v nologin || echo /bin/false)"
	$SUDO useradd --system --user-group --create-home --home-dir "$STATE" --shell "$shell" "$ACCOUNT"
	$SUDO chmod 0700 "$STATE"
}

# The address Tailscale serves glave at in the person's tailnet, set up first if need be.
tailscale_origin() {
	if ! command -v tailscale >/dev/null 2>&1; then
		say "installing Tailscale, with its own installer (tailscale.com/install.sh)"
		curl -fsSL https://tailscale.com/install.sh | $SUDO sh
	fi
	if ! $SUDO tailscale status >/dev/null 2>&1; then
		say "signing this server in to your tailnet: open the link Tailscale prints"
		$SUDO tailscale up
	fi
	# Where the tailnet has no HTTPS yet, Tailscale prints the link that turns it on, and waits.
	$SUDO tailscale serve --bg --https=443 "http://$LISTEN"
	name="$($SUDO tailscale status --json | sed -n 's/.*"DNSName": *"\([^"]*\)".*/\1/p' | head -n 1)"
	name="${name%.}"
	[ -n "$name" ] || fail "Tailscale gave this server no name: turn on MagicDNS and HTTPS in your tailnet's DNS settings, then run this again"
	ORIGIN="https://$name"
}

# Whether the flags given, if any, ask for the way glave is reached already.
unchanged() {
	kept="$($SUDO sed -n 's/^\(domain\|origin\) = "\(.*\)"$/\1 \2/p' "$CONFIG")"
	case "$REACH" in
	"") true ;;
	public) [ "$kept" = "domain $DOMAIN" ] ;;
	origin) [ "$kept" = "origin $ORIGIN" ] ;;
	private) case "$kept" in "origin https://"*.ts.net) true ;; *) false ;; esac ;;
	esac
}

# With HTTPS of glave's own, port 443 open in the server's firewall, when ufw keeps one.
firewall() {
	[ "$REACH" = public ] || return 0
	command -v ufw >/dev/null 2>&1 || return 0
	if $SUDO ufw status 2>/dev/null | grep -q '^Status: active'; then
		$SUDO ufw allow 443/tcp >/dev/null
		say "opened port 443 in this server's firewall (ufw)"
	fi
}

# glave's settings: how people reach it, readable only by its account.
configure() {
	case "$REACH" in
	public)
		settings="[serve]
domain = \"$DOMAIN\""
		[ -z "$ACME" ] || settings="$settings
acme = \"$ACME\""
		;;
	*) settings="[serve]
origin = \"$ORIGIN\"" ;;
	esac
	# The inner shell expands its own arguments.
	# shellcheck disable=SC2016
	as_glave sh -c 'umask 077 && mkdir -p "$1" && cat >"$2"' sh "$FOLDER" "$CONFIG" <<EOF
# How people reach glave from elsewhere, as the installer set it up.
$settings
EOF
}

# The service: glave under its account, on loopback, and on port 443 when it serves HTTPS
# itself, the one capability that needs. Its graceful stop gives running turns ten seconds.
service() {
	capability=""
	if $SUDO grep -q '^domain' "$CONFIG"; then
		capability="AmbientCapabilities=CAP_NET_BIND_SERVICE
CapabilityBoundingSet=CAP_NET_BIND_SERVICE"
	fi
	$SUDO tee "$UNIT" >/dev/null <<EOF
[Unit]
Description=glave
Documentation=$REPOSITORY
After=network-online.target
Wants=network-online.target

[Service]
User=$ACCOUNT
Group=$ACCOUNT
WorkingDirectory=$STATE
ExecStart=$PROGRAM serve --http $LISTEN
KillMode=mixed
TimeoutStopSec=30
Restart=on-failure
RestartSec=5
NoNewPrivileges=yes
$capability

[Install]
WantedBy=multi-user.target
EOF
	$SUDO systemctl daemon-reload
	$SUDO systemctl enable --quiet glave
	$SUDO systemctl restart glave
}

# Waits until glave answers on its socket, and with HTTPS of its own until it has its
# certificate; says why when it does not.
started() {
	for _ in $(seq 1 60); do
		if $SUDO test -S "$FOLDER/run/server.sock"; then
			break
		fi
		if $SUDO systemctl is-failed --quiet glave; then
			$SUDO journalctl -u glave -n 20 --no-pager -o cat >&2 || true
			fail "glave did not start; what it said is above"
		fi
		sleep 1
	done
	$SUDO test -S "$FOLDER/run/server.sock" || fail "glave did not start in a minute: journalctl -u glave says why"
	domain="$($SUDO sed -n 's/^domain = "\(.*\)"$/\1/p' "$CONFIG")"
	[ -n "$domain" ] || return 0
	for _ in $(seq 1 90); do
		# Any answer over HTTPS means glave has its certificate: until then it answers only
		# the authority's check.
		if curl -ks --resolve "$domain:443:127.0.0.1" -o /dev/null "https://$domain/"; then
			return 0
		fi
		sleep 1
	done
	say "glave has no certificate for $domain yet: check that $domain points at this server and that port 443 is open to it (journalctl -u glave says more)"
}

main() {
	parse "$@"
	check_system
	first=yes
	if $SUDO test -f "$CONFIG"; then
		first=no
		unchanged || fail "glave is set up here already, reached as $CONFIG says; to change that, edit [serve] there, then run: sudo systemctl restart glave"
	elif [ -z "$REACH" ]; then
		ask
	fi
	fetch
	sandbox
	account
	if [ "$REACH" = private ]; then tailscale_origin; fi
	$SUDO install -m 0755 "$BINARY" "$PROGRAM.new"
	$SUDO mv -f "$PROGRAM.new" "$PROGRAM"
	if [ "$first" = yes ]; then
		configure
		firewall
	fi
	service
	started
	version="$("$PROGRAM" --version)"
	if [ "$first" = no ]; then
		say "$version runs on this server again, with its settings and data"
		exit 0
	fi
	say "$version runs on this server, and starts when it does"
	printf '\n'
	as_glave "$PROGRAM" server link
	printf '\nFor another device, or if you lose every one, run this on the server:\n  %s\nTo update glave, run this installer again.\n' "$RECOVER"
}

main "$@"
