#!/usr/bin/env bash
# One-command local dev: Postgres (Docker) + Rust API (:8080) + Angular (:4200).
#
#   ./scripts/dev.sh            # start everything; Ctrl-C stops the API + web
#   ./scripts/dev.sh --fresh    # recreate the Postgres container first (wipe data)
#
# The Postgres container (glave-pg-test) is left running between sessions so data
# persists; drop it with `docker rm -f glave-pg-test`.
set -eo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PG_NAME=glave-pg-test
PG_URL='postgres://glave:glave@localhost:55432/glave'
NODE_VERSION="$(cat .node-version 2>/dev/null || echo 24.15.0)"

if ! docker info >/dev/null 2>&1; then
  echo "error: Docker isn't running — start Docker Desktop and retry." >&2
  exit 1
fi

# --- Postgres -------------------------------------------------------------
if [ "${1:-}" = "--fresh" ]; then docker rm -f "$PG_NAME" >/dev/null 2>&1 || true; fi
if ! docker ps --format '{{.Names}}' | grep -q "^${PG_NAME}$"; then
  echo "▸ starting Postgres ($PG_NAME)…"
  docker run -d --name "$PG_NAME" \
    -e POSTGRES_USER=glave -e POSTGRES_PASSWORD=glave -e POSTGRES_DB=glave \
    -p 55432:5432 postgres:16 >/dev/null
fi
printf '▸ waiting for Postgres'
until docker exec "$PG_NAME" pg_isready -U glave >/dev/null 2>&1; do printf '.'; sleep 0.5; done
echo ' ready'

# --- secrets from .env (only the keys the server needs; avoids sourcing the
#     whole file, which can choke on multi-line values like SSH keys) ---------
load_env_key() {
  local k="$1" line v
  line="$(grep -E "^${k}=" .env 2>/dev/null | head -1)" || return 0
  [ -z "$line" ] && return 0
  v="${line#*=}"; v="${v%\"}"; v="${v#\"}"; v="${v%\'}"; v="${v#\'}"
  export "$k=$v"
}
if [ -f .env ]; then
  for k in OPENROUTER_API_KEY GLAVE_API_KEY GLAVE_API_KEY_ACCOUNT; do load_env_key "$k"; done
fi

# --- cleanup: Ctrl-C in the terminal already signals the whole process group;
#     this is belt-and-suspenders for the explicit-exit case ------------------
PIDS=()
cleanup() {
  trap - INT TERM EXIT
  echo; echo '▸ stopping API + web…'
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; pkill -P "$p" 2>/dev/null || true; done
}
trap cleanup INT TERM EXIT

# --- Rust API on :8080 (the port the web proxy targets) -------------------
echo '▸ starting API   → http://127.0.0.1:8080'
GLAVE_DATABASE_URL="$PG_URL" GLAVE_DEV_OTP=1 \
  cargo run -q -p glave-cli -- serve &
PIDS+=($!)

# --- Angular dev server on :4200 (proxy.conf.json forwards /auth + /v1) ----
echo "▸ starting web   → http://localhost:4200  (node $NODE_VERSION)"
fnm exec --using="$NODE_VERSION" -- npm --prefix apps/web start &
PIDS+=($!)

echo '▸ ready — open http://localhost:4200  (Ctrl-C to stop)'
wait
