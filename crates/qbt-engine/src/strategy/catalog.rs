use super::{Regime, Strategy};
use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};

pub const DEFAULT_DCA_INTERVAL: u32 = 24;
pub const DEFAULT_DCA_AMOUNT: f64 = 100.0;
pub const DEFAULT_DIP_LOOKBACK: usize = 168;
pub const DEFAULT_DIP_PERCENT: f64 = 5.0;
pub const COMPARISON_ID: &str = "all";

pub(super) const DEFAULT_ENTRY_AMOUNT: f64 = 100.0;
pub(super) const DCA_ID: &str = "DCA";
pub(super) const DIP_DCA_ID: &str = "DipDCA";
pub(super) const VOL_ADJUSTED_DCA_ID: &str = "VolAdjustedDCA";
pub(super) const MOMENTUM_SMA_ID: &str = "MomentumSMA";
pub(super) const REGIME_SWITCH_ID: &str = "RegimeSwitch";
pub const DEFAULT_STRATEGY: &str = DCA_ID;

const MOMENTUM_LOOKBACK: u32 = 20;
const MOMENTUM_THRESHOLD: f64 = 0.01;
const VOL_LOOKBACK: u32 = 30;
const VOL_MULTIPLIER_MAX: f64 = 2.0;
const FAST_SMA: u32 = 10;
const SLOW_SMA: u32 = 50;
const BUY_REGIME: Regime = Regime::Bull;
const SELL_REGIME: Regime = Regime::Bear;

const ORDER_AMOUNT: &str = "order_amount";
const DCA_INTERVAL: &str = "dca_interval";
const DCA_AMOUNT: &str = "dca_amount";
const DIP_LOOKBACK: &str = "dip_lookback";
const DIP_PERCENT: &str = "dip_percent";

#[derive(Serialize)]
#[serde(untagged)]
enum Setting {
    Integer(u32),
    Number(f64),
    Regime(Regime),
}

#[derive(Serialize)]
struct FixedParameter {
    id: &'static str,
    default: Setting,
}

#[derive(Serialize)]
pub struct StrategyDefinition {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub parameter_ids: &'static [&'static str],
    fixed_parameters: &'static [FixedParameter],
}

impl StrategyDefinition {
    pub fn fixed_settings(&self) -> impl Iterator<Item = (&'static str, Value)> {
        self.fixed_parameters
            .iter()
            .map(|field| (field.id, json!(field.default)))
    }
}

// This order also determines how the engine executes comparison runs.
pub const STRATEGIES: [StrategyDefinition; 5] = [
    StrategyDefinition {
        id: DCA_ID,
        aliases: &[],
        description: "Dollar-cost averaging at fixed intervals",
        parameter_ids: &[ORDER_AMOUNT, DCA_INTERVAL, DCA_AMOUNT],
        fixed_parameters: &[],
    },
    StrategyDefinition {
        id: DIP_DCA_ID,
        aliases: &[],
        description: "Accumulate daily cash and buy on a drawdown",
        parameter_ids: &[ORDER_AMOUNT, DCA_AMOUNT, DIP_LOOKBACK, DIP_PERCENT],
        fixed_parameters: &[],
    },
    StrategyDefinition {
        id: VOL_ADJUSTED_DCA_ID,
        aliases: &["VOLDCA"],
        description: "Increase DCA amount when volatility is high",
        parameter_ids: &[ORDER_AMOUNT, DCA_AMOUNT],
        fixed_parameters: &[
            FixedParameter {
                id: "vol_lookback",
                default: Setting::Integer(VOL_LOOKBACK),
            },
            FixedParameter {
                id: "vol_multiplier_max",
                default: Setting::Number(VOL_MULTIPLIER_MAX),
            },
        ],
    },
    StrategyDefinition {
        id: MOMENTUM_SMA_ID,
        aliases: &[],
        description: "Enter above the moving average and exit below it",
        parameter_ids: &[ORDER_AMOUNT],
        fixed_parameters: &[
            FixedParameter {
                id: "lookback",
                default: Setting::Integer(MOMENTUM_LOOKBACK),
            },
            FixedParameter {
                id: "buy_threshold",
                default: Setting::Number(MOMENTUM_THRESHOLD),
            },
            FixedParameter {
                id: "sell_threshold",
                default: Setting::Number(MOMENTUM_THRESHOLD),
            },
            FixedParameter {
                id: "default_entry_amount",
                default: Setting::Number(DEFAULT_ENTRY_AMOUNT),
            },
        ],
    },
    StrategyDefinition {
        id: REGIME_SWITCH_ID,
        aliases: &[],
        description: "Switch positions based on market regime detection",
        parameter_ids: &[ORDER_AMOUNT],
        fixed_parameters: &[
            FixedParameter {
                id: "fast_sma",
                default: Setting::Integer(FAST_SMA),
            },
            FixedParameter {
                id: "slow_sma",
                default: Setting::Integer(SLOW_SMA),
            },
            FixedParameter {
                id: "buy_regime",
                default: Setting::Regime(BUY_REGIME),
            },
            FixedParameter {
                id: "sell_regime",
                default: Setting::Regime(SELL_REGIME),
            },
            FixedParameter {
                id: "default_entry_amount",
                default: Setting::Number(DEFAULT_ENTRY_AMOUNT),
            },
        ],
    },
];

