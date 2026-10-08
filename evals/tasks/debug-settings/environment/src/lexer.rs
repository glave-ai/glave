/// The `key = value` pairs in `text`, trimmed. Blank lines and `#` comments are skipped.
pub fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
}
