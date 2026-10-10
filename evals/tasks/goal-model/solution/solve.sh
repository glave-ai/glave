#!/usr/bin/env bash
set -euo pipefail
cat > src/fizzbuzz.rs <<'EOF'
pub fn fizzbuzz(n: u32) -> String {
    match (n % 3, n % 5) {
        (0, 0) => "FizzBuzz".to_owned(),
        (0, _) => "Fizz".to_owned(),
        (_, 0) => "Buzz".to_owned(),
        _ => n.to_string(),
    }
}
EOF
cat > src/digits.rs <<'EOF'
pub fn digit_sum(mut n: u64) -> u64 {
    let mut sum = 0;
    while n > 0 {
        sum += n % 10;
        n /= 10;
    }
    sum
}
EOF
cat > src/gcd.rs <<'EOF'
pub fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}
EOF
cat > src/title.rs <<'EOF'
pub fn title(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}
EOF
cat > src/dedupe.rs <<'EOF'
pub fn dedupe(items: &[i32]) -> Vec<i32> {
    let mut seen = std::collections::HashSet::new();
    items.iter().copied().filter(|item| seen.insert(*item)).collect()
}
EOF
