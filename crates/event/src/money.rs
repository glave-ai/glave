//! Money as an exact integer count of millionths plus a currency, never a float.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::InvalidValue;

/// An ISO 4217 currency code: three uppercase ASCII letters, such as `USD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Currency([u8; 3]);

impl Currency {
    pub const USD: Self = Self(*b"USD");

    pub fn new(code: &str) -> Result<Self, InvalidValue> {
        let bytes: [u8; 3] = code
            .as_bytes()
            .try_into()
            .map_err(|_| InvalidValue::new("currency", format!("{code:?} is not three letters")))?;
        if bytes.iter().all(u8::is_ascii_uppercase) {
            Ok(Self(bytes))
        } else {
            Err(InvalidValue::new(
                "currency",
                format!("{code:?} must be three uppercase letters"),
            ))
        }
    }
}

impl TryFrom<String> for Currency {
    type Error = InvalidValue;

    fn try_from(code: String) -> Result<Self, Self::Error> {
        Self::new(&code)
    }
}

impl From<Currency> for String {
    fn from(currency: Currency) -> Self {
        currency.to_string()
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|b| write!(f, "{}", char::from(*b)))
    }
}

/// An exact amount: `micros` millionths of one unit of `currency`. `{"micros": 4200,
/// "currency": "USD"}` is $0.0042.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub micros: i64,
    pub currency: Currency,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_codes_are_validated() {
        assert_eq!(Currency::new("EUR").unwrap().to_string(), "EUR");
        for bad in ["usd", "US", "USDT", "U$D", ""] {
            assert!(Currency::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn money_round_trips_exactly() {
        let cost = Money {
            micros: 4_200,
            currency: Currency::USD,
        };
        let json = serde_json::to_string(&cost).unwrap();
        assert_eq!(json, r#"{"micros":4200,"currency":"USD"}"#);
        assert_eq!(serde_json::from_str::<Money>(&json).unwrap(), cost);
    }
}
