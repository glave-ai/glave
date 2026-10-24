#!/bin/sh
# Pulls the image named by $1, trying again while its registry fails for a moment, as Docker Hub
# does at times with a 502, so a registry's hiccup doesn't fail a run.
set -u
for wait in 5 15 30 60; do
  docker pull -q "$1" >/dev/null && exit 0
  echo "pulling $1 failed; trying again in ${wait}s" >&2
  sleep "$wait"
done
docker pull -q "$1" >/dev/null
