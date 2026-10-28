//! Every task's grader, checked against its reference solution: the untouched task fails its
//! tests and the solved one passes them. No model is involved, so this runs in CI.

use std::path::Path;

use glave_eval::{load_all, oracle};
use glave_process::Processes;

#[test]
fn every_grader_fails_the_task_and_passes_its_solution() {
    let tasks = load_all(Path::new("../../evals/tasks")).unwrap();
    let base: Vec<_> = std::env::vars_os().collect();
    let root = glave_ports::testing::TempDir::new("eval-oracle").unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let failures: Vec<String> = runtime.block_on(async {
        let mut checks = tokio::task::JoinSet::new();
        for task in tasks {
            let (base, dir) = (base.clone(), root.join(&task.name));
            checks.spawn(async move { oracle(&Processes::unrecorded(), &base, &task, &dir).await });
        }
        let mut failures = Vec::new();
        while let Some(result) = checks.join_next().await {
            if let Err(failure) = result.unwrap() {
                failures.push(failure);
            }
        }
        failures
    });
    let _ = std::fs::remove_dir_all(&root);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
