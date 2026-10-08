//! Running every task on every model, several at once, under a cost cap.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::run::{self, Attempt, Setup};
use crate::task::Task;

/// What to run.
pub struct Plan {
    pub tasks: Vec<Task>,
    pub models: Vec<String>,
    /// Attempts of each task on each model. Models answer differently each time, so one
    /// attempt can mislead.
    pub repeat: u32,
    /// Attempts running at once.
    pub concurrency: usize,
    /// No new attempt starts once this much has been spent, in micros. Attempts already
    /// running finish, so the total can pass it by a little.
    pub max_cost_micros: i64,
    /// Each attempt keeps its files in `<dir>/<task>/<model>-<number>`.
    pub dir: PathBuf,
}

/// Runs the plan, calling `finished` as each attempt ends, and returns every attempt ordered by
/// task, model and number.
pub async fn run(
    setup: Arc<Setup>,
    plan: Plan,
    mut finished: impl FnMut(&Attempt),
) -> Vec<Attempt> {
    let permits = Arc::new(Semaphore::new(plan.concurrency.max(1)));
    let spent = Arc::new(AtomicI64::new(0));
    let cap = plan.max_cost_micros;
    let mut running = JoinSet::new();
    let mut started = HashMap::new();
    for task in plan.tasks.into_iter().map(Arc::new) {
        for model in &plan.models {
            for number in 1..=plan.repeat {
                let (setup, task, model) = (Arc::clone(&setup), Arc::clone(&task), model.clone());
                let (permits, spent) = (Arc::clone(&permits), Arc::clone(&spent));
                let dir = plan
                    .dir
                    .join(&task.name)
                    .join(format!("{}-{number}", model.replace('/', "_")));
                let label = (task.name.clone(), model.clone(), number);
                let handle = running.spawn(async move {
                    // The semaphore is never closed, so a permit always comes.
                    let _permit = permits.acquire_owned().await;
                    if spent.load(Ordering::SeqCst) >= cap {
                        return Attempt::unrun(
                            &task.name,
                            &model,
                            number,
                            true,
                            "skipped: the cost cap was reached".to_owned(),
                        );
                    }
                    let attempt = run::attempt(&setup, &task, &model, number, &dir)
                        .await
                        .unwrap_or_else(|err| {
                            Attempt::unrun(
                                &task.name,
                                &model,
                                number,
                                false,
                                format!("could not run: {err}"),
                            )
                        });
                    spent.fetch_add(attempt.cost_micros.unwrap_or(0), Ordering::SeqCst);
                    attempt
                });
                started.insert(handle.id(), label);
            }
        }
    }
    let mut attempts = Vec::new();
    while let Some(joined) = running.join_next_with_id().await {
        let attempt = match joined {
            Ok((_, attempt)) => attempt,
            Err(err) => match started.get(&err.id()) {
                Some((task, model, number)) => Attempt::unrun(
                    task,
                    model,
                    *number,
                    false,
                    format!("the attempt stopped: {err}"),
                ),
                None => continue,
            },
        };
        finished(&attempt);
        attempts.push(attempt);
    }
    attempts.sort_by(|a, b| (&a.task, &a.model, a.number).cmp(&(&b.task, &b.model, b.number)));
    attempts
}
