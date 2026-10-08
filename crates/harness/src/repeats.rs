//! The guard against a stuck model: the same tool call too many times in a row is refused.

use glave_ports::ToolCall;

/// The same call this many times in a row is refused, so a stuck model changes course.
const REPEAT_LIMIT: u32 = 3;

/// Tracks identical consecutive tool calls, so a stuck model is told to change course.
#[derive(Default)]
pub(crate) struct Repeats {
    last: Option<String>,
    count: u32,
}

impl Repeats {
    /// Notes `call`; true when it is the same call too many times in a row.
    pub(crate) fn refuse(&mut self, call: &ToolCall) -> bool {
        let signature = format!("{}{:?}", call.tool, call.arguments);
        if self.last.as_deref() == Some(signature.as_str()) {
            self.count = self.count.saturating_add(1);
        } else {
            self.last = Some(signature);
            self.count = 1;
        }
        self.count >= REPEAT_LIMIT
    }
}
