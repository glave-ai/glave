//! A configured price: what a target's tokens cost, so a call can be priced before it is sent
//! and after, when the provider reports no cost of its own.

use glave_event::{Cost, Currency, Money, Nanos, TokenCounts};

/// A target's price per million tokens, in billionths of `currency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Price {
    /// The price list the numbers come from, recorded with every cost computed from them.
    pub list: String,
    pub currency: Currency,
    /// Input tokens neither read from nor written to the provider's cache.
    pub input_per_mtok: Nanos,
    /// Input tokens read from the provider's cache.
    pub cache_read_per_mtok: Nanos,
    /// Input tokens written to the provider's cache.
    pub(crate) cache_write_per_mtok: Nanos,
    /// Output tokens, reasoning included.
    pub output_per_mtok: Nanos,
    /// Dearer prices for every token of a call whose prompt is longer than its maker names.
    pub(crate) long: Option<LongPrice>,
    /// Image tokens made, per million, for an image model billed by the image token: what its
    /// answer is priced at where that is dearer than text, since its answer is its images and no
    /// provider glave reaches reports image tokens apart yet.
    pub(crate) image_output_per_mtok: Option<Nanos>,
    /// What each call costs besides its tokens.
    pub(crate) per_request: Nanos,
}

/// The prices of a call whose prompt is over `above` tokens, per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LongPrice {
    pub(crate) above: u64,
    pub(crate) input_per_mtok: Nanos,
    pub(crate) cache_read_per_mtok: Nanos,
    pub(crate) cache_write_per_mtok: Nanos,
    pub(crate) output_per_mtok: Nanos,
}

impl Price {
    /// `input`, `cache_read` and `output` tokens at these prices per million from `list`, in
    /// dollars, a cache write at the price of input.
    pub fn per_mtok(
        list: impl Into<String>,
        input: Nanos,
        cache_read: Nanos,
        output: Nanos,
    ) -> Self {
        Self {
            list: list.into(),
            currency: Currency::USD,
            input_per_mtok: input,
            cache_read_per_mtok: cache_read,
            cache_write_per_mtok: input,
            output_per_mtok: output,
            long: None,
            image_output_per_mtok: None,
            per_request: Nanos::ZERO,
        }
    }

    /// The prices `counts` are charged at: the long prompt's on every token when the prompt is
    /// over its length, else the price's own; an image model's answer at its image tokens'
    /// price where that is dearer.
    fn rates(&self, counts: &TokenCounts) -> LongPrice {
        let own = LongPrice {
            above: u64::MAX,
            input_per_mtok: self.input_per_mtok,
            cache_read_per_mtok: self.cache_read_per_mtok,
            cache_write_per_mtok: self.cache_write_per_mtok,
            output_per_mtok: self.output_per_mtok,
        };
        let mut rates = (self.long)
            .filter(|long| counts.input > long.above)
            .unwrap_or(own);
        if let Some(images) = self.image_output_per_mtok {
            rates.output_per_mtok = rates.output_per_mtok.max(images);
        }
        rates
    }

