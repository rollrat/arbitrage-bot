use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{Currency, ExchangeId, PerpSnapshot, SpotSnapshot};

use super::{ExchangeError, PerpExchange, SpotExchange};

const BASE_URL: &str = "https://api.bybit.com";

#[derive(Clone)]
pub struct BybitClient {
    http: reqwest::Client,
}

impl BybitClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitTickerResponse {
    ret_code: i32,
    ret_msg: String,
    result: BybitTickerResult,
}

#[derive(Debug, Deserialize)]
struct BybitTickerResult {
    #[allow(dead_code)]
    category: String,
    list: Vec<BybitTicker>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitTicker {
    symbol: String,
    #[serde(default)]
    mark_price: String,
    #[serde(default)]
    funding_rate: String,
    #[serde(default)]
    open_interest: String,
    #[serde(default)]
    turnover24h: String,
    #[serde(default)]
    next_funding_time: String,
}

#[async_trait]
impl PerpExchange for BybitClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bybit
    }

    async fn fetch_all(&self) -> Result<Vec<PerpSnapshot>, ExchangeError> {
        let url = format!("{BASE_URL}/v5/market/tickers?category=linear");
        let response: BybitTickerResponse = self.http.get(&url).send().await?.json().await?;

        if response.ret_code != 0 {
            return Err(ExchangeError::Other(format!(
                "Bybit API error: {} - {}",
                response.ret_code, response.ret_msg
            )));
        }

        let now = Utc::now();
        let mut out = Vec::new();

        for ticker in response.result.list {
            if !ticker.symbol.ends_with("USDT") {
                continue; // 선형 USDT perp만
            }

            let mark_price: f64 = match ticker.mark_price.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let funding_rate: f64 = ticker.funding_rate.parse().unwrap_or(0.0);

            let oi_contracts: f64 = ticker.open_interest.parse().unwrap_or(0.0);
            let oi_usd = oi_contracts * mark_price;

            let vol_24h_usd: f64 = ticker.turnover24h.parse().unwrap_or(0.0);

            let next_funding_time: Option<DateTime<Utc>> = if !ticker.next_funding_time.is_empty() {
                ticker
                    .next_funding_time
                    .parse::<i64>()
                    .ok()
                    .and_then(|ts| DateTime::from_timestamp_millis(ts))
            } else {
                None
            };

            out.push(PerpSnapshot {
                exchange: ExchangeId::Bybit,
                symbol: ticker.symbol,
                currency: Currency::USDT,
                mark_price,
                oi_usd,
                vol_24h_usd,
                funding_rate,
                next_funding_time,
                updated_at: now,
            });
        }

        Ok(out)
    }
}

#[derive(Clone)]
pub struct BybitSpotClient {
    http: reqwest::Client,
}

impl BybitSpotClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitSpotTicker {
    symbol: String,
    #[serde(default)]
    last_price: String,
    #[serde(default)]
    turnover24h: String,
}

#[derive(Debug, Deserialize)]
struct BybitSpotTickerResult {
    #[allow(dead_code)]
    category: String,
    list: Vec<BybitSpotTicker>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BybitSpotTickerResponse {
    ret_code: i32,
    ret_msg: String,
    result: BybitSpotTickerResult,
}

#[async_trait]
impl SpotExchange for BybitSpotClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bybit
    }

    async fn fetch_all(&self) -> Result<Vec<SpotSnapshot>, ExchangeError> {
        let url = format!("{BASE_URL}/v5/market/tickers?category=spot");
        let response: BybitSpotTickerResponse = self.http.get(&url).send().await?.json().await?;

        if response.ret_code != 0 {
            return Err(ExchangeError::Other(format!(
                "Bybit API error (spot): {} - {}",
                response.ret_code, response.ret_msg
            )));
        }

        let now = Utc::now();
        let mut out = Vec::new();

        for ticker in response.result.list {
            if !ticker.symbol.ends_with("USDT") {
                continue; // USDT 페어만
            }

            let price: f64 = match ticker.last_price.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // price가 0보다 큰 경우만 추가
            if price <= 0.0 {
                continue;
            }

            let vol_24h_usd: f64 = ticker.turnover24h.parse().unwrap_or(0.0);

            out.push(SpotSnapshot {
                exchange: ExchangeId::Bybit,
                symbol: ticker.symbol,
                currency: Currency::USDT,
                price,
                vol_24h_usd,
                updated_at: now,
            });
        }

        Ok(out)
    }
}
