//! A person's computer grown for what a command ran short of, one growth at a time: in place
//! where the cluster can, else at its next wake.

use std::sync::Arc;

use super::super::sizing::Size;
use super::{Person, State, holds, slept};
use crate::Short;

/// Grows `person`'s computer for what a command ran `short` of, within `ceiling`, one growth
/// at a time: a computer held at its CPU limit in place where the cluster can, else at its next
/// wake; one killed for memory, its glave with it, at its next wake, which the next call makes.
/// What to tell whoever ran the command; nothing when it did not run short.
pub(super) async fn grow(person: &Arc<Person>, short: Short, ceiling: Size) -> Option<String> {
    let _growing = person.growing.lock().await;
    let now = person.pod.sizing.now();
    match short {
        Short::Cpu => {
            let bigger = now.grown(short, ceiling)?;
            if holds(person) && person.pod.resize(bigger).await.is_ok() {
                return Some(format!(
                    "glave: this computer now has {}.",
                    bigger.said(short)
                ));
            }
            person.pod.sizing.wake_at(bigger);
            Some(format!(
                "glave: this computer gets {} when it next wakes.",
                bigger.said(short)
            ))
        }
        Short::Memory => {
            if !person.pod.ran_out_of_memory().await {
                return None;
            }
            let said = match now.grown(short, ceiling) {
                Some(bigger) => {
                    person.pod.sizing.wake_at(bigger);
                    format!(
                        "glave: this computer ran out of memory at {} and restarts with {}; \
                         anything it ran in the background stopped. Run the command again.",
                        now.said(short),
                        bigger.said(short)
                    )
                }
                None => format!(
                    "glave: this computer ran out of memory at {}, as much as it gets.",
                    now.said(short)
                ),
            };
            let awake = match &*person.state.borrow() {
                State::Awake(_, home) => Some(home.clone()),
                _ => None,
            };
            if let Some(home) = awake {
                slept(person, &home).await;
            }
            Some(said)
        }
    }
}
