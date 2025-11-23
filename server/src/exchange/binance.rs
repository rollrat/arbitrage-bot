use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::model::{Currency, ExchangeId, PerpSnapshot, SpotSnapshot};

use super::{ExchangeError, PerpExchange, SpotExchange};

const BASE_URL: &str = "https://fapi.binance.com";
const SPOT_BASE_URL: &str = "https://api.binance.com";

#[derive(Clone)]
pub struct BinanceClient {
    http: reqwest::Client,
}

impl BinanceClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinancePremiumIndex {
    symbol: String,
    mark_price: String,
    last_funding_rate: String,
    next_funding_time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceTicker24h {
    symbol: String,
    quote_volume: String,
    #[serde(default)]
    open_interest: String,
}

#[async_trait]
impl PerpExchange for BinanceClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Binance
    }

    async fn fetch_all(&self) -> Result<Vec<PerpSnapshot>, ExchangeError> {
        // 1) funding / mark price info
        let premium: Vec<BinancePremiumIndex> = self
            .http
            .get(format!("{BASE_URL}/fapi/v1/premiumIndex"))
            .send()
            .await?
            .json()
            .await?;

        // 2) 24h ticker
        let tickers: Vec<BinanceTicker24h> = self
            .http
            .get(format!("{BASE_URL}/fapi/v1/ticker/24hr"))
            .send()
            .await?
            .json()
            .await?;

        let mut ticker_map: HashMap<String, BinanceTicker24h> = HashMap::new();
        for t in tickers {
            ticker_map.insert(t.symbol.clone(), t);
        }

        let now = Utc::now();

        let mut out = Vec::new();

        for p in premium {
            if !p.symbol.ends_with("USDT") {
                continue; // 선형 USDT perp만
            }

            let t = match ticker_map.get(&p.symbol) {
                Some(t) => t,
                None => continue,
            };

            let mark_price: f64 = match p.mark_price.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let funding_rate: f64 = p.last_funding_rate.parse().unwrap_or(0.0);

            let oi_contracts: f64 = t.open_interest.parse().unwrap_or(0.0);
            let oi_usd = oi_contracts * mark_price;

            let vol_24h_usd: f64 = t.quote_volume.parse().unwrap_or(0.0);

            let next_funding_time: Option<DateTime<Utc>> = if p.next_funding_time > 0 {
                DateTime::from_timestamp_millis(p.next_funding_time)
            } else {
                None
            };

            out.push(PerpSnapshot {
                exchange: ExchangeId::Binance,
                symbol: p.symbol,
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
pub struct BinanceSpotClient {
    http: reqwest::Client,
}

impl BinanceSpotClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceSpotTicker24h {
    symbol: String,
    last_price: String, // Binance Spot API는 lastPrice 필드를 사용
    quote_volume: String,
}

#[async_trait]
impl SpotExchange for BinanceSpotClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Binance
    }

    async fn fetch_all(&self) -> Result<Vec<SpotSnapshot>, ExchangeError> {
        let tickers: Vec<BinanceSpotTicker24h> = self
            .http
            .get(format!("{SPOT_BASE_URL}/api/v3/ticker/24hr"))
            .send()
            .await?
            .json()
            .await?;

        let now = Utc::now();
        let mut out = Vec::new();

        for ticker in tickers {
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

            let vol_24h_usd: f64 = ticker.quote_volume.parse().unwrap_or(0.0);

            out.push(SpotSnapshot {
                exchange: ExchangeId::Binance,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binance_spot_client_id() {
        let client = BinanceSpotClient::new();
        assert_eq!(client.id(), ExchangeId::Binance);
    }

    #[tokio::test]
    async fn test_fetch_all_binance_spot() {
        let client = BinanceSpotClient::new();
        let result = client.fetch_all().await;

        match result {
            Ok(snapshots) => {
                // API 호출이 성공했는지 확인
                assert!(!snapshots.is_empty(), "snapshots should not be empty");

                // 모든 스냅샷이 Binance 거래소인지 확인
                for snapshot in &snapshots {
                    println!("Binance spot snapshot: {:?}", snapshot);
                    assert_eq!(snapshot.exchange, ExchangeId::Binance);
                    assert!(snapshot.symbol.ends_with("USDT"));
                    assert!(snapshot.price > 0.0);
                    assert!(snapshot.vol_24h_usd >= 0.0);
                }

                // 심볼이 올바른지 확인 (예: BTCUSDT)
                let btc_snapshot = snapshots.iter().find(|s| s.symbol == "BTCUSDT");
                if let Some(btc) = btc_snapshot {
                    println!(
                        "Found BTCUSDT spot snapshot: price={}, vol_24h_usd={}",
                        btc.price, btc.vol_24h_usd
                    );
                }
            }
            Err(e) => {
                // 네트워크 오류 등은 테스트 실패로 간주하지 않음
                // 하지만 API 오류는 확인
                if let ExchangeError::Other(msg) = &e {
                    if msg.contains("Binance API error") {
                        panic!("Binance API error: {}", msg);
                    }
                }
                // 네트워크 오류는 테스트 환경에 따라 실패할 수 있으므로 경고만
                eprintln!("Warning: fetch_all failed: {:?}", e);

                panic!();
            }
        }
    }
}
