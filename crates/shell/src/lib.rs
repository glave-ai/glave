//! glave-shell: reading a shell command before it runs. The command is parsed with
//! tree-sitter-bash, as Codex, OpenCode and Claude Code parse it, and every command in it is
//! found, wherever it sits: pipelines, lists, subshells, substitutions and scripts given to
//! `bash -c`. Then it is judged: read-only, risky with a reason, or neither.
//!
//! These checks are tripwires for mistakes and plain attacks, not a security boundary. A
//! determined program can always say the same thing another way; the sandbox is the boundary.

mod credentials;
mod judge;
mod parse;
mod paths;
mod reads;
mod remote;
mod rules;

pub use judge::{Caution, Context, Kind, Verdict, judge};
