//! A glave as what it starts outside this computer's processes names it (workers in containers
//! or Pods): the machine it runs on, its process and when that started, so a glave starting
//! later tells whether it is gone, as the ledger does for processes.

use serde::{Deserialize, Serialize};

use crate::start::{self, Start};

/// A glave, by its machine, its process id and that process's start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Glave {
    /// The machine's name, as the system gives it: an id and a start are only compared on the
    /// machine (or in the container) they were read on.
    host: String,
    pid: i32,
    start: Start,
}

impl Glave {
    /// This glave; none where its machine's name or its start cannot be read. Blocking.
    pub fn this() -> Option<Self> {
        let pid = i32::try_from(std::process::id()).ok()?;
        let start = start::of(&[pid]).remove(&pid)?;
        Some(Self {
            host: host()?,
            pid,
            start,
        })
    }

    /// For each of `glaves`, whether it is known to be gone: it ran on this machine, and no
    /// process here has its id with its start. One from another machine is never taken for
    /// gone, nor any when this glave's own start cannot be read, which means the starts
    /// could not be read at all. Blocking.
    pub fn gone(glaves: &[Self]) -> Vec<bool> {
        let none = vec![false; glaves.len()];
        let (Some(here), Some(this)) = (host(), Self::this()) else {
            return none;
        };
        let mut pids: Vec<i32> = glaves
            .iter()
            .filter(|glave| glave.host == here)
            .map(|glave| glave.pid)
            .collect();
        pids.push(this.pid);
        let starts = start::of(&pids);
        if starts.get(&this.pid) != Some(&this.start) {
            return none;
        }
        glaves
            .iter()
            .map(|glave| glave.host == here && starts.get(&glave.pid) != Some(&glave.start))
            .collect()
    }
}

/// This machine's name; none where it has none.
fn host() -> Option<String> {
    let name = rustix::system::uname().nodename().to_str().ok()?.to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn with(glave: &Glave, field: &str, value: serde_json::Value) -> Glave {
        let mut changed = serde_json::to_value(glave).unwrap();
        changed[field] = value;
        serde_json::from_value(changed).unwrap()
    }

    #[test]
    fn this_glave_is_not_gone_and_one_with_another_start_is() {
        let this = Glave::this().expect("this glave's start can be read");
        let restarted = with(&this, "start", json!("another start"));
        assert_eq!(Glave::gone(&[this, restarted]), vec![false, true]);
    }

    #[test]
    fn a_glave_on_another_machine_is_never_taken_for_gone() {
        let this = Glave::this().unwrap();
        let elsewhere = with(&this, "host", json!("another machine"));
        let elsewhere = with(&elsewhere, "start", json!("another start"));
        assert_eq!(Glave::gone(&[elsewhere]), vec![false]);
        assert_eq!(Glave::gone(&[]), Vec::<bool>::new());
    }
}
