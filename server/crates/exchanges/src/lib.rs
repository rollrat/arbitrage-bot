use async_trait::async_trait;

use interface::{
    DepositWithdrawalFee, ExchangeError, ExchangeId, FeeInfo, FutureAsset, MarketType, OrderBook,
    PerpSnapshot, SpotAsset, SpotSnapshot,
};

pub mod binance;
pub mod bitget;
pub mod bithumb;
pub mod bybit;
pub mod exchange_rate;
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

#[async_trait]
pub trait AssetExchange: Send + Sync {
    fn id(&self) -> ExchangeId;

    async fn fetch_spots(&self) -> Result<Vec<SpotAsset>, ExchangeError>;

    async fn fetch_futures(&self) -> Result<Vec<FutureAsset>, ExchangeError>;
}

#[async_trait]
pub trait OrderBookExchange: Send + Sync {
    fn id(&self) -> ExchangeId;

    /// 특정 심볼의 Orderbook 조회
    /// symbol: 거래쌍 (예: "BTC-KRW", "USDT-KRW")
    async fn fetch_orderbook(&self, symbol: &str) -> Result<OrderBook, ExchangeError>;
}

#[async_trait]
pub trait FeeExchange: Send + Sync {
    fn id(&self) -> ExchangeId;

    /// 특정 마켓 타입의 거래 수수료 정보 조회
    /// market_type: 마켓 타입 (KRW, USDT, BTC 등)
    fn get_fee(&self, market_type: MarketType) -> FeeInfo;

    /// 특정 통화의 입출금 수수료 조회
    /// currency: 통화 코드 (예: "BTC", "ETH")
    async fn get_deposit_withdrawal_fee(
        &self,
        currency: &str,
    ) -> Result<DepositWithdrawalFee, ExchangeError>;
}

// Convenience re-exports
pub use binance::BinanceClient;
pub use bitget::BitgetClient;
pub use bithumb::BithumbClient;
pub use bybit::BybitClient;
pub use okx::OkxClient;
