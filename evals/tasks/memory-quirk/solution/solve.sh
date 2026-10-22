#!/usr/bin/env bash
set -euo pipefail
# The $ are sed's and the script's, not this shell's.
# shellcheck disable=SC2016
sed -i.bak 's/\$((\$1 - \$2))/$(($1 + $2))/' calc.sh
rm -f calc.sh.bak
