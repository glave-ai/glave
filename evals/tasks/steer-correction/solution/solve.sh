#!/usr/bin/env bash
set -euo pipefail
sed -i.orig 's/8080/4000/g' config.toml README.md
rm -f config.toml.orig README.md.orig
