//! A configured price: what a target's tokens cost, so a call can be priced before it is sent
//! and after, when the provider reports no cost of its own.

use glave_event::{Cost, Currency, Money, Nanos, TokenCounts};

/// A target's price per million tokens, in billionths of `currency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Price {
    /// The price list the numbers come from, recorded with every cost computed from them.
    pub list: String,
    pub currency: Currency,
    /// Input tokens, cache writes included.
    pub input_per_mtok: Nanos,
    /// Input tokens read from the provider's cache.
    pub cache_read_per_mtok: Nanos,
    /// Output tokens, reasoning included.
    pub output_per_mtok: Nanos,
}

impl Price {
    /// What `counts` cost, in billionths of the currency: rounded up, so a call that cost
    /// anything never counts as free, and at the most there can be if it would not fit.
    pub(crate) fn nanos(&self, counts: &TokenCounts) -> Nanos {
        let fresh = counts.input.saturating_sub(counts.cache_read);
        let at = |tokens: u64, per_mtok: Nanos| {
            let per_mtok = u128::try_from(per_mtok.get()).unwrap_or(0);
            u128::from(tokens).saturating_mul(per_mtok)
        };
        let total = at(fresh, self.input_per_mtok)
            .saturating_add(at(counts.cache_read, self.cache_read_per_mtok))
            .saturating_add(at(counts.output, self.output_per_mtok));
        i64::try_from(total.div_ceil(1_000_000)).map_or(Nanos::MAX, Nanos::new)
    }

    /// The cost of `counts`, as `model.responded` records it.
    pub(crate) fn cost(&self, counts: &TokenCounts) -> Cost {
        Cost {
            amount: Money {
                nanos: self.nanos(counts),
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
            input_per_mtok: Nanos::new(3_000_000_000),
            cache_read_per_mtok: Nanos::new(300_000_000),
            output_per_mtok: Nanos::new(15_000_000_000),
        };
        let counts = TokenCounts {
            input: 43_003,
            cache_read: 40_000,
            cache_write: 3_000,
            output: 120,
            reasoning: 0,
        };
        // 3,003 fresh × $3 + 40,000 cached × $0.30 + 120 out × $15, per million.
        let exact = 9_009_000 + 12_000_000 + 1_800_000;
        assert_eq!(price.nanos(&counts), Nanos::new(exact));
        let one = TokenCounts {
            input: 1,
            ..TokenCounts::default()
        };
        assert_eq!(
            price.nanos(&one),
            Nanos::new(3_000),
            "three millionths of a dollar exactly"
        );
        let tiny = Price {
            input_per_mtok: Nanos::new(1),
            ..price.clone()
        };
        assert_eq!(
            tiny.nanos(&one),
            Nanos::new(1),
            "a fraction of a billionth is never free"
        );
        assert_eq!(price.cost(&counts).price_list, "test-2026-10");
    }

    #[test]
    fn a_cost_at_the_limits_stops_at_the_most_there_can_be() {
        let price = Price {
            list: "test".into(),
            currency: Currency::USD,
            input_per_mtok: Nanos::MAX,
            cache_read_per_mtok: Nanos::MAX,
            output_per_mtok: Nanos::MAX,
        };
        let counts = TokenCounts {
            input: u64::MAX,
            cache_read: u64::MAX / 2,
            cache_write: 0,
            output: u64::MAX,
            reasoning: 0,
        };
        assert_eq!(price.nanos(&counts), Nanos::MAX);
    }

    #[test]
    fn a_price_below_nothing_costs_nothing() {
        let price = Price {
            list: "test".into(),
            currency: Currency::USD,
            input_per_mtok: Nanos::new(-1),
            cache_read_per_mtok: Nanos::new(-1),
            output_per_mtok: Nanos::new(-1),
        };
        let counts = TokenCounts {
            input: 10,
            ..TokenCounts::default()
        };
        assert_eq!(price.nanos(&counts), Nanos::ZERO);
    }

    proptest::proptest! {
        /// Exactly what integers say: the sum of each kind of token at its price, rounded up.
        #[test]
        fn a_cost_is_exact_and_rounded_up_only_past_a_billionth(
            fresh in 0_u64..10_u64.pow(9),
            cached in 0_u64..10_u64.pow(9),
            output in 0_u64..10_u64.pow(9),
            per_mtok in 0_i64..10_i64.pow(12),
        ) {
            let price = Price {
                list: "test".into(),
                currency: Currency::USD,
                input_per_mtok: Nanos::new(per_mtok),
                cache_read_per_mtok: Nanos::new(per_mtok),
                output_per_mtok: Nanos::new(per_mtok),
            };
            let counts = TokenCounts {
                input: fresh + cached,
                cache_read: cached,
                output,
                ..TokenCounts::default()
            };
            let scaled = i128::from(fresh + cached + output) * i128::from(per_mtok);
            let cost = i128::from(price.nanos(&counts).get());
            proptest::prop_assert!(cost * 1_000_000 >= scaled);
            proptest::prop_assert!((cost - 1) * 1_000_000 < scaled);
        }
    }
}
