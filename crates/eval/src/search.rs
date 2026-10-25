//! A task's web search: the search connection glave suggests (#677), turned on, as the
//! person's server runs it, when the task has a `search.md`. Its server is a stand-in for Exa's, never Exa itself: the
//! MCP test server built beside this program (`cargo xtask eval` builds it), answering every
//! query with the file's text, `{web}` in it filled with the task's pages' address. Named and
//! labelled as the suggestion is: `search`, its one tool `web_search_exa`, what it returns
//! untrusted, each call told as what was searched.

use std::collections::BTreeMap;
use std::path::Path;

use glave_mcp::{Limits, McpServer};
use glave_ports::settings::mcp::ServerConfig;
use glave_process::Processes;

use crate::error::EvalError;
use crate::task::Task;

/// The name the person's settings give the search glave suggests.
const NAME: &str = "search";
/// Its one tool.
const TOOL: &str = "web_search_exa";
/// The stand-in's program, beside this one.
const STAND_IN: &str = "glave-mcp-test-server";

/// The search for `task`, its results' `{web}` filled with `web`, started in `root` through
/// `processes`; none when the task has no `search.md`.
pub(crate) async fn connection(
    task: &Task,
    web: Option<&str>,
    root: &Path,
    processes: &Processes,
) -> Result<Option<McpServer>, EvalError> {
    let Some(file) = task.search_results() else {
        return Ok(None);
    };
    let text = tokio::fs::read_to_string(&file)
        .await
        .map_err(EvalError::io("read", &file))?;
    let results = web.map_or_else(|| text.clone(), |base| crate::web::fill(&text, base));
    let here = std::env::current_exe().map_err(EvalError::io("find", STAND_IN))?;
    let command = here.with_file_name(STAND_IN);
    if !command.is_file() {
        return Err(EvalError::Setup(format!(
            "{}: the search stand-in is not built: cargo build -p glave-mcp --features \
             test-server --bin {STAND_IN} (cargo xtask eval builds it)",
            task.name
        )));
    }
    let config = ServerConfig::Stdio {
        command: command.to_string_lossy().into_owned(),
        args: vec!["--results".to_owned(), results],
        env: BTreeMap::new(),
    };
    let none = BTreeMap::new();
    let server = McpServer::start(NAME, &config, &none, root, Limits::default(), processes);
    // Untrusted, as a remote server the person's settings leave unmarked is.
    Ok(Some(server.untrusted(true).searching_with(TOOL)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str) -> Task {
        crate::task::load(&Path::new("../../evals/tasks").join(name)).unwrap()
    }

    #[test]
    fn only_a_task_with_its_results_searches_and_the_stand_in_must_be_built() {
        let runtime = glave_ports::testing::runtime().unwrap();
        let processes = Processes::unrecorded();
        let root = Path::new(".");
        runtime.block_on(async {
            let none = connection(&task("web-lookup"), None, root, &processes).await;
            assert!(none.unwrap().is_none(), "no search.md, no search");
            // This test's own program has no stand-in beside it.
            let searching = task("web-search");
            let missing = connection(&searching, None, root, &processes).await;
            let said = missing
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            assert!(said.contains("cargo xtask eval builds it"), "{said}");
        });
    }
}
