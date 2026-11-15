//! A task's web search, on as glave's own `web_search` is where the server has a search key,
//! when the task has a `search.md`: the gateway's search service is a stand-in that answers
//! every query with the file's results, `{web}` in them filled with the task's pages' address.
//! Never a real search service: a task's search costs nothing and reaches nothing. Each result
//! in the file is a `Title:` line, a `URL:` line and the lines after them, the results apart by
//! a blank line.

use std::sync::Arc;

use glave_event::{Currency, Money, Nanos};
use glave_gateway::WebSearch;
use glave_ports::{
    Completion, Delta, Failure, Hit, ModelProvider, ModelRequest, ProviderError, Purpose,
    SearchAsk, SearchResults,
};

use crate::error::EvalError;
use crate::task::Task;

/// A task's search service: its results, whatever is asked.
struct TaskSearch {
    found: SearchResults,
}

impl ModelProvider for TaskSearch {
    /// Never a model's call.
    fn complete(
        &self,
        request: &ModelRequest,
        _: &mut (dyn FnMut(Delta) + Send),
    ) -> impl Future<Output = Result<Completion, ProviderError>> + Send {
        std::future::ready(Err(ProviderError::Unserved {
            failure: Failure::Rejected,
            reason: format!("a task's search serves no {}", request.purpose.id()),
        }))
    }

    /// The task's results, as many as asked at most.
    fn search(
        &self,
        _: &ModelRequest,
        ask: &SearchAsk,
    ) -> impl Future<Output = Result<SearchResults, ProviderError>> + Send {
        let hits = self.found.hits.iter().take(usize::from(ask.count));
        std::future::ready(Ok(SearchResults {
            hits: hits.cloned().collect(),
            cost: None,
        }))
    }

    fn serves(&self, purpose: Purpose) -> bool {
        purpose == Purpose::WebSearch
    }
}

/// The gateway's search service for `task`, its results' `{web}` filled with `web`, free; none
/// when the task has no `search.md`.
pub(crate) async fn service(
    task: &Task,
    web: Option<&str>,
) -> Result<Option<WebSearch>, EvalError> {
    let Some(found) = results(task, web).await? else {
        return Ok(None);
    };
    Ok(Some(WebSearch {
        handle: Arc::new(TaskSearch { found }),
        price: Money {
            nanos: Nanos::ZERO,
            currency: Currency::USD,
        },
    }))
}

/// The search results of `task`, `{web}` in them filled with `web`; none when it has no
/// `search.md`.
async fn results(task: &Task, web: Option<&str>) -> Result<Option<SearchResults>, EvalError> {
    let Some(file) = task.search_results() else {
        return Ok(None);
    };
    let text = tokio::fs::read_to_string(&file)
        .await
        .map_err(EvalError::io("read", &file))?;
    let text = web.map_or_else(|| text.clone(), |base| crate::web::fill(&text, base));
    let hits = read(&text);
    if hits.is_empty() {
        return Err(EvalError::Setup(format!(
            "{}: search.md holds no result with a Title: and a URL: line",
            file.display()
        )));
    }
    Ok(Some(SearchResults { hits, cost: None }))
}

/// The results `text` holds, in order.
fn read(text: &str) -> Vec<Hit> {
    text.split("\n\n")
        .filter_map(|block| {
            let mut lines = block.trim().lines();
            let title = lines.next()?.strip_prefix("Title:")?.trim().to_owned();
            let url = lines.next()?.strip_prefix("URL:")?.trim().to_owned();
            let snippet = lines.map(str::trim).collect::<Vec<_>>().join(" ");
            Some(Hit {
                title,
                url,
                snippet,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn task(name: &str) -> Task {
        crate::task::load(&Path::new("../../evals/tasks").join(name)).unwrap()
    }

    /// Search and `web_fetch` run outside the sandbox; a command with the network could look
    /// anywhere else, this repository included.
    #[test]
    fn the_search_task_s_commands_have_no_network() {
        assert!(!task("web-search").network);
    }

    #[test]
    fn only_a_task_with_its_results_searches_and_each_result_is_read() {
        let runtime = glave_ports::testing::runtime().unwrap();
        runtime.block_on(async {
            let none = results(&task("web-lookup"), None).await.unwrap();
            assert!(none.is_none(), "no search.md, no search");
            let found = results(&task("web-search"), Some("http://127.0.0.1:9"))
                .await
                .unwrap()
                .unwrap();
            let first = &found.hits[0];
            assert_eq!(first.title, "Bramblewick 3.1 release notes");
            assert_eq!(first.url, "http://127.0.0.1:9/bramblewick-releases.html");
            assert!(
                first
                    .snippet
                    .starts_with("Published: 2026-09-30 Highlights:")
            );
            assert_eq!(found.hits.len(), 2);
        });
    }
}
