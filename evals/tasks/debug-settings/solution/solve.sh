#!/usr/bin/env bash
set -euo pipefail
perl -pi -e "s/replace\('_', \"-\"\)/replace('-', \"_\")/" src/keys.rs
cat > tests/regression.rs <<'EOF'
use settings::load;

#[test]
fn two_word_keys_are_read() {
    assert_eq!(load("max_retries = 5").retries, 5);
}
EOF
