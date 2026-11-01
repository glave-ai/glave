//! glave's `web_fetch` as a tool source: an http or https page read as Markdown, with private
//! addresses refused at every hop, size and time limits, and a host policy. What it returns is
//! marked untrusted, for the harness to label and to judge risky actions after it knowing it
//! was read (`specs/approvals.md`). Web search is not built in: it comes from remote tools.

mod cache;
mod fetch;
#[cfg(feature = "pages")]
pub mod pages;
mod resolve;
mod source;
mod text;
mod title;

pub use fetch::{FetchError, Fetcher, Limits, Page};
pub use source::{WEB_FETCH, Web, WebSettings};
