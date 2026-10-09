//! The tool source port: anything that offers tools to the harness. The built-in tools are one
//! source, each MCP server another. Every call still goes through the harness, which asks the
//! person and records it; a source only says what its tools do and runs them.

use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::task::Poll;

use glave_event::{Arguments, Part};
use thiserror::Error;

use crate::model::ToolSpec;

/// A boxed future, so sources of different kinds can sit in one list.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a tool does, which decides when the person is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    /// Only reads: it never asks, except for files that usually hold secrets.
    Reads,
    /// Changes files in the workspace.
    Changes,
    /// Runs commands or reaches beyond the workspace. Tools that do not say what they do
    /// count as this.
    Runs,
}

/// A tool on offer: what the model is told, and what the harness needs to know about it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub spec: ToolSpec,
    pub effect: Effect,
    /// Whether calls may run beside each other.
    pub parallel: bool,
}

/// What a call produced, as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub is_error: bool,
    pub parts: Vec<Part>,
}

impl ToolOutput {
    pub fn text(is_error: bool, text: impl Into<String>) -> Self {
        Self {
            is_error,
            parts: vec![Part::Text { text: text.into() }],
        }
    }
}

/// Why a source could not list its tools.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{source_name}: {message}")]
pub struct ToolError {
    pub source_name: String,
    pub message: String,
}

/// A signal that the turn running a call was cancelled.
pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
    /// Resolves once cancelled.
    fn cancelled(&self) -> BoxFuture<'_, ()>;
}

/// Runs `work` until it finishes, or until `cancel` fires: `None` then, and `work` is dropped.
/// Cancellation wins a tie.
pub async fn unless_cancelled<T>(
    cancel: &dyn Cancellation,
    work: impl Future<Output = T>,
) -> Option<T> {
    let mut work = pin!(work);
    let mut cancelled = cancel.cancelled();
    poll_fn(|cx| {
        if cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        work.as_mut().poll(cx).map(Some)
    })
    .await
}

/// What one call does, beyond its tool's declared effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assessment {
    /// As the tool declares.
    AsDeclared,
    /// This call only reads, whatever the tool may do.
    Reads,
    /// This call needs the person whatever the mode, for `reason`. `secret` when it would show
    /// the model a file that usually holds secrets.
    Caution { reason: String, secret: bool },
}

/// Anything that offers tools.
pub trait ToolSource: Send + Sync {
    /// The tools on offer now.
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>>;

    /// How to use these tools, in their maker's words, for the model; asked after `tools`.
    fn instructions(&self) -> Option<String> {
        None
    }

    /// What this one call does, when the source can tell more than its tool's effect: a
    /// command that only reads, or one that needs the person whatever the mode, such as a read
    /// of a file that usually holds secrets. The check runs where the files are.
    fn assess<'a>(&'a self, name: &'a str, arguments: &'a Arguments) -> BoxFuture<'a, Assessment> {
        let _ = (name, arguments);
        Box::pin(std::future::ready(Assessment::AsDeclared))
    }

    /// The call in a few words, for the person: `bash: cargo test`.
    fn describe(&self, name: &str, arguments: &Arguments) -> String {
        describe_call(name, arguments)
    }

    /// Runs one call. Failures are outputs the model can read, never errors.
    fn call<'a>(
        &'a self,
        name: &'a str,
        arguments: &'a Arguments,
        cancel: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput>;
}

/// How much of a call's arguments its description shows.
const DESCRIBED: usize = 120;

/// A call in a few words: the tool's name, then its arguments cut short.
pub fn describe_call(name: &str, arguments: &Arguments) -> String {
    let text = match arguments {
        Arguments::Json(map) => serde_json::to_string(map).unwrap_or_default(),
        Arguments::Malformed(raw) => raw.clone(),
    };
    let short: String = text.chars().take(DESCRIBED).collect();
    let more = if text.chars().count() > DESCRIBED {
        "…"
    } else {
        ""
    };
    format!("{name} {short}{more}")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Never;
    impl Cancellation for Never {
        fn is_cancelled(&self) -> bool {
            false
        }
        fn cancelled(&self) -> BoxFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }

    struct Already;
    impl Cancellation for Already {
        fn is_cancelled(&self) -> bool {
            true
        }
        fn cancelled(&self) -> BoxFuture<'_, ()> {
            Box::pin(std::future::ready(()))
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn work_runs_unless_cancelled() {
        assert_eq!(block_on(unless_cancelled(&Never, async { 3 })), Some(3));
        assert_eq!(block_on(unless_cancelled(&Already, async { 3 })), None);
    }

    #[test]
    fn calls_are_described_briefly() {
        struct Any;
        impl ToolSource for Any {
            fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
                Box::pin(std::future::ready(Ok(Vec::new())))
            }
            fn call<'a>(
                &'a self,
                _: &'a str,
                _: &'a Arguments,
                _: &'a dyn Cancellation,
            ) -> BoxFuture<'a, ToolOutput> {
                Box::pin(std::future::ready(ToolOutput::text(false, "")))
            }
        }
        let mut map = serde_json::Map::new();
        map.insert("title".into(), "x".repeat(300).into());
        let described = Any.describe("mcp__github__create_issue", &Arguments::Json(map));
        assert!(described.starts_with("mcp__github__create_issue {\"title\""));
        assert!(described.ends_with('…'));
        assert!(described.chars().count() < 160);
    }
}
