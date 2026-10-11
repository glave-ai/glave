//! Routes: which targets serve each purpose, in the order they are tried.

use std::borrow::Cow;
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
    /// Models a thread may choose in place of `main`'s route.
    choices: Vec<Target>,
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
            choices: Vec::new(),
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

    /// Offers `targets` as models a thread may choose (`model/list` in the session API): a
    /// call asking for one by its name or model goes to it in place of `main`'s route.
    #[must_use]
    pub fn with_choices(mut self, targets: Vec<Target>) -> Self {
        self.choices = targets;
        self
    }

    /// The models a thread may choose besides `main`'s.
    pub fn choices(&self) -> &[Target] {
        &self.choices
    }

    /// The targets for a call for `purpose` that asks for `model`. When the purpose uses
    /// `main`'s route and `model` names one of the choices other than `main`'s first target,
    /// the default, that choice alone; otherwise the purpose's route, whatever was asked for.
    pub fn for_model(&self, purpose: Purpose, model: &str) -> Cow<'_, [Target]> {
        let route = self.targets(purpose);
        let own = purpose != Purpose::Main && self.others.contains_key(&purpose);
        let default = self.main.first().is_none_or(|first| is(first, model));
        if own || default {
            return Cow::Borrowed(route);
        }
        match self.choices.iter().find(|target| is(target, model)) {
            Some(chosen) => Cow::Owned(vec![chosen.clone()]),
            None => Cow::Borrowed(route),
        }
    }
}

/// Whether `target` is the one `model` names: by its name or its model.
fn is(target: &Target, model: &str) -> bool {
    target.model == model || target.name == model
}
