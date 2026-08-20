//! Estimated cost.
//!
//! Every number here is an estimate twice over: the token counts come from an
//! estimator rather than the target model's tokenizer, and the price per token
//! is whatever the user typed into their configuration. So there is no built-in
//! rate. Without one, cost is simply not reported — an unconfigured `$0.00`
//! would read as "this saved nothing", and a made-up default rate would read as
//! a fact.

use serde::{Deserialize, Serialize};

/// The rate cost is derived from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostRates {
    /// The model these rates describe, shown next to every figure so a reader
    /// knows what the estimate assumes.
    pub model: String,
    /// Price of a million input tokens, in `currency`.
    pub input_per_million: f64,
    pub currency: String,
}

impl CostRates {
    /// Read the rates out of configuration, or `None` when none is set.
    pub fn from_config(config: &ctxc_core::Config) -> Option<Self> {
        let rate = config.metrics.cost_per_million_input_tokens;
        if !(rate.is_finite() && rate > 0.0) {
            return None;
        }

        Some(CostRates {
            model: config.metrics.cost_model.clone(),
            input_per_million: rate,
            currency: config.metrics.cost_currency.clone(),
        })
    }

    /// What `tokens` would have cost as input. Tokens saved are input tokens
    /// never sent, so a saving and a cost use the same rate.
    pub fn estimate(&self, tokens: i64) -> CostEstimate {
        CostEstimate {
            amount: (tokens as f64 / 1_000_000.0) * self.input_per_million,
            currency: self.currency.clone(),
            model: self.model.clone(),
            estimated: true,
        }
    }
}

/// A cost figure, permanently labelled as an estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub amount: f64,
    pub currency: String,
    /// The model whose rates produced this.
    pub model: String,
    /// Always true. Serialized anyway, so that a consumer reading only the JSON
    /// cannot present the figure as exact.
    pub estimated: bool,
}

impl CostEstimate {
    /// Render as an amount with its currency, e.g. `USD 84.21`.
    pub fn to_display(&self) -> String {
        format!("{} {:.2}", self.currency, self.amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctxc_core::Config;

    #[test]
    fn no_rate_means_no_cost_estimate() {
        let config = Config::default();
        assert_eq!(
            config.metrics.cost_per_million_input_tokens, 0.0,
            "shipping a default price would put a guess in front of the user"
        );
        assert!(CostRates::from_config(&config).is_none());
    }

    #[test]
    fn a_configured_rate_produces_an_estimate() {
        let mut config = Config::default();
        config.metrics.cost_model = "some-model".into();
        config.metrics.cost_per_million_input_tokens = 3.0;

        let rates = CostRates::from_config(&config).unwrap();
        let estimate = rates.estimate(52_500_000);

        assert!((estimate.amount - 157.5).abs() < 1e-9);
        assert_eq!(estimate.model, "some-model");
        assert!(estimate.estimated);
        assert_eq!(estimate.to_display(), "USD 157.50");
    }

    #[test]
    fn a_nonsense_rate_is_ignored_rather_than_propagated() {
        let mut config = Config::default();
        for rate in [-1.0, f64::NAN, f64::INFINITY] {
            config.metrics.cost_per_million_input_tokens = rate;
            assert!(CostRates::from_config(&config).is_none());
        }
    }
}
