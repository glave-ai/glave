#!/usr/bin/env bash
# The agent stopped the server it started, and the integration tests pass against a fresh one.
set -euo pipefail

answers() {
  python3 -c 'import pathlib, sys, urllib.request
urllib.request.urlopen(pathlib.Path(sys.argv[1]).read_text().strip() + "/health", timeout=2)' \
    .server-address 2>/dev/null
}

if [ -f .server-address ] && answers; then
  echo "the server is still running at $(cat .server-address)"
  exit 1
fi
rm -f .server-address
python3 server.py > server.log 2>&1 &
server=$!
trap 'kill "$server" 2>/dev/null || true' EXIT
for _ in $(seq 100); do
  [ -f .server-address ] && answers && break
  sleep 0.1
done
python3 -m unittest discover tests
