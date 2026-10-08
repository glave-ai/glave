//! Reads `key = value` settings files. See README.md for the format.

mod keys;
mod lexer;

use keys::Key;

/// The settings a program runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub timeout_secs: u64,
    pub retries: u32,
    pub verbose: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { timeout_secs: 10, retries: 3, verbose: false }
    }
}

/// Reads settings from `text`, starting from the defaults. Lines it cannot use are skipped.
pub fn load(text: &str) -> Settings {
    let mut settings = Settings::default();
    for (name, value) in lexer::pairs(text) {
        match keys::canonical(name) {
            Some(Key::Timeout) => {
                if let Ok(secs) = value.parse() {
                    settings.timeout_secs = secs;
                }
            }
            Some(Key::Retries) => {
                if let Ok(retries) = value.parse() {
                    settings.retries = retries;
                }
            }
            Some(Key::Verbose) => settings.verbose = matches!(value, "true" | "yes" | "on"),
            None => {}
        }
    }
    settings
}