/// Strategy IDs in canonical execution order.
pub const BUILTINS: [&str; STRATEGIES.len()] = {
    let mut names = [""; STRATEGIES.len()];
    let mut i = 0;
    while i < STRATEGIES.len() {
        names[i] = STRATEGIES[i].id;
        i += 1;
    }
    names
};

/// Metadata for the current build's runnable strategies and their CLI options.
pub fn strategy_catalog() -> Value {
    json!({
        "version": 1,
        "default_strategy": DEFAULT_STRATEGY,
        "comparison_id": COMPARISON_ID,
        "strategies": STRATEGIES,
        "parameters": [
            {
                "id": ORDER_AMOUNT,
                "label": "Order amount",
                "description": "Entry notional, or daily reserve allocation for DipDCA. VolAdjustedDCA uses one twenty-fourth as the hourly base.",
                "type": "number",
                "default": null,
                "exclusive_minimum": 0,
                "advanced": false,
            },
            {
                "id": DCA_INTERVAL,
                "label": "DCA interval (hours)",
                "description": "Hours between DCA buy signals.",
                "type": "integer",
                "default": DEFAULT_DCA_INTERVAL,
                "minimum": 1,
                "maximum": u32::MAX,
                "advanced": true,
            },
            {
                "id": DCA_AMOUNT,
                "label": "DCA amount",
                "description": "Fallback amount for DCA, DipDCA and VolAdjustedDCA when Order amount is blank.",
                "type": "number",
                "default": DEFAULT_DCA_AMOUNT,
                "exclusive_minimum": 0,
                "advanced": true,
            },
            {
                "id": DIP_LOOKBACK,
                "label": "DipDCA lookback (hours)",
                "description": "Trailing hourly closes used to find the high.",
                "type": "integer",
                "default": DEFAULT_DIP_LOOKBACK,
                "minimum": 2,
                "advanced": true,
            },
            {
                "id": DIP_PERCENT,
                "label": "DipDCA drop (%)",
                "description": "Percent below the trailing high required to signal a buy.",
                "type": "number",
                "default": DEFAULT_DIP_PERCENT,
                "exclusive_minimum": 0,
                "exclusive_maximum": 100,
                "advanced": true,
            },
        ],
    })
}

pub fn builtin_strategy(
    name: &str,
    interval: u32,
    amount: f64,
    vol_base: f64,
    dip_lookback: usize,
    dip_percent: f64,
) -> Result<Strategy> {
    let resolve = STRATEGIES.iter().find(|definition| {
        definition.id.eq_ignore_ascii_case(name)
            || definition
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(name))
    });
    Ok(match resolve.map(|definition| definition.id) {
        Some(DCA_ID) => Strategy::DCA {
            interval_hours: interval,
            amount,
        },
        Some(DIP_DCA_ID) => Strategy::DipDCA {
            daily_allocation: amount,
            lookback_hours: dip_lookback,
            drop_fraction: dip_percent / 100.0,
        },
        Some(VOL_ADJUSTED_DCA_ID) => Strategy::VolAdjustedDCA {
            base_amount: vol_base,
            vol_lookback: VOL_LOOKBACK,
            vol_multiplier_max: VOL_MULTIPLIER_MAX,
        },
        Some(MOMENTUM_SMA_ID) => Strategy::MomentumSMA {
            lookback: MOMENTUM_LOOKBACK,
            buy_threshold: MOMENTUM_THRESHOLD,
            sell_threshold: MOMENTUM_THRESHOLD,
        },
        Some(REGIME_SWITCH_ID) => Strategy::RegimeSwitch {
            fast_sma: FAST_SMA,
            slow_sma: SLOW_SMA,
            buy_regime: BUY_REGIME,
            sell_regime: SELL_REGIME,
        },
        _ => anyhow::bail!("Unknown strategy: {}", name),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_and_aliases_resolve_to_the_same_strategies() -> Result<()> {
        assert_eq!(
            BUILTINS,
            [
                "DCA",
                "DipDCA",
                "VolAdjustedDCA",
                "MomentumSMA",
                "RegimeSwitch"
            ]
        );
        let resolve = |name: &str| {
            builtin_strategy(
                name,
                DEFAULT_DCA_INTERVAL,
                DEFAULT_DCA_AMOUNT,
                DEFAULT_DCA_AMOUNT,
                DEFAULT_DIP_LOOKBACK,
                DEFAULT_DIP_PERCENT,
            )
        };
        for definition in &STRATEGIES {
            let expected = serde_json::to_value(resolve(definition.id)?)?;
            for name in std::iter::once(&definition.id).chain(definition.aliases) {
                let strategy = resolve(&name.to_ascii_lowercase())?;
                assert_eq!(strategy.name(), definition.id);
                assert_eq!(serde_json::to_value(strategy)?, expected);
            }
        }
        assert!(resolve("Custom").is_err());
        assert!(resolve(COMPARISON_ID).is_err());
        Ok(())
    }
}
