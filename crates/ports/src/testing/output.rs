//! A command's output, kept as it arrives, for a test to read what it printed.

use std::sync::Mutex;

use crate::BoxFuture;
use crate::computer::Output;

/// Everything written to it, in order.
#[derive(Debug, Default)]
pub struct Captured(Mutex<String>);

impl Captured {
    /// What was written so far.
    pub fn text(&self) -> String {
        self.0.lock().map(|text| text.clone()).unwrap_or_default()
    }
}

impl Output for Captured {
    fn write<'a>(&'a self, text: &'a str) -> BoxFuture<'a, ()> {
        if let Ok(mut all) = self.0.lock() {
            all.push_str(text);
        }
        Box::pin(std::future::ready(()))
    }
}
