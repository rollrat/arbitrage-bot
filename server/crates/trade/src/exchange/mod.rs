use async_trait::async_trait;

use interface::{
    Asset, DepositWithdrawalFee, ExchangeError, ExchangeId, FeeInfo, MarketType, OrderBook,
};

pub mod bithumb;

#[async_trait]
pub trait AssetExchange: Send + Sync {
    fn id(&self) -> ExchangeId;

    async fn fetch_assets(&self) -> Result<Vec<Asset>, ExchangeError>;
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

pub use bithumb::{BithumbAssetClient, BithumbOrderBookClient};
