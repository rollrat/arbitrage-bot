use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use tracing;

use crate::model::{Currency, ExchangeId, PerpSnapshot, SpotSnapshot};

use super::{ExchangeError, PerpExchange, SpotExchange};

const BASE_URL: &str = "https://api.bitget.com";

#[derive(Clone)]
pub struct BitgetClient {
    http: reqwest::Client,
}

impl BitgetClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

/// Bitget의 다음 펀딩 시간 계산
/// Bitget은 UTC 00:00, 04:00, 08:00, 12:00, 16:00, 20:00에 펀딩이 발생 (4시간 주기)
fn next_bitget_funding_time(now: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::Timelike;

    // Bitget funding schedule: 00:00, 04:00, 08:00, 12:00, 16:00, 20:00 UTC
    let hours = now.hour();

    // 다음 펀딩 시각의 hour 결정
    let next_hour = if hours < 4 {
        4
    } else if hours < 8 {
        8
    } else if hours < 12 {
        12
    } else if hours < 16 {
        16
    } else if hours < 20 {
        20
    } else {
        24 // 내일 00:00
    };

    let date = now.date_naive();
    let next_date = if next_hour == 24 {
        date.succ_opt().unwrap()
    } else {
        date
    };

    let hour = if next_hour == 24 { 0 } else { next_hour };

    DateTime::<Utc>::from_naive_utc_and_offset(next_date.and_hms_opt(hour, 0, 0).unwrap(), Utc)
}

#[derive(Debug, Deserialize)]
struct BitgetResponse<T> {
    code: String,
    msg: String,
    data: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitgetTicker {
    symbol: String,
    #[serde(default)]
    #[allow(dead_code)]
    last: String,
    #[serde(default)]
    usdt_volume: String, // 24h volume in USDT
    #[serde(default)]
    index_price: String, // mark price
    #[serde(default)]
    funding_rate: String,
    #[serde(default)]
    #[allow(dead_code)]
    holding_amount: String, // net position (can be negative, not OI)
    #[serde(default)]
    #[allow(dead_code)]
    timestamp: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitgetOpenInterest {
    #[allow(dead_code)]
    symbol: String,
    #[serde(default)]
    amount: String, // actual open interest in contracts (always positive)
}

#[async_trait]
impl PerpExchange for BitgetClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bitget
    }

    async fn fetch_all(&self) -> Result<Vec<PerpSnapshot>, ExchangeError> {
        // 1) 티커 정보 (24h 거래량, 마크 가격, 펀딩 레이트)
        let tickers_url = format!("{BASE_URL}/api/mix/v1/market/tickers?productType=umcbl");
        let tickers_response: BitgetResponse<Vec<BitgetTicker>> =
            self.http.get(&tickers_url).send().await?.json().await?;

        if tickers_response.code != "00000" {
            return Err(ExchangeError::Other(format!(
                "Bitget API error (tickers): {} - {}",
                tickers_response.code, tickers_response.msg
            )));
        }

        // 2) 오픈 이너스트 - 각 심볼별로 병렬 조회 (holdingAmount는 net position이므로 실제 OI는 별도 조회 필요)
        let usdt_symbols: Vec<String> = tickers_response
            .data
            .iter()
            .filter(|t| t.symbol.ends_with("_UMCBL"))
            .map(|t| t.symbol.clone())
            .collect();

        // 모든 심볼에 대해 병렬로 open-interest 조회 (동시 요청 수 제한하여 Cloudflare 차단 방지)
        let oi_results: Vec<_> = stream::iter(usdt_symbols.iter().cloned())
            .map(|symbol| {
                let http = self.http.clone();
                let oi_url = format!(
                    "{BASE_URL}/api/mix/v1/market/open-interest?symbol={}&productType=umcbl",
                    symbol
                );

                async move {
                    // 작은 딜레이 추가 (Cloudflare rate limiting 방지)
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                    // HTTP 요청
                    let resp = match http.get(&oi_url).send().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!("Failed to fetch OI for {}: {:?}", symbol, e);
                            return None;
                        }
                    };

                    // 텍스트로 변환
                    let text = match resp.text().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("Failed to get OI text for {}: {:?}", symbol, e);
                            return None;
                        }
                    };

                    // Cloudflare 차단 응답 확인
                    if text.contains("cloudflare") || text.contains("block") {
                        return None;
                    }

                    // JSON 파싱
                    let data: BitgetResponse<BitgetOpenInterest> = match serde_json::from_str(&text)
                    {
                        Ok(d) => d,
                        Err(e) => {
                            // "The symbol has been removed" 메시지는 정상적인 경우이므로 warn 출력하지 않음
                            if !text.contains("The symbol has been removed") {
                                tracing::warn!(
                                    "Failed to parse OI for {}: {:?} - Response: {}",
                                    symbol,
                                    e,
                                    text
                                );
                            }
                            return None;
                        }
                    };

                    // 응답 코드 확인
                    if data.code != "00000" {
                        // "The symbol has been removed" 메시지는 정상적인 경우이므로 warn 출력하지 않음
                        if !data.msg.contains("The symbol has been removed") {
                            tracing::warn!(
                                "Bitget OI API error for {}: {} - {}",
                                symbol,
                                data.code,
                                data.msg
                            );
                        }
                        return None;
                    }

