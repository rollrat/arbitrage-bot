use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::model::{Currency, ExchangeId, PerpSnapshot, SpotSnapshot};

use super::{ExchangeError, PerpExchange, SpotExchange};

const BASE_URL: &str = "https://www.okx.com";
const WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";

#[derive(Debug, Clone)]
struct FundingInfo {
    funding_rate: f64,
    next_funding_time: Option<DateTime<Utc>>,
}

#[derive(Clone)]
pub struct OkxClient {
    http: reqwest::Client,
    funding_cache: Arc<RwLock<HashMap<String, FundingInfo>>>,
}

impl OkxClient {
    pub fn new() -> Self {
        let funding_cache = Arc::new(RwLock::new(HashMap::new()));
        let cache_clone = funding_cache.clone();

        // WebSocket 연결을 백그라운드 태스크로 시작
        tokio::spawn(async move {
            Self::start_websocket(cache_clone).await;
        });

        Self {
            http: reqwest::Client::new(),
            funding_cache,
        }
    }

    async fn start_websocket(cache: Arc<RwLock<HashMap<String, FundingInfo>>>) {
        loop {
            match Self::connect_and_subscribe(cache.clone()).await {
                Ok(_) => {
                    tracing::warn!("OKX WebSocket 연결이 종료되었습니다. 재연결 시도...");
                }
                Err(e) => {
                    tracing::error!("OKX WebSocket 오류: {:?}. 재연결 시도...", e);
                }
            }

            // 재연결 전 대기
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    async fn connect_and_subscribe(
        cache: Arc<RwLock<HashMap<String, FundingInfo>>>,
    ) -> eyre::Result<()> {
        // WebSocket 연결
        let (ws_stream, _) = connect_async(WS_URL).await?;
        let (mut write, mut read) = ws_stream.split();
        tracing::info!("OKX WebSocket 연결 성공");

        // 먼저 모든 USDT-SWAP 심볼 목록 가져오기
        let http = reqwest::Client::new();
        let tickers_url = format!("{BASE_URL}/api/v5/market/tickers?instType=SWAP");
        let response: OkxResponse<Vec<OkxTicker>> =
            http.get(&tickers_url).send().await?.json().await?;

        if response.code != "0" {
            return Err(eyre::eyre!(
                "OKX API error: {} - {}",
                response.code,
                response.msg
            ));
        }

        // USDT-SWAP 심볼만 필터링
        let usdt_swap_symbols: Vec<String> = response
            .data
            .into_iter()
            .filter(|t| t.inst_id.ends_with("-USDT-SWAP"))
            .map(|t| t.inst_id)
            .collect();

        tracing::info!(
            "OKX funding-rate 채널 구독 시작: {}개 심볼",
            usdt_swap_symbols.len()
        );

        // 각 심볼에 대해 funding-rate 채널 구독
        // OKX는 한 번에 최대 20개 심볼까지 구독 가능
        for chunk in usdt_swap_symbols.chunks(20) {
            let args: Vec<serde_json::Value> = chunk
                .iter()
                .map(|inst_id| {
                    json!({
                        "channel": "funding-rate",
                        "instId": inst_id
                    })
                })
                .collect();

            let subscribe_msg = json!({
                "op": "subscribe",
                "args": args
            });

            let msg = Message::Text(serde_json::to_string(&subscribe_msg)?);
            write.send(msg).await?;

            // 구독 메시지 간 약간의 지연
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        tracing::info!("OKX funding-rate 채널 구독 완료");

        // 메시지 수신 루프
        while let Some(msg) = read.next().await {
            match msg? {
                Message::Text(text) => {
                    if let Err(e) = Self::handle_ws_message(&text, cache.clone()).await {
                        tracing::warn!("WebSocket 메시지 처리 오류: {:?}", e);
                    }
                }
                Message::Close(_) => {
                    tracing::warn!("OKX WebSocket 연결이 닫혔습니다");
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_ws_message(
        text: &str,
        cache: Arc<RwLock<HashMap<String, FundingInfo>>>,
    ) -> eyre::Result<()> {
        // OKX WebSocket 응답 파싱
        #[derive(Debug, Deserialize)]
        struct WsResponse {
            #[serde(default)]
            #[allow(dead_code)]
            arg: Option<WsArg>,
            #[serde(default)]
            data: Vec<WsFundingData>,
        }

        #[derive(Debug, Deserialize)]
        struct WsArg {
            #[serde(rename = "instId")]
            #[allow(dead_code)]
            inst_id: String,
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct WsFundingData {
            #[serde(rename = "instId")]
            inst_id: String,
            funding_rate: String,
            next_funding_time: String,
        }

        let response: WsResponse = match serde_json::from_str(text) {
            Ok(r) => r,
            Err(_) => {
                // 구독 확인 메시지 등은 무시
                return Ok(());
            }
        };

        // funding-rate 데이터 처리
        for data in response.data {
            let inst_id = data.inst_id;

            let funding_rate: f64 = data.funding_rate.parse().unwrap_or(0.0);
            let next_funding_time: Option<DateTime<Utc>> = data
                .next_funding_time
                .parse::<i64>()
                .ok()
                .and_then(|ts| DateTime::from_timestamp_millis(ts));

            let funding_info = FundingInfo {
                funding_rate,
                next_funding_time,
            };

            let mut guard = cache.write().await;
            guard.insert(inst_id, funding_info);
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct OkxResponse<T> {
    code: String,
    msg: String,
    data: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxTicker {
    inst_id: String,
    #[serde(default)]
    #[allow(dead_code)]
    last: String,
    #[serde(default)]
    vol_24h: String,
    #[serde(default)]
    vol_ccy_24h: String, // 24h volume in quote currency (USDT)
    #[serde(default)]
    funding_rate: String, // funding rate (tickers 응답에 포함됨)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxMarkPrice {
    inst_id: String,
    #[serde(default)]
    mark_px: String,
    // OKX mark-price API는 instType, instId, markPx, ts만 제공
    // fundingRate와 nextFundingTime은 제공하지 않음
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxOpenInterest {
    inst_id: String,
    #[serde(default)]
    oi: String,
    #[serde(default)]
    oi_ccy: String, // open interest in quote currency (USDT)
}

#[async_trait]
impl PerpExchange for OkxClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Okx
    }

    async fn fetch_all(&self) -> Result<Vec<PerpSnapshot>, ExchangeError> {
        // 1) 티커 정보 (24h 거래량)
        let tickers_url = format!("{BASE_URL}/api/v5/market/tickers?instType=SWAP");
        let tickers_response: OkxResponse<Vec<OkxTicker>> =
            self.http.get(&tickers_url).send().await?.json().await?;

        if tickers_response.code != "0" {
            return Err(ExchangeError::Other(format!(
                "OKX API error (tickers): {} - {}",
                tickers_response.code, tickers_response.msg
            )));
        }

        // 2) 마크 가격
        let mark_price_url = format!("{BASE_URL}/api/v5/public/mark-price?instType=SWAP");
        let mark_price_response: OkxResponse<Vec<OkxMarkPrice>> =
            self.http.get(&mark_price_url).send().await?.json().await?;

        if mark_price_response.code != "0" {
            return Err(ExchangeError::Other(format!(
                "OKX API error (mark-price): {} - {}",
                mark_price_response.code, mark_price_response.msg
            )));
        }

        // 3) 오픈 이너스트
        let oi_url = format!("{BASE_URL}/api/v5/public/open-interest?instType=SWAP");
        let oi_response: OkxResponse<Vec<OkxOpenInterest>> =
            self.http.get(&oi_url).send().await?.json().await?;

        if oi_response.code != "0" {
            return Err(ExchangeError::Other(format!(
                "OKX API error (open-interest): {} - {}",
                oi_response.code, oi_response.msg
            )));
        }

        // 맵으로 변환하여 조회 속도 향상
        let mut ticker_map: HashMap<String, OkxTicker> = HashMap::new();
        for ticker in tickers_response.data {
            if ticker.inst_id.ends_with("-USDT-SWAP") {
                ticker_map.insert(ticker.inst_id.clone(), ticker);
            }
        }

        let mut mark_price_map: HashMap<String, OkxMarkPrice> = HashMap::new();
        for mp in mark_price_response.data {
            if mp.inst_id.ends_with("-USDT-SWAP") {
                mark_price_map.insert(mp.inst_id.clone(), mp);
            }
        }

        let mut oi_map: HashMap<String, OkxOpenInterest> = HashMap::new();
        for oi in oi_response.data {
            if oi.inst_id.ends_with("-USDT-SWAP") {
                oi_map.insert(oi.inst_id.clone(), oi);
            }
        }

        let now = Utc::now();
        let mut out = Vec::new();

        // 모든 USDT-SWAP 심볼에 대해 데이터 조합
        for inst_id in ticker_map.keys() {
            let ticker = match ticker_map.get(inst_id) {
                Some(t) => t,
                None => continue,
            };

            let mark_price_data = match mark_price_map.get(inst_id) {
                Some(mp) => mp,
                None => continue,
            };

            let mark_price: f64 = match mark_price_data.mark_px.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // 펀딩 레이트와 next_funding_time은 WebSocket에서 가져옴
            let funding_cache = self.funding_cache.read().await;
            let funding_info = funding_cache.get(inst_id);

            let funding_rate = funding_info
                .map(|info| info.funding_rate)
                .unwrap_or_else(|| ticker.funding_rate.parse().unwrap_or(0.0));

            let next_funding_time = funding_info.and_then(|info| info.next_funding_time);

            // 오픈 이너스트는 oi_ccy (USDT 기준)를 우선 사용, 없으면 oi * mark_price
            let oi_usd = match oi_map.get(inst_id) {
                Some(oi_data) => {
                    if !oi_data.oi_ccy.is_empty() {
                        oi_data.oi_ccy.parse().unwrap_or(0.0)
                    } else {
                        let oi_contracts: f64 = oi_data.oi.parse().unwrap_or(0.0);
                        oi_contracts * mark_price
                    }
                }
                None => 0.0,
            };

            // 24h 거래량은 volCcy24h (USDT 기준)를 우선 사용, 없으면 vol24h
            let vol_24h_usd: f64 = if !ticker.vol_ccy_24h.is_empty() {
                ticker.vol_ccy_24h.parse().unwrap_or(0.0)
            } else {
                ticker.vol_24h.parse().unwrap_or(0.0)
            };

            // OKX는 "BTC-USDT-SWAP" 형식이므로 "BTCUSDT"로 변환
            let symbol = inst_id.replace("-USDT-SWAP", "USDT").replace("-", "");

            out.push(PerpSnapshot {
                exchange: ExchangeId::Okx,
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
pub struct OkxSpotClient {
    http: reqwest::Client,
}

impl OkxSpotClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OkxSpotTicker {
    inst_id: String,
    #[serde(default)]
    last: String,
    #[serde(default)]
    vol_ccy_24h: String, // 24h volume in quote currency (USDT)
}

#[async_trait]
impl SpotExchange for OkxSpotClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Okx
    }

    async fn fetch_all(&self) -> Result<Vec<SpotSnapshot>, ExchangeError> {
        let tickers_url = format!("{BASE_URL}/api/v5/market/tickers?instType=SPOT");
        let tickers_response: OkxResponse<Vec<OkxSpotTicker>> =
            self.http.get(&tickers_url).send().await?.json().await?;

        if tickers_response.code != "0" {
            return Err(ExchangeError::Other(format!(
                "OKX API error (spot tickers): {} - {}",
                tickers_response.code, tickers_response.msg
            )));
        }

        let now = Utc::now();
        let mut out = Vec::new();

        for ticker in tickers_response.data {
            if !ticker.inst_id.ends_with("-USDT") {
                continue; // USDT 페어만
            }

            let price: f64 = match ticker.last.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            // price가 0보다 큰 경우만 추가
            if price <= 0.0 {
                continue;
            }

            let vol_24h_usd: f64 = ticker.vol_ccy_24h.parse().unwrap_or(0.0);

            // OKX는 "BTC-USDT" 형식이므로 "BTCUSDT"로 변환
            let symbol = ticker.inst_id.replace("-USDT", "USDT").replace("-", "");

            out.push(SpotSnapshot {
                exchange: ExchangeId::Okx,
                symbol,
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
    fn test_okx_client_id() {
        let client = OkxClient::new();
        assert_eq!(client.id(), ExchangeId::Okx);
    }

    #[tokio::test]
    async fn test_fetch_all_okx() {
        let client = OkxClient::new();

        // WebSocket 연결이 완료되고 데이터가 들어올 때까지 대기
        // OKX WebSocket은 연결 후 즉시 데이터를 보내므로 최대 10초 대기
        let mut attempts = 0;
        let max_attempts = 20; // 10초 (0.5초 * 20)

        loop {
            let result = client.fetch_all().await;

            match result {
                Ok(snapshots) => {
                    if snapshots.is_empty() {
                        attempts += 1;
                        if attempts >= max_attempts {
                            panic!("snapshots가 비어있습니다. WebSocket 연결 문제일 수 있습니다.");
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        continue;
                    }

                    // WebSocket에서 funding_rate 데이터가 들어왔는지 확인
                    let has_funding_data = snapshots.iter().any(|s| {
                        // funding_rate가 0이 아니거나 next_funding_time이 Some인 경우
                        s.funding_rate != 0.0 || s.next_funding_time.is_some()
                    });

                    if !has_funding_data && attempts < max_attempts {
                        attempts += 1;
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        continue;
                    }

                    // API 호출이 성공했는지 확인
                    assert!(!snapshots.is_empty(), "snapshots should not be empty");

                    // 모든 스냅샷이 Okx 거래소인지 확인
                    for snapshot in &snapshots {
                        assert_eq!(snapshot.exchange, ExchangeId::Okx);
                        assert!(snapshot.symbol.ends_with("USDT"));
                        assert!(snapshot.mark_price > 0.0);
                        assert!(snapshot.oi_usd >= 0.0);
                        assert!(snapshot.vol_24h_usd >= 0.0);
                    }

                    // WebSocket 데이터 확인
                    let funding_snapshots: Vec<_> = snapshots
                        .iter()
                        .filter(|s| s.funding_rate != 0.0 || s.next_funding_time.is_some())
                        .collect();

                    if !funding_snapshots.is_empty() {
                        println!(
                            "WebSocket에서 {}개의 funding 데이터를 받았습니다.",
                            funding_snapshots.len()
                        );

                        // 예시로 첫 번째 funding 데이터 출력
                        if let Some(sample) = funding_snapshots.first() {
                            println!(
                                "예시: {} - funding_rate: {}, next_funding_time: {:?}",
                                sample.symbol, sample.funding_rate, sample.next_funding_time
                            );
                        }
                    } else {
                        println!("경고: WebSocket에서 funding 데이터를 아직 받지 못했습니다. (연결 중일 수 있음)");
                    }

                    // 심볼 변환이 올바른지 확인 (예: BTC-USDT-SWAP -> BTCUSDT)
                    let btc_snapshot = snapshots.iter().find(|s| s.symbol == "BTCUSDT");
                    if let Some(btc) = btc_snapshot {
                        println!(
                            "Found BTCUSDT snapshot: funding_rate={}, next_funding_time={:?}",
                            btc.funding_rate, btc.next_funding_time
                        );
                    }

                    break;
                }
                Err(e) => {
                    // 네트워크 오류 등은 테스트 실패로 간주하지 않음
                    // 하지만 API 오류는 확인
                    if let ExchangeError::Other(msg) = &e {
                        if msg.contains("OKX API error") {
                            panic!("OKX API error: {}", msg);
                        }
                    }
                    // 네트워크 오류는 테스트 환경에 따라 실패할 수 있으므로 경고만
                    eprintln!("Warning: fetch_all failed: {:?}", e);
                    break;
                }
            }
        }
    }
}
