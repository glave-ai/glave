//! Checking a task's grader against its reference solution, `solution/solve.sh`. The untouched
//! task must fail its tests and the solved one must pass them: a grader that always passes
//! would make every model look perfect, and one that never passes would hide every gain.

use std::ffi::OsString;
use std::path::Path;

use crate::prepare::{Prepared, last_lines, prepare, script, tidy};
use crate::task::Task;

/// `Ok` when `task` has no tests, or when its tests fail before its solution and pass after.
pub async fn oracle(base: &[(OsString, OsString)], task: &Task, dir: &Path) -> Result<(), String> {
    let Some(test) = task.test_script() else {
        return Ok(());
    };
    let solution = task.dir.join("solution").join("solve.sh");
    if !solution.is_file() {
        return Err(format!(
            "{} has tests/test.sh but no solution/solve.sh",
            task.name
        ));
    }
    let checked = check(base, task, dir, &test, &solution).await;
    tidy(dir).await;
    checked
}

async fn check(
    base: &[(OsString, OsString)],
    task: &Task,
    dir: &Path,
    test: &Path,
    solution: &Path,
) -> Result<(), String> {
    let name = &task.name;
    let Prepared {
        workspace,
        environment,
    } = prepare(base, task, dir)
        .await
        .map_err(|err| err.to_string())?;
    let run = |path: &Path| {
        let path = path.display().to_string();
        let (workspace, environment) = (&workspace, &environment);
        async move {
            script("bash", &[path.as_str()], workspace, environment)
                .await
                .map_err(|err| err.to_string())
        }
    };
    if run(test).await?.success {
        return Err(format!("{name}: the tests pass before anything is done"));
    }
    let solved = run(solution).await?;
    if !solved.success {
        return Err(format!(
            "{name}: solution/solve.sh failed: {}",
            last_lines(&solved.text)
        ));
    }
    let graded = run(test).await?;
    if !graded.success {
        return Err(format!(
            "{name}: the tests fail on the solution: {}",
            last_lines(&graded.text)
        ));
    }
    Ok(())
}
