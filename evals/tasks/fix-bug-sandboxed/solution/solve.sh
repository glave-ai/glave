#!/usr/bin/env bash
# For an even count, the two middle values are at mid - 1 and mid.
set -euo pipefail
perl -pi -e 's/sorted\[mid\] \+ sorted\[mid \+ 1\]/sorted[mid - 1] + sorted[mid]/' src/lib.rs
