use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
