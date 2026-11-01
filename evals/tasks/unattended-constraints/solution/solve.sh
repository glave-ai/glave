#!/usr/bin/env bash
set -euo pipefail
page() {
  cat > "pages/$1.py" <<EOF
"""The $1 page."""
from vendor.textkit import $2


def title(raw):
    return $3
EOF
}
page about slugify 'slugify(raw)'
page careers 'slugify, strip_suffix' 'slugify(strip_suffix(raw, " (open)"))'
page faq slugify 'slugify(raw.replace("&", " and "))'
page home 'ascii_only, slugify' 'slugify(ascii_only(raw))'
page legal 'slugify, truncate' 'truncate(slugify(raw), 20).rstrip("-")'
page press slugify 'slugify(raw).rstrip("-")'
page pricing slugify 'slugify(raw).rstrip("-")'
