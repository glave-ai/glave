//! Money as an exact integer count of millionths plus a currency, never a float.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::InvalidValue;

/// An ISO 4217 currency code: three uppercase ASCII letters, such as `USD`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub micros: i64,
    pub currency: Currency,
}

/// Millionths in one unit.
const MICROS: u64 = 1_000_000;

/// Millionths in one hundredth of a unit: a cent of a dollar.
const CENT: i64 = 10_000;

impl Money {
    /// US dollars written as a plain decimal, such as `2`, `0.50` or `-0.0042`: digits, then
    /// a point and more digits if any. Past the sixth decimal place it rounds half away from
    /// zero; exponents, a leading `+` and a bare point are refused.
    pub fn parse_dollars(text: &str) -> Result<Self, InvalidValue> {
        let micros = micros_of(text).ok_or_else(|| {
            InvalidValue::new(
                "amount",
                format!("{text:?} is not dollars, such as 2 or 0.50"),
            )
        })?;
        Ok(Self {
            micros,
            currency: Currency::USD,
        })
    }

    /// The sum of two amounts in one currency; `None` when the currencies differ or the sum
    /// overflows.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        if self.currency != other.currency {
            return None;
        }
        Some(Self {
            micros: self.micros.checked_add(other.micros)?,
            currency: self.currency,
        })
    }

    /// The amount rounded up to the next cent, as what something could cost is said.
    #[must_use]
    pub fn cents_up(self) -> Self {
        Self {
            micros: self
                .micros
                .div_euclid(CENT)
                .saturating_add(i64::from(self.micros.rem_euclid(CENT) != 0))
                .saturating_mul(CENT),
            ..self
        }
    }

    /// The amount rounded down to the cent, as what is left is said.
    #[must_use]
    pub fn cents_down(self) -> Self {
        Self {
            micros: self.micros.div_euclid(CENT).saturating_mul(CENT),
            ..self
        }
    }

    /// The amount alone, without its currency, as a plain decimal such as `0.0042`.
    pub fn decimal(self) -> Decimal {
        Decimal(self.micros)
    }
}

/// `$1.5` for US dollars, `1.5 EUR` for any other currency. Without a precision the amount
/// is exact and as short as it can be; `{:.2}` rounds half away from zero to two places.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let amount = decimal(self.micros, f.precision());
        if self.currency != Currency::USD {
            return write!(f, "{amount} {}", self.currency);
        }
        match amount.strip_prefix('-') {
            Some(magnitude) => write!(f, "-${magnitude}"),
            None => write!(f, "${amount}"),
        }
    }
}

/// An amount of money without its currency, formatted as [`Money`] is: `0.0042`, or with
/// `{:.6}`, `0.004200`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal(i64);

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&decimal(self.0, f.precision()))
    }
}

/// `micros` as a decimal: exact and shortest without `places`, else rounded half away from
/// zero to that many places.
fn decimal(micros: i64, places: Option<usize>) -> String {
    let magnitude = micros.unsigned_abs();
    let Some(places) = places else {
        let sign = if micros < 0 { "-" } else { "" };
        let (whole, fraction) = (magnitude / MICROS, magnitude % MICROS);
        if fraction == 0 {
            return format!("{sign}{whole}");
        }
        let fraction = format!("{fraction:06}");
        return format!("{sign}{whole}.{}", fraction.trim_end_matches('0'));
    };
    let kept = places.min(6);
    // A power of ten from 1 (all six places kept) to a million (none kept).
    let step = (kept..6).fold(1_u64, |step, _| step.saturating_mul(10));
    let half_or_more = (magnitude % step).saturating_mul(2) >= step && step > 1;
    let rounded = (magnitude / step).saturating_add(u64::from(half_or_more));
    let scale = MICROS / step;
    let (whole, fraction) = (rounded / scale, rounded % scale);
    let sign = if micros < 0 && rounded > 0 { "-" } else { "" };
    if places == 0 {
        return format!("{sign}{whole}");
    }
    let pad = places.saturating_sub(kept);
    format!("{sign}{whole}.{fraction:0kept$}{:0<pad$}", "")
}

/// The millionths `text` names, rounded half away from zero past the sixth place; `None`
/// unless it is a plain decimal that fits.
fn micros_of(text: &str) -> Option<i64> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((_, "")) => return None,
        Some(parts) => parts,
        None => (digits, ""),
    };
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !all_digits(whole) || !all_digits(fraction) {
        return None;
    }
    let mut micros = whole.parse::<i64>().ok()?.checked_mul(1_000_000)?;
    let mut fraction = fraction.bytes().map(|b| i64::from(b - b'0'));
    let mut scale = 100_000;
    for _ in 0..6 {
        let digit = fraction.next().unwrap_or(0);
        micros = micros.checked_add(digit.checked_mul(scale)?)?;
        scale /= 10;
    }
    if fraction.next().is_some_and(|digit| digit >= 5) {
        micros = micros.checked_add(1)?;
    }
    Some(if negative { -micros } else { micros })
}

#[cfg(test)]
mod tests;
