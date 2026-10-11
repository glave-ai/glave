#!/usr/bin/env bash
set -euo pipefail
sed -i.bak 's/\$((\$1 - \$2))/$(($1 + $2))/' calc.sh
rm -f calc.sh.bak
