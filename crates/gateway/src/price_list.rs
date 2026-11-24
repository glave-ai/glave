//! glave's own price list (`specs/gateway.md`, "Prices"): what each model costs at each of the
//! makers Glave Cloud pays directly, as their own pages say, in `prices.toml`, built into glave
//! and never fetched. A call is priced nearest first: the settings of whoever pays the provider,
//! then this list where it names the model at that service, then the service's own catalog;
//! else it has no price.

use std::collections::BTreeMap;

use glave_event::Nanos;
use glave_ports::settings::{PriceEntry, PriceError, check_prices};
use serde::Deserialize;
use thiserror::Error;

use crate::price::{LongPrice, Price};
use crate::target::Target;

/// The list, as glave was built with it.
const BUILT_IN: &str = include_str!("../prices.toml");

/// What a price the payer's own settings set is recorded as.
const PAYER: &str = "the server's settings";

/// Why a price list could not be read: one glave was built with, which its tests read, or the
/// payer's own prices, which their settings checked as they were read.
#[derive(Debug, Error)]
pub enum PriceListError {
    #[error("glave's price list does not read: {0}")]
    Unreadable(String),
    #[error(transparent)]
    Unpriced(#[from] PriceError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    prices: Vec<PriceEntry>,
}

/// glave's own price list: each model's price at each service it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceList {
    /// Its name, recorded with every cost computed from it: `glave` and its latest entry's date.
    name: String,
    /// Each entry, by its service and model.
    entries: BTreeMap<(String, String), PriceEntry>,
    /// The payer's own prices, which come before this list's.
    payer: Option<Box<PriceList>>,
}

impl PriceList {
    /// The list glave was built with.
    pub fn built_in() -> Result<Self, PriceListError> {
        Self::read(BUILT_IN)
    }

    /// This list under the payer's own `prices` (`[[prices]]` in their settings): each
    /// replaces this list's for its service and model, or adds one, recorded as the server's
    /// settings.
    pub fn under(self, prices: Vec<PriceEntry>) -> Result<Self, PriceListError> {
        if prices.is_empty() {
            return Ok(self);
        }
        let payer = Self::of(PAYER.to_owned(), prices)?;
        Ok(Self {
            payer: Some(Box::new(payer)),
            ..self
        })
    }

    /// The list `text` writes: each entry checked, none twice.
    fn read(text: &str) -> Result<Self, PriceListError> {
        let file: File =
            toml::from_str(text).map_err(|error| PriceListError::Unreadable(error.to_string()))?;
        let latest = file.prices.iter().map(|entry| entry.date).max();
        let name = latest.map_or_else(|| "glave".to_owned(), |date| format!("glave {date}"));
        Self::of(name, file.prices)
    }

    /// The list `name` of `prices`: each checked, none twice.
    fn of(name: String, prices: Vec<PriceEntry>) -> Result<Self, PriceListError> {
        check_prices(&prices)?;
        let entries = prices
            .into_iter()
            .map(|entry| ((entry.service.clone(), entry.model.clone()), entry))
            .collect();
        Ok(Self {
            name,
            entries,
            payer: None,
        })
    }

    /// The price of `model` at `service`, if the payer's prices or the list name it: by its
    /// id, else by the id without the snapshot date at its end (Anthropic's
    /// `claude-haiku-4-5-20251001`, OpenAI's `gpt-5.2-2025-12-11`), which a maker gives a model
    /// that also goes by the id without it.
    pub(crate) fn price(&self, service: &str, model: &str) -> Option<Price> {
        if let Some(price) = self
            .payer
            .as_ref()
            .and_then(|payer| payer.price(service, model))
        {
            return Some(price);
        }
        let at = |model: &str| self.entries.get(&(service.to_owned(), model.to_owned()));
        let entry = at(model).or_else(|| at(undated(model)?))?;
        Some(self.priced_entry(entry))
    }

    /// `target` at the payer's price, else the list's, where either names its model at its
    /// service, which wins over its catalog's; else as it was.
    #[must_use]
    pub fn priced(&self, target: Target) -> Target {
        match self.price(&target.provider, &target.model) {
            Some(price) => target.with_price(price),
            None => target,
        }
    }

    /// What `entry` charges, as the gateway prices a call: what it doesn't price apart costs as
    /// input does, or nothing for a price per request alone.
    fn priced_entry(&self, entry: &PriceEntry) -> Price {
        let input = entry.input.unwrap_or(Nanos::ZERO);
        let long = entry.long.map(|long| LongPrice {
            above: long.above,
            input_per_mtok: long.input,
            cache_read_per_mtok: long.cache_read.unwrap_or(long.input),
            cache_write_per_mtok: long.cache_write.unwrap_or(long.input),
            output_per_mtok: long.output,
        });
        Price {
            cache_write_per_mtok: entry.cache_write.unwrap_or(input),
            long,
            image_output_per_mtok: entry.image_output,
            per_request: entry.per_request.unwrap_or(Nanos::ZERO),
            ..Price::per_mtok(
                self.name.clone(),
                input,
                entry.cache_read.unwrap_or(input),
                entry.output.unwrap_or(Nanos::ZERO),
            )
        }
    }
}

/// `model` without a snapshot date at its end, if it has one: `-YYYYMMDD` or `-YYYY-MM-DD`.
fn undated(model: &str) -> Option<&str> {
    let digits = |text: &str, count: usize| {
        text.len() == count && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    let (base, last) = model.rsplit_once('-')?;
    if digits(last, 8) {
        return Some(base);
    }
    let (base, month) = base.rsplit_once('-')?;
    let (base, year) = base.rsplit_once('-')?;
    (digits(year, 4) && digits(month, 2) && digits(last, 2)).then_some(base)
}

#[cfg(test)]
mod tests;
