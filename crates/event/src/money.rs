//! Money as an exact integer count of billionths plus a currency, never a float.

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

/// An exact count of billionths of one unit of a currency, the unit Google's billing uses: in
/// 64 bits it holds about nine billion dollars. The one way glave counts money, and, as what a
/// million tokens cost, prices.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct Nanos(i64);

/// Billionths in one unit.
const UNIT: u64 = 1_000_000_000;

/// Decimal places a count of [`Nanos`] keeps.
const PLACES: u32 = 9;

/// Decimal places of a price per token held per million tokens: [`PLACES`] and six more.
const PER_TOKEN_PLACES: u32 = 15;

/// Billionths in one hundredth of a unit: a cent of a dollar.
const CENT: i64 = 10_000_000;

impl Nanos {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(i64::MAX);

    pub const fn new(nanos: i64) -> Self {
        Self(nanos)
    }

    pub const fn get(self) -> i64 {
        self.0
    }

    /// The price of a million tokens from what a service lists for one, in dollars as decimal
    /// text such as `0.000003` or `3e-6`: exact, with no float on the way, and past the last
    /// place kept rounded up, so a price is never free by rounding. `None` unless it is a
    /// number at or above zero that fits.
    pub fn per_million_tokens(dollars_per_token: &str) -> Option<Self> {
        scaled(dollars_per_token, PER_TOKEN_PLACES)
            .map(Self)
            .filter(|price| *price >= Self::ZERO)
    }

    /// The sum, or `None` when it would not fit.
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    /// The sum, or the most or least there can be.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// The difference, or the most or least there can be.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// The amount alone as a plain decimal such as `0.0042`, formatted as [`Money`] is.
    pub const fn decimal(self) -> Decimal {
        Decimal(self)
    }
}

/// An exact amount: `nanos` billionths of one unit of `currency`. `{"nanos": 4200000,
/// "currency": "USD"}` is $0.0042.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub nanos: Nanos,
    pub currency: Currency,
}

impl Money {
    /// US dollars written as a number, as a person or a provider writes one: `2`, `0.50`,
    /// `-0.0042` or `4e-7`. Past the ninth decimal place it rounds away from zero, so an amount
    /// is never nothing by rounding; a leading `+`, a bare point and anything else are refused.
    pub fn parse_dollars(text: &str) -> Result<Self, InvalidValue> {
        let nanos = scaled(text, PLACES).ok_or_else(|| {
            InvalidValue::new(
                "amount",
                format!("{text:?} is not dollars, such as 2 or 0.50"),
            )
        })?;
        Ok(Self {
            nanos: Nanos(nanos),
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
            nanos: self.nanos.checked_add(other.nanos)?,
            currency: self.currency,
        })
    }

    /// The amount rounded up to the next cent, as what something could cost is said: for
    /// showing, never for counting.
    #[must_use]
    pub fn cents_up(self) -> Self {
        let nanos = self.nanos.get();
        Self {
            nanos: Nanos(
                nanos
                    .div_euclid(CENT)
                    .saturating_add(i64::from(nanos.rem_euclid(CENT) != 0))
                    .saturating_mul(CENT),
            ),
            ..self
        }
    }

    /// The amount rounded down to the cent, as what is left is said: for showing, never for
    /// counting.
    #[must_use]
    pub fn cents_down(self) -> Self {
        Self {
            nanos: Nanos(self.nanos.get().div_euclid(CENT).saturating_mul(CENT)),
            ..self
        }
    }

    /// The amount alone, without its currency, as a plain decimal such as `0.0042`.
    pub const fn decimal(self) -> Decimal {
        self.nanos.decimal()
    }
}

/// `$1.5` for US dollars, `1.5 EUR` for any other currency. Without a precision the amount
/// is exact and as short as it can be; `{:.2}` rounds half away from zero to two places.
impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let amount = decimal(self.nanos.get(), f.precision());
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
/// `{:.9}`, `0.004200000`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decimal(Nanos);

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&decimal(self.0.get(), f.precision()))
    }
}

/// `nanos` as a decimal: exact and shortest without `places`, else rounded half away from
/// zero to that many places.
fn decimal(nanos: i64, places: Option<usize>) -> String {
    let magnitude = nanos.unsigned_abs();
    let Some(places) = places else {
        let sign = if nanos < 0 { "-" } else { "" };
        let (whole, fraction) = (magnitude / UNIT, magnitude % UNIT);
        if fraction == 0 {
            return format!("{sign}{whole}");
        }
        let fraction = format!("{fraction:09}");
        return format!("{sign}{whole}.{}", fraction.trim_end_matches('0'));
    };
    let kept = places.min(PLACES as usize);
    // A power of ten from 1 (all nine places kept) to a billion (none kept).
    let step = (kept..PLACES as usize).fold(1_u64, |step, _| step.saturating_mul(10));
    let half_or_more = (magnitude % step).saturating_mul(2) >= step && step > 1;
    let rounded = (magnitude / step).saturating_add(u64::from(half_or_more));
    let scale = UNIT / step;
    let (whole, fraction) = (rounded / scale, rounded % scale);
    let sign = if nanos < 0 && rounded > 0 { "-" } else { "" };
    if places == 0 {
        return format!("{sign}{whole}");
    }
    let pad = places.saturating_sub(kept);
    format!("{sign}{whole}.{fraction:0kept$}{:0<pad$}", "")
}

/// The number `text` writes, in units of ten to the minus `places`, worked out digit by digit
/// with no float on the way. Past the last place it rounds away from zero, so nothing but zero
/// becomes zero. `None` unless it is a decimal, with an exponent if any, that fits.
fn scaled(text: &str, places: u32) -> Option<i64> {
    let (negative, number) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = match number.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (number, 0),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((_, "")) => return None,
        Some(parts) => parts,
        None => (mantissa, ""),
    };
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !all_digits(whole) || !all_digits(fraction) {
        return None;
    }
    let mut digits = whole
        .bytes()
        .chain(fraction.bytes())
        .map(|b| u128::from(b - b'0'));
    let kept = i128::try_from(whole.len()).ok()? + i128::from(exponent) + i128::from(places);
    let mut magnitude = 0_u128;
    for _ in 0..kept {
        let digit = digits.next();
        // Zeros after zero, however many an exponent asks for, stay zero; any other number
        // runs out of room within forty.
        if digit.is_none() && magnitude == 0 {
            break;
        }
        magnitude = magnitude.checked_mul(10)?.checked_add(digit.unwrap_or(0))?;
    }
    if digits.any(|digit| digit != 0) {
        magnitude = magnitude.checked_add(1)?;
    }
    let magnitude = i128::try_from(magnitude).ok()?;
    i64::try_from(if negative { -magnitude } else { magnitude }).ok()
}

#[cfg(test)]
mod tests;
