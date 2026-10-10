//! A configured price: what a target's tokens cost, so a call can be priced before it is sent
//! and after, when the provider reports no cost of its own.

use glave_event::{Cost, Currency, Money, TokenCounts};

/// A target's price per million tokens, in millionths of `currency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Price {
    /// The price list the numbers come from, recorded with every cost computed from them.
    pub list: String,
    pub currency: Currency,
    /// Input tokens, cache writes included.
    pub input_per_mtok: u64,
    /// Input tokens read from the provider's cache.
    pub cache_read_per_mtok: u64,
    /// Output tokens, reasoning included.
    pub output_per_mtok: u64,
}

impl Price {
    /// What `counts` cost, in millionths of the currency, rounded up.
    pub(crate) fn micros(&self, counts: &TokenCounts) -> u64 {
        let fresh = counts.input.saturating_sub(counts.cache_read);
        let millionths = u128::from(fresh) * u128::from(self.input_per_mtok)
            + u128::from(counts.cache_read) * u128::from(self.cache_read_per_mtok)
            + u128::from(counts.output) * u128::from(self.output_per_mtok);
        u64::try_from(millionths.div_ceil(1_000_000)).unwrap_or(u64::MAX)
    }

    /// The cost of `counts`, as `model.responded` records it.
    pub(crate) fn cost(&self, counts: &TokenCounts) -> Cost {
        Cost {
            amount: Money {
                micros: i64::try_from(self.micros(counts)).unwrap_or(i64::MAX),
                currency: self.currency,
            },
            price_list: self.list.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_input_is_priced_apart_and_costs_round_up() {
        let price = Price {
            list: "test-2026-10".into(),
            currency: Currency::USD,
            input_per_mtok: 3_000_000,
            cache_read_per_mtok: 300_000,
            output_per_mtok: 15_000_000,
        };
        let counts = TokenCounts {
            input: 43_003,
            cache_read: 40_000,
            cache_write: 3_000,
            output: 120,
            reasoning: 0,
        };
        // 3,003 fresh × $3 + 40,000 cached × $0.30 + 120 out × $15, per million.
        assert_eq!(price.micros(&counts), 9_009 + 12_000 + 1_800);
        let one = TokenCounts {
            input: 1,
            ..TokenCounts::default()
        };
        assert_eq!(price.micros(&one), 3, "three millionths exactly");
        let tiny = Price {
            input_per_mtok: 1,
            ..price.clone()
        };
        assert_eq!(tiny.micros(&one), 1, "a fraction of a millionth rounds up");
        assert_eq!(price.cost(&counts).price_list, "test-2026-10");
    }
}
