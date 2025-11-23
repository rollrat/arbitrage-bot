use async_trait::async_trait;

use interface::{ExchangeError, ExchangeId, PerpSnapshot, SpotSnapshot};

pub mod binance;
pub mod bitget;
pub mod bithumb;
pub mod bybit;
pub mod okx;

#[async_trait]
pub trait PerpExchange: Send + Sync {
    fn id(&self) -> ExchangeId;

    async fn fetch_all(&self) -> Result<Vec<PerpSnapshot>, ExchangeError>;
}

#[async_trait]
pub trait SpotExchange: Send + Sync {
    fn id(&self) -> ExchangeId;

    async fn fetch_all(&self) -> Result<Vec<SpotSnapshot>, ExchangeError>;
}

// Convenience re-exports
pub use binance::{BinanceClient, BinanceSpotClient};
pub use bitget::{BitgetClient, BitgetSpotClient};
pub use bithumb::BithumbSpotClient;
pub use bybit::{BybitClient, BybitSpotClient};
pub use okx::{OkxClient, OkxSpotClient};
