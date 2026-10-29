//! Reaping at start: what the ledger holds whose glave is gone is stopped, if it is still the
//! process recorded, and forgotten.

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::ProcessError;
use crate::group;
use crate::ledger::Ledger;
use crate::start;

/// Stops and forgets what glaves that are gone left; how many groups were stopped.
pub(crate) async fn reap(ledger: Arc<Ledger>) -> Result<usize, ProcessError> {
    let plan = tokio::task::spawn_blocking(move || plan(&ledger))
        .await
        .map_err(|error| ProcessError::Start {
            program: "the reaper".into(),
            source: std::io::Error::other(error),
        })??;
    let groups: Vec<_> = plan.iter().filter_map(|(_, id)| group::of(*id)).collect();
    // What outlives SIGKILL's wait is forgotten all the same: this start moves on.
    let _left_behind = group::stop_all(&groups).await;
    for (path, _) in &plan {
        let _ = std::fs::remove_file(path);
    }
    Ok(groups.len())
}

/// The groups to stop, with their entries, from one look at every process the ledger names;
/// entries whose process is gone, or is another now, are forgotten on the way. Blocking.
fn plan(ledger: &Ledger) -> Result<Vec<(PathBuf, i32)>, ProcessError> {
    ledger.clear_abandoned();
    let records = ledger.records()?;
    let starts = start::of(&Ledger::pids(&records));
    let mut stop = Vec::new();
    for (path, record) in records {
        // A glave still running, this one included, owns what it started.
        if record.glave.alive_in(&starts) {
            continue;
        }
        if record.process.alive_in(&starts) {
            stop.push((path, record.group));
        } else {
            // Its leader has ended, or its number is another process's now: nothing proves
            // the group is still the one recorded, so it is never signalled.
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(stop)
}
