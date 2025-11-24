pub mod state;
pub mod strategy;

pub use super::trader::binance::BinanceTrader;
pub use state::ArbitrageState;
pub use strategy::{
    cross_basis::CrossBasisArbitrageStrategy, intra_basis::IntraBasisArbitrageStrategy,
    StrategyParams,
};