                    Some((symbol, data.data))
                }
            })
            .buffer_unordered(10) // 동시에 최대 10개 요청만 처리 (Cloudflare 차단 방지)
            .collect()
            .await;
        let mut oi_map: HashMap<String, BitgetOpenInterest> = HashMap::new();
        for result in oi_results {
            if let Some((symbol, oi_data)) = result {
                oi_map.insert(symbol, oi_data);
            }
        }

        let now = Utc::now();
        let mut out = Vec::new();

        for ticker in tickers_response.data {
            // Bitget은 "BTCUSDT_UMCBL" 형식이므로 "_UMCBL"로 끝나는 것만 필터링
            if !ticker.symbol.ends_with("_UMCBL") {
                continue;
            }

            // 심볼 변환: "BTCUSDT_UMCBL" -> "BTCUSDT"
            let symbol = ticker.symbol.replace("_UMCBL", "");

            let mark_price: f64 = match ticker.index_price.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let funding_rate: f64 = ticker.funding_rate.parse().unwrap_or(0.0);

            // 오픈 이너스트: open-interest 엔드포인트의 amount (계약 수) * mark_price
            // v1 amount가 음수일 수 있어서 절대값 사용
            let oi_contracts: f64 = match oi_map.get(&ticker.symbol) {
                Some(oi_data) => oi_data.amount.parse::<f64>().unwrap_or(0.0).abs(),
                None => 0.0,
            };
            let oi_usd = oi_contracts * mark_price;

            // 24h 거래량은 usdtVolume (USDT 기준)
            let vol_24h_usd: f64 = ticker.usdt_volume.parse().unwrap_or(0.0);

            // 다음 펀딩 시간 계산 (UTC 00:00, 08:00, 16:00)
            let next_funding_time = Some(next_bitget_funding_time(now));

            out.push(PerpSnapshot {
                exchange: ExchangeId::Bitget,
                symbol,
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
pub struct BitgetSpotClient {
    http: reqwest::Client,
}

impl BitgetSpotClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BitgetSpotTicker {
    symbol: String,
    #[serde(default)]
    close: String, // last price
    #[serde(default)]
    usdt_volume: String, // 24h volume in USDT
}

#[async_trait]
impl SpotExchange for BitgetSpotClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bitget
    }

    async fn fetch_all(&self) -> Result<Vec<SpotSnapshot>, ExchangeError> {
        let tickers_url = format!("{BASE_URL}/api/spot/v1/market/tickers");
        let tickers_response: BitgetResponse<Vec<BitgetSpotTicker>> =
            self.http.get(&tickers_url).send().await?.json().await?;

        if tickers_response.code != "00000" {
            return Err(ExchangeError::Other(format!(
                "Bitget API error (spot tickers): {} - {}",
                tickers_response.code, tickers_response.msg
            )));
        }

        let now = Utc::now();
        let mut out = Vec::new();

        for ticker in tickers_response.data {
            if !ticker.symbol.ends_with("USDT") {
                continue; // USDT 페어만
            }

            let price: f64 = match ticker.close.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // price가 0보다 큰 경우만 추가
            if price <= 0.0 {
                continue;
            }

            let vol_24h_usd: f64 = ticker.usdt_volume.parse().unwrap_or(0.0);

            out.push(SpotSnapshot {
                exchange: ExchangeId::Bitget,
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
    fn test_bitget_client_id() {
        let client = BitgetClient::new();
        assert_eq!(client.id(), ExchangeId::Bitget);
    }

    #[tokio::test]
    async fn test_fetch_all_bitget() {
        let client = BitgetClient::new();
        let result = client.fetch_all().await;

        match result {
            Ok(snapshots) => {
                // API 호출이 성공했는지 확인
                assert!(!snapshots.is_empty(), "snapshots should not be empty");

                // 모든 스냅샷이 Bitget 거래소인지 확인
                for snapshot in &snapshots {
                    println!("Bitget snapshot: {:?}", snapshot);
                    assert_eq!(snapshot.exchange, ExchangeId::Bitget);
                    assert!(snapshot.symbol.ends_with("USDT"));
                    assert!(snapshot.mark_price > 0.0);
                    assert!(snapshot.oi_usd >= 0.0);
                    assert!(snapshot.vol_24h_usd >= 0.0);
                }

                // 심볼이 올바른지 확인 (예: BTCUSDT)
                let btc_snapshot = snapshots.iter().find(|s| s.symbol == "BTCUSDT");
                if btc_snapshot.is_some() {
                    println!("Found BTCUSDT snapshot: {:?}", btc_snapshot);
                }
            }
            Err(e) => {
                // 네트워크 오류 등은 테스트 실패로 간주하지 않음
                // 하지만 API 오류는 확인
                if let ExchangeError::Other(msg) = &e {
                    if msg.contains("Bitget API error") {
                        panic!("Bitget API error: {}", msg);
                    }
                }
                // 네트워크 오류는 테스트 환경에 따라 실패할 수 있으므로 경고만
                eprintln!("Warning: fetch_all failed: {:?}", e);
                panic!();
            }
        }
    }
}
