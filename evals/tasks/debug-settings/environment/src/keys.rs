//! Setting names and the spellings accepted for each.

/// A setting this crate understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Timeout,
    Retries,
    Verbose,
}

/// Every accepted name, in normal form.
const NAMES: &[(&str, Key)] = &[
    ("timeout", Key::Timeout),
    ("timeout_secs", Key::Timeout),
    ("retries", Key::Retries),
    ("max_retries", Key::Retries),
    ("verbose", Key::Verbose),
];

/// The setting `name` refers to, if any.
pub fn canonical(name: &str) -> Option<Key> {
    let name = normal(name);
    NAMES.iter().find(|(known, _)| *known == name).map(|(_, key)| *key)
}

/// `name` in normal form, so that case and the choice of `-` or `_` do not matter.
fn normal(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}
