//! The secrets a person added, as the agent's work meets them (`specs/model.md`, "Secrets"):
//! commands see each one's placeholder in its variable, never its value, which only the
//! outbound proxy puts in, for the hosts it may reach; and a value that shows in a command's or
//! a tool's output anyway is masked before it is recorded or sent to the model.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use glave_event::{Part, Secret};
use serde_json::Value;

use crate::ToolOutput;
use crate::auth::redact;
use crate::computer::Output;
use crate::tools::BoxFuture;

/// What a placeholder starts with, so the proxy knows one when a request carries it.
const PLACEHOLDER: &str = "glave-secret-";

/// One secret, as commands and their output meet it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The variable commands see it as: `STRIPE_KEY`.
    pub name: String,
    /// What commands see in place of its value: the same for as long as the name is kept, so
    /// replacing the value changes nothing a command or a file holds.
    pub placeholder: String,
    pub value: Secret,
}

impl Known {
    /// The secret seen as `name`, with its `value`.
    pub fn new(name: String, value: Secret) -> Self {
        Self {
            placeholder: format!("{PLACEHOLDER}{name}"),
            name,
            value,
        }
    }
}

/// The person's secrets now, shared: whatever keeps them sets them each time they change, and
/// every workspace reads them for each command and each tool's output.
#[derive(Debug, Clone, Default)]
pub struct KnownSecrets(Arc<RwLock<Vec<Known>>>);

impl KnownSecrets {
    /// The secrets from now on, `known`.
    pub fn set(&self, mut known: Vec<Known>) {
        // The longest first, so a value inside another is never masked out of it first.
        known.sort_by_key(|known| std::cmp::Reverse(known.value.expose().len()));
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = known;
    }

    /// Each secret's variable and its placeholder, for a command's environment.
    pub fn placeholders(&self) -> BTreeMap<String, String> {
        let known = self.0.read().unwrap_or_else(PoisonError::into_inner);
        known
            .iter()
            .map(|known| (known.name.clone(), known.placeholder.clone()))
            .collect()
    }

    /// `text` with every secret's value in it replaced by `[redacted]`.
    pub fn mask(&self, text: &str) -> String {
        let known = self.0.read().unwrap_or_else(PoisonError::into_inner);
        redact(text, known.iter().map(|known| known.value.expose()))
    }

    /// `output` with every secret's value in its texts and its record masked, before it is
    /// recorded or the model is shown it.
    pub fn masked(&self, mut output: ToolOutput) -> ToolOutput {
        for part in &mut output.parts {
            if let Part::Text { text } = part {
                *text = self.mask(text);
            }
        }
        if let Some(record) = &mut output.structured {
            self.mask_value(record);
        }
        output
    }

    /// Every string in `value` masked, where it is: its own text, never its JSON's.
    fn mask_value(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.mask(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.mask_value(item)),
            Value::Object(fields) => fields.values_mut().for_each(|field| self.mask_value(field)),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// `inner`, given a command's output masked as it comes, so a value is masked before any of
    /// it is kept or cut, however the writes split it. [`Masking::flush`] once the command ends.
    pub fn masking<'a>(&'a self, inner: &'a dyn Output) -> Masking<'a> {
        Masking {
            known: self,
            inner,
            held: Mutex::new(String::new()),
        }
    }

    /// The bytes of the longest value.
    fn longest(&self) -> usize {
        let known = self.0.read().unwrap_or_else(PoisonError::into_inner);
        known
            .iter()
            .map(|known| known.value.expose().len())
            .max()
            .unwrap_or(0)
    }
}

/// A command's output passed on masked: what could be the start of a value waits for what
/// follows it.
pub struct Masking<'a> {
    known: &'a KnownSecrets,
    inner: &'a dyn Output,
    /// Output not passed on yet, masked: at most the longest value less one byte.
    held: Mutex<String>,
}

impl Masking<'_> {
    /// Passes on what is held: the command ended.
    pub async fn flush(&self) {
        let rest = std::mem::take(&mut *self.held());
        if !rest.is_empty() {
            self.inner.write(&rest).await;
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, String> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Output for Masking<'_> {
    fn write<'b>(&'b self, text: &'b str) -> BoxFuture<'b, ()> {
        let ready = {
            let mut held = self.held();
            held.push_str(text);
            let mut ready = self.known.mask(&held);
            let keep = self.known.longest().saturating_sub(1);
            let mut at = ready.len().saturating_sub(keep);
            while !ready.is_char_boundary(at) {
                at = at.saturating_sub(1);
            }
            *held = ready.split_off(at);
            ready
        };
        Box::pin(async move {
            if !ready.is_empty() {
                self.inner.write(&ready).await;
            }
        })
    }
}

#[cfg(test)]
mod tests;
