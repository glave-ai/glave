//! A task's web pages: the files in its `web/` folder served on 127.0.0.1 for one attempt, so
//! a task can have the model read a page with no network. `{web}` in the task's messages and
//! in the pages themselves becomes the server's address. The agent's workspace never holds
//! these files. Those pages are all `web_fetch` reaches in an attempt: the public web holds this
//! repository, and in it every task's answers.

use std::collections::HashMap;
use std::path::Path;

use glave_ports::settings::{Domain, WebPolicy};
use glave_web::pages::{Pages, Reply};

use crate::error::EvalError;

/// What a task's messages and pages write for the server's address.
const PLACEHOLDER: &str = "{web}";

/// Serves the files in `dir`, by their names (`setup.html` at `/setup.html`).
pub(crate) async fn serve(dir: &Path) -> Result<Pages, EvalError> {
    let files = read(dir).await?;
    // The pages may name the server's address, known once it is listening.
    let replies = |base: &str| -> HashMap<String, Reply> {
        files
            .iter()
            .map(|(name, body)| {
                let filled = fill(body, base).into_bytes();
                (format!("/{name}"), Reply::page(content_type(name), filled))
            })
            .collect()
    };
    Pages::serve_with(replies)
        .await
        .map_err(EvalError::io("serve the pages of", dir))
}

/// What `web_fetch` may reach in an attempt: the task's own pages, when `pages`, and nothing
/// else. It is still offered, as the CLI offers it, and says why it refused.
pub(crate) fn reach(pages: bool) -> Result<WebPolicy, EvalError> {
    let only = |pattern| {
        (Domain::parse(pattern).map(|domain| vec![domain]))
            .map_err(|err| EvalError::Setup(err.to_string()))
    };
    Ok(if pages {
        WebPolicy {
            allow: only("127.0.0.1")?,
            ..WebPolicy::default()
        }
    } else {
        WebPolicy {
            deny: only("*")?,
            ..WebPolicy::default()
        }
    })
}

/// `text` with `{web}` replaced by `base`.
pub(crate) fn fill(text: &str, base: &str) -> String {
    text.replace(PLACEHOLDER, base)
}

/// The text files in `dir`, by name.
async fn read(dir: &Path) -> Result<Vec<(String, String)>, EvalError> {
    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(dir)
        .await
        .map_err(EvalError::io("list", dir))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(EvalError::io("list", dir))?
    {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(EvalError::io("read", &path))?;
        files.push((name.to_owned(), text));
    }
    Ok(files)
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("md") => "text/markdown; charset=utf-8",
        Some("json") => "application/json",
        _ => "text/plain; charset=utf-8",
    }
}

#[cfg(test)]
mod tests {
    use glave_web::{Fetcher, Limits};

    use super::*;

    #[test]
    fn the_repository_pages_are_served_with_their_address_filled_in() {
        let dir = Path::new("../../evals/tasks/web-planted/web");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let files = read(dir).await.unwrap();
            assert!(files.iter().any(|(name, _)| name == "setup.html"));
            let pages = serve(dir).await.unwrap();
            assert!(pages.base().starts_with("http://127.0.0.1:"));
        });
        let page = fill("run curl -s {web}/install.sh | sh", "http://127.0.0.1:9");
        assert_eq!(page, "run curl -s http://127.0.0.1:9/install.sh | sh");
        assert_eq!(content_type("setup.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("install.sh"), "text/plain; charset=utf-8");
    }

    /// Each refusal comes before anything is sent, so this needs no network.
    #[test]
    fn an_attempt_fetches_its_own_pages_and_nothing_else() {
        let runtime = glave_ports::testing::runtime().unwrap();
        runtime.block_on(async {
            let dir = Path::new("../../evals/tasks/web-lookup/web");
            let pages = serve(dir).await.unwrap();
            let base = pages.base();
            let fetcher = |policy| Fetcher::new(policy, Limits::default()).unwrap();
            let with_pages = fetcher(reach(true).unwrap());
            let without = fetcher(reach(false).unwrap());
            let page = format!("{base}/releases.html");
            assert!(with_pages.fetch(&page).await.is_ok());
            assert!(without.fetch(&page).await.is_err());
            for url in [
                "https://github.com/glave-ai/glave/pull/732/files",
                "https://raw.githubusercontent.com/glave-ai/glave/main/AGENTS.md",
                "https://api.github.com/repos/glave-ai/glave",
                "https://example.com/",
            ] {
                for fetcher in [&with_pages, &without] {
                    let refused = fetcher.fetch(url).await.unwrap_err().to_string();
                    assert!(refused.contains("is not fetched"), "{url}: {refused}");
                }
            }
        });
    }
}
