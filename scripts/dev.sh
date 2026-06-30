#!/usr/bin/env bash
# Local dev stack: Postgres (Docker) + Rust API (:8080) + Angular (:4200).
#
#   ./scripts/dev.sh              # foreground: run all, Ctrl-C stops API + web
#   ./scripts/dev.sh start        # detached: start in the background, return
#   ./scripts/dev.sh stop         # stop the detached API + web (Postgres stays)
#   ./scripts/dev.sh restart      # stop, then start (detached)
#   ./scripts/dev.sh status       # what's up
#   ./scripts/dev.sh logs [api|web]   # tail logs (both if omitted)
#   ./scripts/dev.sh down         # stop API + web AND remove the Postgres container
#
# Add --fresh to `start`/foreground to recreate Postgres (wipes its data).
# Detached PIDs + logs live in .dev/ (gitignored).
set -eo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PG_NAME=glave-pg-test
PG_URL='postgres://glave:glave@localhost:55432/glave'
NODE_VERSION="$(cat .node-version 2>/dev/null || echo 24.15.0)"
DEV_DIR="$ROOT/.dev"
API_PID="$DEV_DIR/api.pid"; API_LOG="$DEV_DIR/api.log"
WEB_PID="$DEV_DIR/web.pid"; WEB_LOG="$DEV_DIR/web.log"
mkdir -p "$DEV_DIR"

ensure_docker() {
  docker info >/dev/null 2>&1 || { echo "error: Docker isn't running — start Docker Desktop." >&2; exit 1; }
}

ensure_pg() {
  ensure_docker
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
}

# Pull only the keys the server needs from .env (avoids sourcing the whole file,
# which can choke on multi-line values like SSH keys).
load_env() {
  [ -f .env ] || return 0
  local k line v
  for k in OPENROUTER_API_KEY GLAVE_API_KEY GLAVE_API_KEY_ACCOUNT; do
    line="$(grep -E "^${k}=" .env 2>/dev/null | head -1)" || true
    [ -z "$line" ] && continue
    v="${line#*=}"; v="${v%\"}"; v="${v#\"}"; v="${v%\'}"; v="${v#\'}"
    export "$k=$v"
  done
}

is_running() { [ -f "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null; }

# Kill a process and all its descendants (cargo→server, npm→ng→node).
kill_tree() {
  local pid="$1" child
  for child in $(pgrep -P "$pid" 2>/dev/null); do kill_tree "$child"; done
  kill "$pid" 2>/dev/null || true
}

stop_one() {
  local pidfile="$1" name="$2"
  if is_running "$pidfile"; then
    echo "▸ stopping $name (pid $(cat "$pidfile"))…"
    kill_tree "$(cat "$pidfile")"
  fi
  rm -f "$pidfile"
}

cmd_start() {
  ensure_pg "${1:-}"
  load_env
  if is_running "$API_PID"; then
    echo "▸ API already running (pid $(cat "$API_PID"))"
  else
    echo "▸ starting API → http://127.0.0.1:8080   (logs: .dev/api.log)"
    GLAVE_DATABASE_URL="$PG_URL" GLAVE_DEV_OTP=1 \
      nohup cargo run -q -p glave-cli -- serve >"$API_LOG" 2>&1 &
    echo $! > "$API_PID"
  fi
  if is_running "$WEB_PID"; then
    echo "▸ web already running (pid $(cat "$WEB_PID"))"
  else
    echo "▸ starting web → http://localhost:4200   (logs: .dev/web.log)"
    nohup fnm exec --using="$NODE_VERSION" -- npm --prefix apps/web start >"$WEB_LOG" 2>&1 &
    echo $! > "$WEB_PID"
  fi
  echo "▸ up (detached). Open http://localhost:4200 — './scripts/dev.sh status | logs | stop'"
}

cmd_stop() {
  stop_one "$API_PID" API
  stop_one "$WEB_PID" web
  # ng serve / vite can outlive npm; sweep any stragglers for this project.
  pkill -f "ng serve" 2>/dev/null || true
  echo "▸ stopped (Postgres left running — './scripts/dev.sh down' to remove it)"
}

cmd_status() {
  printf 'Postgres : '; docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${PG_NAME}$" && echo 'up (:55432)' || echo 'down'
  printf 'API      : '; { is_running "$API_PID" && curl -fsS http://127.0.0.1:8080/health >/dev/null 2>&1; } && echo 'up (:8080)' || echo 'down'
  printf 'Web      : '; { is_running "$WEB_PID" && curl -fsS http://localhost:4200 >/dev/null 2>&1; } && echo 'up (:4200)' || echo 'down'
}

cmd_logs() {
  case "${1:-all}" in
    api) tail -f "$API_LOG" ;;
    web) tail -f "$WEB_LOG" ;;
    *)   tail -f "$API_LOG" "$WEB_LOG" ;;
  esac
}

# Foreground mode: everything in this terminal; Ctrl-C signals the whole process
# group, so children die with it. The trap is belt-and-suspenders.
cmd_fg() {
  ensure_pg "${1:-}"
  load_env
  local pids=()
  cleanup() { trap - INT TERM EXIT; echo; echo '▸ stopping API + web…'; for p in "${pids[@]}"; do kill_tree "$p"; done; }
  trap cleanup INT TERM EXIT
  echo '▸ starting API → http://127.0.0.1:8080'
  GLAVE_DATABASE_URL="$PG_URL" GLAVE_DEV_OTP=1 cargo run -q -p glave-cli -- serve &
  pids+=($!)
  echo "▸ starting web → http://localhost:4200  (node $NODE_VERSION)"
  fnm exec --using="$NODE_VERSION" -- npm --prefix apps/web start &
  pids+=($!)
  echo '▸ ready — open http://localhost:4200  (Ctrl-C to stop)'
  wait
}

usage() { sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

case "${1:-fg}" in
  start)   shift; cmd_start "$@" ;;
  stop)    cmd_stop ;;
  restart) shift; cmd_stop; cmd_start "$@" ;;
  status)  cmd_status ;;
  logs)    shift; cmd_logs "$@" ;;
  down)    cmd_stop; docker rm -f "$PG_NAME" >/dev/null 2>&1 && echo "▸ Postgres removed" || true ;;
  -h|--help|help) usage ;;
  fg|--fresh|"") cmd_fg "${1:-}" ;;
  *) echo "unknown command: $1" >&2; usage; exit 1 ;;
esac
