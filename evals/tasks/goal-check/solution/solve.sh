#!/usr/bin/env bash
set -euo pipefail
cat > src/caesar.rs <<'EOF'
/// Shifts every ASCII letter in `text` forward by `n` places in the alphabet, wrapping from z
/// to a and keeping its case. Everything else is unchanged. `shift("Zebra!", 1)` is "Afcsb!".
pub fn shift(text: &str, n: u8) -> String {
    let n = n % 26;
    text.chars()
        .map(|c| match c {
            'a'..='z' => (b'a' + (c as u8 - b'a' + n) % 26) as char,
            'A'..='Z' => (b'A' + (c as u8 - b'A' + n) % 26) as char,
            _ => c,
        })
        .collect()
}
EOF
cat > src/vowels.rs <<'EOF'
/// How many of the letters a, e, i, o and u, in either case, `text` holds.
pub fn count_vowels(text: &str) -> usize {
    text.chars()
        .filter(|c| matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u'))
        .count()
}
EOF
cat > src/palindrome.rs <<'EOF'
/// Whether `text` reads the same backwards, looking only at ASCII letters and digits and
/// ignoring case. "A man, a plan, a canal: Panama" is one; so is the empty string.
pub fn is_palindrome(text: &str) -> bool {
    let kept: Vec<char> = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    kept.iter().eq(kept.iter().rev())
}
EOF
cat > src/acronym.rs <<'EOF'
/// The first letter of every word, uppercased. Words are separated by whitespace or hyphens.
/// `acronym("portable network-graphics")` is "PNG".
pub fn acronym(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c == '-')
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}
EOF
cat > src/runs.rs <<'EOF'
/// Run-length encodes `text`: each run of the same character becomes its length and then the
/// character, with the length left out when it is 1. `encode("aaabcc")` is "3ab2c".
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let mut count = 1;
        while chars.peek() == Some(&c) {
            chars.next();
            count += 1;
        }
        if count > 1 {
            out.push_str(&count.to_string());
        }
        out.push(c);
    }
    out
}
EOF
