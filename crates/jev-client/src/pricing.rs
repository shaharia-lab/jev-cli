//! Built-in prices, and cost estimates derived from token usage.
//!
//! Prices are data taken from the upstream models page. They are keyed by **versioned** model id
//! because an alias such as `jev-latest` moves without notice. A model that is not in the table
//! has no price here: callers get `None` and must not substitute a guess.
//!
//! Every cost this module returns is an *estimate*. The invoice is the source of truth.
//!
//! ```
//! use jev_client::pricing::{self, Price};
//! use jev_client::Usage;
//!
//! let usage = Usage::new(1_000_000, 50_000);
//!
//! assert_eq!(pricing::estimate_cost_usd("jev-1.13.0", usage), Some(0.042));
//! assert_eq!(pricing::estimate_cost_usd("jev-latest", usage), None);
//!
//! // A caller with its own rate (the CLI's `pricing.usd_per_mtok` override) builds a price.
//! let negotiated = Price::from_usd_per_mtok(0.03).unwrap();
//! assert_eq!(negotiated.estimate_usd(usage), 0.03);
//! ```

use crate::response::Usage;

/// Number of tokens in an "Mtok", the unit upstream prices are quoted in.
const TOKENS_PER_MTOK: f64 = 1_000_000.0;

/// Built-in prices: versioned model id and USD per million input tokens.
const PRICES: &[(&str, f64)] = &[("jev-1.13.0", 0.042)];

/// The price of a model's input tokens. Output tokens are free.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    usd_per_mtok: f64,
}

impl Price {
    /// A price of `usd_per_mtok` US dollars per million input tokens.
    ///
    /// Returns `None` unless the rate is a finite number that is zero or greater.
    #[must_use]
    pub fn from_usd_per_mtok(usd_per_mtok: f64) -> Option<Self> {
        (usd_per_mtok.is_finite() && usd_per_mtok >= 0.0).then_some(Self { usd_per_mtok })
    }

    /// The rate, in US dollars per million input tokens.
    #[must_use]
    pub const fn usd_per_mtok(self) -> f64 {
        self.usd_per_mtok
    }

    /// The estimated cost of `usage` in US dollars, counting input tokens only.
    #[must_use]
    pub fn estimate_usd(self, usage: Usage) -> f64 {
        // Token counts are far below 2^53, the point where a `u64` stops converting exactly.
        #[allow(clippy::cast_precision_loss)]
        let input_tokens = usage.input_tokens as f64;

        // Dividing first keeps round numbers of tokens exact: one million tokens is exactly 1.0.
        input_tokens / TOKENS_PER_MTOK * self.usd_per_mtok
    }
}

/// The built-in price for a versioned model id, or `None` when the model is not known.
///
/// Aliases are deliberately absent. Price the versioned id that
/// [`Response::model`](crate::Response::model) reports, not the name the request used.
#[must_use]
pub fn price_for(model: &str) -> Option<Price> {
    PRICES
        .iter()
        .find(|(id, _)| *id == model)
        .map(|&(_, usd_per_mtok)| Price { usd_per_mtok })
}

/// The estimated cost of `usage` on `model` in US dollars, or `None` for an unknown model.
#[must_use]
pub fn estimate_cost_usd(model: &str, usage: Usage) -> Option<f64> {
    price_for(model).map(|price| price.estimate_usd(usage))
}

#[cfg(test)]
mod tests {
    use super::{PRICES, Price, estimate_cost_usd, price_for};
    use crate::Usage;

    #[test]
    fn one_million_input_tokens_on_jev_1_13_0_cost_exactly_0_042() {
        let cost = estimate_cost_usd("jev-1.13.0", Usage::new(1_000_000, 0));

        assert_eq!(cost, Some(0.042));
    }

    #[test]
    fn output_tokens_are_free() {
        let with_output = estimate_cost_usd("jev-1.13.0", Usage::new(465, 1_000_000));
        let without = estimate_cost_usd("jev-1.13.0", Usage::new(465, 0));

        assert_eq!(with_output, without);
        assert_eq!(
            estimate_cost_usd("jev-1.13.0", Usage::new(0, 99)),
            Some(0.0)
        );
    }

    #[test]
    fn an_unknown_model_has_no_price_and_is_never_guessed() {
        for model in [
            "jev-latest",
            "jev-preview",
            "jev-1.13",
            "jev-1.13.1",
            "JEV-1.13.0",
            "",
        ] {
            assert_eq!(price_for(model), None, "{model:?}");
            assert_eq!(
                estimate_cost_usd(model, Usage::new(1_000_000, 0)),
                None,
                "{model:?}"
            );
        }
    }

    #[test]
    fn small_requests_cost_a_proportional_fraction() {
        let cost = estimate_cost_usd("jev-1.13.0", Usage::new(465, 73)).unwrap();

        assert!((cost - 0.000_019_53).abs() < 1e-12, "{cost}");
    }

    #[test]
    fn a_custom_price_rejects_rates_that_are_not_meaningful() {
        assert_eq!(
            Price::from_usd_per_mtok(0.0).map(Price::usd_per_mtok),
            Some(0.0)
        );
        for rate in [-0.01, f64::NAN, f64::INFINITY] {
            assert_eq!(Price::from_usd_per_mtok(rate), None, "{rate}");
        }
    }

    #[test]
    fn every_built_in_price_is_a_valid_rate_for_a_versioned_id() {
        for &(model, rate) in PRICES {
            assert_eq!(Price::from_usd_per_mtok(rate), price_for(model), "{model}");
            assert!(
                model.split('.').count() == 3,
                "{model} is not a versioned id"
            );
        }
    }
}
