//! A task's web search, on as glave's own `web_search` is where the server has a search key,
//! when the task has a `search.md`: the model under test with a search service beside it that
//! answers every query with the file's results, `{web}` in them filled with the task's pages'
//! address. Never a real search service: a task's search costs nothing and reaches nothing.
//! Each result in the file is a `Title:` line, a `URL:` line and the lines after them, the
//! results apart by a blank line.

use glave_ports::{
    Completion, Delta, Failure, Hit, Made, MediaAsk, ModelProvider, ModelRequest, ProviderError,
    Purpose, SearchAsk, SearchResults,
};

use crate::error::EvalError;
use crate::task::Task;

/// The model under test, and the task's search results, when it searches.
pub(crate) struct WithSearch<P> {
    pub(crate) model: P,
    pub(crate) found: Option<SearchResults>,
}

#[deny(clippy::missing_trait_methods)]
impl<P: ModelProvider> ModelProvider for WithSearch<P> {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<Completion, ProviderError> {
        self.model.complete(request, on_delta).await
    }

    async fn make(&self, request: &ModelRequest, ask: &MediaAsk) -> Result<Made, ProviderError> {
        self.model.make(request, ask).await
    }

    /// The task's results, whatever is asked, as many as it asks at most.
    fn search(
        &self,
        _: &ModelRequest,
        ask: &SearchAsk,
    ) -> impl Future<Output = Result<SearchResults, ProviderError>> + Send {
        let found = match &self.found {
            Some(found) => Ok(SearchResults {
                hits: found
                    .hits
                    .iter()
                    .take(usize::from(ask.count))
                    .cloned()
                    .collect(),
                cost: None,
            }),
            None => Err(ProviderError::Unserved {
                failure: Failure::Rejected,
                reason: "this task searches nothing".to_owned(),
            }),
        };
        std::future::ready(found)
    }

    fn serves(&self, purpose: Purpose) -> bool {
        match purpose {
            Purpose::WebSearch => self.found.is_some(),
            purpose => self.model.serves(purpose),
        }
    }
}

/// The search results of `task`, `{web}` in them filled with `web`; none when it has no
/// `search.md`.
pub(crate) async fn results(
    task: &Task,
    web: Option<&str>,
) -> Result<Option<SearchResults>, EvalError> {
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
