//! A price as a price list writes it (`specs/gateway.md`, "Prices"): what a use of one model at
//! one service costs, in US dollars written as decimal text and read exactly, never through a
//! float, with the maker's page that says so and the day the price last changed. glave's own
//! list is made of these.

use glave_event::{Money, Nanos};
use jiff::civil::Date;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// One model's price at one service. Token prices are per million tokens.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceEntry {
    /// The service, as receipts name it: `anthropic`, `openai`, or another server's host.
    pub service: String,
    /// The model, by its own id as that service's API names it.
    pub model: String,
    /// Input tokens.
    #[serde(default, deserialize_with = "dollars")]
    pub input: Option<Nanos>,
    /// Output tokens, reasoning included.
    #[serde(default, deserialize_with = "dollars")]
    pub output: Option<Nanos>,
    /// Input read from the service's cache, where it bills that apart.
    #[serde(default, deserialize_with = "dollars")]
    pub cache_read: Option<Nanos>,
    /// Input written to the service's cache, where it bills that apart.
    #[serde(default, deserialize_with = "dollars")]
    pub cache_write: Option<Nanos>,
    /// Image tokens out, for a model the maker bills by the image token.
    #[serde(default, deserialize_with = "dollars")]
    pub image_output: Option<Nanos>,
    /// One request, where there is a price for one.
    #[serde(default, deserialize_with = "dollars")]
    pub per_request: Option<Nanos>,
    /// Dearer prices for a call whose prompt is longer than its maker names, on every token.
    #[serde(default)]
    pub long: Option<LongPrompt>,
    /// The day the price last changed, as a person checked it against `page` then.
    pub date: Date,
    /// The maker's own page that says the price.
    pub page: String,
}

/// The prices of a call whose prompt is over `above` tokens, per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongPrompt {
    /// The longest prompt, in tokens, the entry's own prices cover.
    pub above: u64,
    #[serde(deserialize_with = "required_dollars")]
    pub input: Nanos,
    #[serde(deserialize_with = "required_dollars")]
    pub output: Nanos,
    #[serde(default, deserialize_with = "dollars")]
    pub cache_read: Option<Nanos>,
    #[serde(default, deserialize_with = "dollars")]
    pub cache_write: Option<Nanos>,
}

/// Why an entry can't price anything.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the price of {model:?} at {service:?} {reason}")]
pub struct PriceError {
    pub service: String,
    pub model: String,
    pub reason: &'static str,
}

impl PriceEntry {
    /// Whether the entry prices something, as the spec says one must: a price per request
    /// alone, or input with output or image tokens out; and names its maker's page.
    pub fn check(&self) -> Result<(), PriceError> {
        let refused = |reason| {
            Err(PriceError {
                service: self.service.clone(),
                model: self.model.clone(),
                reason,
            })
        };
        let named = |name: &str| !name.is_empty() && name.trim() == name;
        if !named(&self.service) || !named(&self.model) {
            return refused("names no service or no model");
        }
        if !self.page.starts_with("https://") || self.page.contains(char::is_whitespace) {
            return refused("cites no https page of its maker's");
        }
        let tokens = [
            self.input,
            self.output,
            self.cache_read,
            self.cache_write,
            self.image_output,
        ];
        let by_tokens = tokens.iter().any(Option::is_some) || self.long.is_some();
        let out = self.output.or(self.image_output);
        if (by_tokens || self.per_request.is_none()) && (self.input.is_none() || out.is_none()) {
            return refused("has no price for input and for what it makes");
        }
        if self.long.is_some_and(|long| long.above == 0) {
            return refused("is dearer above a prompt of no tokens");
        }
        Ok(())
    }
}

/// US dollars as decimal text, such as `"0.125"`, read exactly: never below nothing.
fn dollars<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Nanos>, D::Error> {
    required_dollars(deserializer).map(Some)
}

fn required_dollars<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Nanos, D::Error> {
    let text = String::deserialize(deserializer)?;
    Money::parse_dollars(&text)
        .ok()
        .map(|money| money.nanos)
        .filter(|nanos| *nanos >= Nanos::ZERO)
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not dollars, such as \"0.50\""))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> Result<PriceEntry, serde_json::Error> {
        serde_json::from_str(text)
    }

    const PAGE: &str = r#""date": "2026-11-24", "page": "https://example.com/pricing""#;

    #[test]
    fn prices_are_read_exactly_from_decimal_text() {
        let read = entry(&format!(
            r#"{{"service": "anthropic", "model": "m", "input": "0.125", "output": "2", {PAGE}}}"#
        ))
        .unwrap();
        assert_eq!(read.input, Some(Nanos::new(125_000_000)));
        assert_eq!(read.output, Some(Nanos::new(2_000_000_000)));
        assert_eq!(read.check(), Ok(()));
        for wrong in [r#""-1""#, r#""1e-3""#, "0.5", r#""""#] {
            let text = format!(
                r#"{{"service": "s", "model": "m", "input": {wrong}, "output": "1", {PAGE}}}"#
            );
            assert!(entry(&text).is_err(), "{wrong} is not dollars");
        }
    }

    #[test]
    fn an_entry_prices_something_and_cites_its_page() {
        let checked = |fields: &str, page: &str| {
            let text = format!(
                r#"{{"service": "s", "model": "m", {fields} "date": "2026-11-24", "page": "{page}"}}"#
            );
            entry(&text).unwrap().check().map_err(|error| error.reason)
        };
        let page = "https://example.com/pricing";
        assert_eq!(checked(r#""input": "1", "output": "2","#, page), Ok(()));
        assert_eq!(checked(r#""per_request": "0.005","#, page), Ok(()));
        assert_eq!(
            checked(r#""input": "5", "image_output": "30","#, page),
            Ok(())
        );
        assert!(checked(r#""input": "1","#, page).is_err(), "no output");
        assert!(checked("", page).is_err(), "nothing priced");
        assert!(
            checked(r#""per_request": "1", "output": "2","#, page).is_err(),
            "tokens without input"
        );
        assert!(checked(r#""input": "1", "output": "2","#, "http://example.com").is_err());
        assert!(checked(r#""input": "1", "output": "2","#, "").is_err());
        let no_date = r#"{"service": "s", "model": "m", "input": "1", "output": "2", "page": "https://example.com"}"#;
        assert!(
            entry(no_date).is_err(),
            "an entry without its date is refused"
        );
    }
}
