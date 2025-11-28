use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExchangeId {
    Binance,
    Bybit,
    Okx,
    Bitget,
    Bithumb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Currency {
    USD,
    KRW,
    USDT,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerpSnapshot {
    pub exchange: ExchangeId,
    pub symbol: String,
    pub currency: Currency,
    pub mark_price: f64,
    pub oi_usd: f64,
    pub vol_24h_usd: f64,
    pub funding_rate: f64, // 0.01 == 1%
    pub next_funding_time: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotSnapshot {
    pub exchange: ExchangeId,
    pub symbol: String,
    pub currency: Currency,
    pub price: f64,
    pub vol_24h_usd: f64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSnapshot {
    pub exchange: ExchangeId,
    pub symbol: String,
    pub currency: Currency,
    // 선물 데이터
    pub perp: Option<PerpData>,
    // 현물 데이터
    pub spot: Option<SpotData>,
    // 환율 정보 (USD 기준)
    pub exchange_rates: ExchangeRates,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeRates {
    pub usd_krw: f64,  // 1 USD = ? KRW (예: 1300.0)
    pub usdt_usd: f64, // 1 USDT = ? USD (보통 1.0)
    pub usdt_krw: f64, // 1 USDT = ? KRW (예: 1300.0)
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerpData {
    pub currency: Currency,
    pub mark_price: f64,
    pub oi_usd: f64,
    pub vol_24h_usd: f64,
    pub funding_rate: f64, // 0.01 == 1%
    pub next_funding_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotData {
    pub currency: Currency,
    pub price: f64,
    pub vol_24h_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotAsset {
    pub currency: String,
    pub total: f64,     // 총 보유량
    pub available: f64, // 사용 가능한 잔액
    pub in_use: f64,    // 주문에 사용 중인 잔액 (locked)
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FutureAsset {
    pub symbol: String,
    pub position_amt: f64, // 양수면 롱, 음수면 숏
    pub updated_at: DateTime<Utc>,
}

// 하위 호환성을 위한 타입 별칭
#[deprecated(note = "Use SpotAsset instead")]
pub type Asset = SpotAsset;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub exchange: ExchangeId,
    pub symbol: String,
    pub bids: Vec<OrderBookEntry>, // 매수 주문 (가격 높은 순)
    pub asks: Vec<OrderBookEntry>, // 매도 주문 (가격 낮은 순)
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookEntry {
    pub price: f64,
    pub quantity: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketType {
    KRW,           // 원화 마켓
    USDT,          // USDT 마켓
    BTC,           // BTC 마켓
    Other(String), // 기타 마켓
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeInfo {
    pub maker: f64, // 메이커 수수료 (예: 0.0004 = 0.04%)
    pub taker: f64, // 테이커 수수료 (예: 0.0004 = 0.04%)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositWithdrawalFee {
    pub currency: String,
    pub deposit_fee: f64,    // 입금 수수료
    pub withdrawal_fee: f64, // 출금 수수료
    pub updated_at: DateTime<Utc>,
}

impl FeeInfo {
    pub fn new(maker: f64, taker: f64) -> Self {
        Self { maker, taker }
    }

    /// 수수료 무료
    pub fn free() -> Self {
        Self {
            maker: 0.0,
            taker: 0.0,
        }
    }
}

#[derive(Error, Debug)]
pub enum ExchangeError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("other error: {0}")]
    Other(String),
}