    /// What `counts` cost, in billionths of the currency, its price per request included:
    /// rounded up, so a call that cost anything never counts as free, and at the most there can
    /// be if it would not fit.
    pub(crate) fn nanos(&self, counts: &TokenCounts) -> Nanos {
        let rates = self.rates(counts);
        let fresh = (counts.input)
            .saturating_sub(counts.cache_read)
            .saturating_sub(counts.cache_write);
        let at = |tokens: u64, per_mtok: Nanos| {
            let per_mtok = u128::try_from(per_mtok.get()).unwrap_or(0);
            u128::from(tokens).saturating_mul(per_mtok)
        };
        let total = at(fresh, rates.input_per_mtok)
            .saturating_add(at(counts.cache_read, rates.cache_read_per_mtok))
            .saturating_add(at(counts.cache_write, rates.cache_write_per_mtok))
            .saturating_add(at(counts.output, rates.output_per_mtok));
        let tokens = i64::try_from(total.div_ceil(1_000_000)).map_or(Nanos::MAX, Nanos::new);
        tokens.saturating_add(self.per_request.max(Nanos::ZERO))
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
        let price = Price::per_mtok(
            "test-2026-10",
            Nanos::new(3_000_000_000),
            Nanos::new(300_000_000),
            Nanos::new(15_000_000_000),
        );
        let counts = TokenCounts {
            input: 43_003,
            cache_read: 40_000,
            cache_write: 3_000,
            output: 120,
            reasoning: 0,
        };
        // 3 fresh and 3,000 written × $3 + 40,000 cached × $0.30 + 120 out × $15, per million.
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
            per_request: Nanos::MAX,
            ..Price::per_mtok("test", Nanos::MAX, Nanos::MAX, Nanos::MAX)
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
            per_request: Nanos::new(-1),
            ..Price::per_mtok("test", Nanos::new(-1), Nanos::new(-1), Nanos::new(-1))
        };
        let counts = TokenCounts {
            input: 10,
            ..TokenCounts::default()
        };
        assert_eq!(price.nanos(&counts), Nanos::ZERO);
    }

    /// Claude Haiku 5.5's prices: dearer on every token above 100,000 in the prompt.
    fn haiku() -> Price {
        Price {
            cache_write_per_mtok: Nanos::new(125_000_000),
            long: Some(LongPrice {
                above: 100_000,
                input_per_mtok: Nanos::new(500_000_000),
                cache_read_per_mtok: Nanos::new(50_000_000),
                cache_write_per_mtok: Nanos::new(625_000_000),
                output_per_mtok: Nanos::new(2_500_000_000),
            }),
            ..Price::per_mtok(
                "test",
                Nanos::new(100_000_000),
                Nanos::new(10_000_000),
                Nanos::new(500_000_000),
            )
        }
    }

    #[test]
    fn a_long_prompt_is_dearer_on_every_token_and_cache_writes_are_priced_apart() {
        let counts = |input: u64| TokenCounts {
            input,
            cache_read: 60_000,
            cache_write: 20_000,
            output: 1_000,
            reasoning: 0,
        };
        // 20,000 fresh × $0.10 + 60,000 read × $0.01 + 20,000 written × $0.125 + 1,000 × $0.50.
        let short = 2_000_000 + 600_000 + 2_500_000 + 500_000;
        assert_eq!(haiku().nanos(&counts(100_000)), Nanos::new(short));
        // 20,001 fresh × $0.50 + 60,000 × $0.05 + 20,000 × $0.625 + 1,000 × $2.50.
        let long = 10_000_500 + 3_000_000 + 12_500_000 + 2_500_000;
        assert_eq!(haiku().nanos(&counts(100_001)), Nanos::new(long));
    }

    #[test]
    fn an_image_model_s_answer_is_priced_as_images_and_a_request_has_its_own_price() {
        let gpt_image = Price {
            image_output_per_mtok: Some(Nanos::new(30_000_000_000)),
            ..Price::per_mtok("test", Nanos::new(5_000_000_000), Nanos::ZERO, Nanos::ZERO)
        };
        let counts = TokenCounts {
            input: 100,
            output: 1_000,
            ..TokenCounts::default()
        };
        assert_eq!(gpt_image.nanos(&counts), Nanos::new(500_000 + 30_000_000));
        let search = Price {
            per_request: Nanos::new(5_000_000),
            ..Price::per_mtok("test", Nanos::ZERO, Nanos::ZERO, Nanos::ZERO)
        };
        assert_eq!(search.nanos(&TokenCounts::default()), Nanos::new(5_000_000));
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
            let price = Price::per_mtok(
                "test",
                Nanos::new(per_mtok),
                Nanos::new(per_mtok),
                Nanos::new(per_mtok),
            );
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
