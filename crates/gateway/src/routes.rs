//! Routes: which targets serve each purpose, in the order they are tried.

use std::collections::HashMap;

use glave_ports::Purpose;
use thiserror::Error;

use crate::target::Target;

/// The targets for each purpose. `main` always has at least one; any other purpose without a
/// route of its own uses `main`'s.
#[derive(Debug, Clone)]
pub struct Routes {
    main: Vec<Target>,
    others: HashMap<Purpose, Vec<Target>>,
}

/// Why routes could not be built.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RouteError {
    #[error("the {0} route names no targets")]
    Empty(&'static str),
}

impl Routes {
    /// Routes where every purpose is served by `main`, tried in order.
    pub fn new(main: Vec<Target>) -> Result<Self, RouteError> {
        if main.is_empty() {
            return Err(RouteError::Empty(Purpose::Main.id()));
        }
        Ok(Self {
            main,
            others: HashMap::new(),
        })
    }

    /// Gives `purpose` its own targets, replacing any it had (`main`'s too).
    pub fn with_route(
        mut self,
        purpose: Purpose,
        targets: Vec<Target>,
    ) -> Result<Self, RouteError> {
        if targets.is_empty() {
            return Err(RouteError::Empty(purpose.id()));
        }
        if purpose == Purpose::Main {
            self.main = targets;
        } else {
            self.others.insert(purpose, targets);
        }
        Ok(self)
    }

    /// The targets that serve `purpose`, in the order they are tried.
    pub fn targets(&self, purpose: Purpose) -> &[Target] {
        self.others.get(&purpose).unwrap_or(&self.main)
    }
}
