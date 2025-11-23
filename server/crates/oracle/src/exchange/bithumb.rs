use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use interface::{Currency, ExchangeId, SpotSnapshot};

use super::{ExchangeError, SpotExchange};

const BASE_URL: &str = "https://api.bithumb.com";

#[derive(Clone)]
pub struct BithumbSpotClient {
    http: reqwest::Client,
}

impl BithumbSpotClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct BithumbResponse {
    status: String,
    data: HashMap<String, serde_json::Value>, // 일부 필드는 문자열일 수 있으므로 Value로 받음
}

#[derive(Debug, Deserialize)]
struct BithumbTicker {
    #[serde(rename = "closing_price")]
    closing_price: String,
    #[serde(rename = "acc_trade_value_24H")]
    acc_trade_value_24h: String, // 24h 거래량 (원화 기준)
}

#[async_trait]
impl SpotExchange for BithumbSpotClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bithumb
    }

    async fn fetch_all(&self) -> Result<Vec<SpotSnapshot>, ExchangeError> {
        // 빗썸은 원화(KRW) 거래쌍을 제공
        let url = format!("{BASE_URL}/public/ticker/ALL_KRW");
        let response: BithumbResponse = self.http.get(&url).send().await?.json().await?;

        if response.status != "0000" {
            return Err(ExchangeError::Other(format!(
                "Bithumb API error: status {}",
                response.status
            )));
        }

        let now = Utc::now();
        let mut out = Vec::new();

        for (symbol, value) in response.data {
            // "date" 필드는 건너뛰기 (문자열 타임스탬프)
            if symbol == "date" {
                continue;
            }

            // Value를 BithumbTicker로 파싱 시도
            let ticker: BithumbTicker = match serde_json::from_value(value) {
                Ok(t) => t,
                Err(_) => continue, // 파싱 실패 시 건너뛰기
            };

            // 빗썸은 "BTC", "ETH" 형식이므로 "BTCUSDT"로 변환
            // 빗썸은 원화 거래쌍이지만, 통일성을 위해 USDT 형식으로 변환
            let symbol_usdt = format!("{}USDT", symbol);

            let price: f64 = match ticker.closing_price.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // price가 0보다 큰 경우만 추가
            if price <= 0.0 {
                continue;
            }

            // 빗썸은 원화 기준 거래량이므로, USD로 변환 필요
            // 하지만 정확한 환율 정보가 없으므로 일단 원화 거래량을 그대로 사용
            // 또는 0으로 설정하고 나중에 환율 정보를 추가할 수 있음
            let vol_24h_krw: f64 = ticker.acc_trade_value_24h.parse().unwrap_or(0.0);
            // 원화를 USD로 변환 (대략 1 USD = 1300 KRW 가정, 실제로는 환율 API 필요)
            let vol_24h_usd = vol_24h_krw / 1300.0;

            out.push(SpotSnapshot {
                exchange: ExchangeId::Bithumb,
                symbol: symbol_usdt,
                currency: Currency::KRW, // 빗썸은 원화 거래쌍
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
    fn test_bithumb_spot_client_id() {
        let client = BithumbSpotClient::new();
        assert_eq!(client.id(), ExchangeId::Bithumb);
    }

    #[tokio::test]
    async fn test_fetch_all_bithumb_spot() {
        let client = BithumbSpotClient::new();
        let result = client.fetch_all().await;

        match result {
            Ok(snapshots) => {
                // API 호출이 성공했는지 확인
                assert!(!snapshots.is_empty(), "snapshots should not be empty");

                // 모든 스냅샷이 Bithumb 거래소인지 확인
                for snapshot in &snapshots {
                    assert_eq!(snapshot.exchange, ExchangeId::Bithumb);
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
                    if msg.contains("Bithumb API error") {
                        panic!("Bithumb API error: {}", msg);
                    }
                }
                // 네트워크 오류는 테스트 환경에 따라 실패할 수 있으므로 경고만
                eprintln!("Warning: fetch_all failed: {:?}", e);
            }
        }
    }
}
