use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use tracing::info;

use exchanges::{AssetExchange, BinanceClient};
use interface::ExchangeError;

use crate::trader::{FuturesExchangeTrader, SpotExchangeTrader};

const SPOT_BASE_URL: &str = "https://api.binance.com";
const FUTURES_BASE_URL: &str = "https://fapi.binance.com";

#[async_trait]
impl SpotExchangeTrader for BinanceTrader {
    async fn ensure_exchange_info(&self) -> Result<(), ExchangeError> {
        self.load_spot_exchange_info().await
    }

    async fn get_spot_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        self.get_spot_price(symbol).await
    }

    fn clamp_spot_quantity(&self, symbol: &str, qty: f64) -> f64 {
        self.clamp_spot_quantity(symbol, qty)
    }

    async fn buy_spot(&self, symbol: &str, qty: f64) -> Result<OrderResponse, ExchangeError> {
        self.place_spot_order(symbol, "BUY", qty, false).await
    }

    async fn sell_spot(&self, symbol: &str, qty: f64) -> Result<OrderResponse, ExchangeError> {
        self.place_spot_order(symbol, "SELL", qty, false).await
    }

    async fn get_spot_balance(&self, asset: &str) -> Result<f64, ExchangeError> {
        self.get_spot_balance(asset).await
    }
}

#[async_trait]
impl FuturesExchangeTrader for BinanceTrader {
    async fn ensure_exchange_info(&self) -> Result<(), ExchangeError> {
        self.load_futures_exchange_info().await
    }

    async fn ensure_account_setup(
        &self,
        symbol: &str,
        leverage: u32,
        isolated: bool,
    ) -> Result<(), ExchangeError> {
        self.ensure_futures_setup(symbol, leverage, isolated).await
    }

    async fn get_mark_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        self.get_futures_mark_price(symbol).await
    }

    fn clamp_futures_quantity(&self, symbol: &str, qty: f64) -> f64 {
        self.clamp_futures_quantity(symbol, qty)
    }

    async fn buy_futures(
        &self,
        symbol: &str,
        qty: f64,
        reduce_only: bool,
    ) -> Result<OrderResponse, ExchangeError> {
        self.place_futures_order(symbol, "BUY", qty, reduce_only)
            .await
    }

    async fn sell_futures(
        &self,
        symbol: &str,
        qty: f64,
        reduce_only: bool,
    ) -> Result<OrderResponse, ExchangeError> {
        self.place_futures_order(symbol, "SELL", qty, reduce_only)
            .await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub symbol: String,
    pub order_id: Option<u64>,
    pub client_order_id: Option<String>,
    pub executed_qty: Option<String>,
    pub status: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Binance LOT_SIZE 필터 정보
#[derive(Debug, Clone, Copy)]
pub struct LotSizeFilter {
    pub min_qty: f64,
    pub max_qty: f64,
    pub step_size: f64,
}

pub struct BinanceTrader {
    pub spot_client: BinanceClient,
    pub futures_client: BinanceClient,
    /// 스팟 심볼별 LOT_SIZE 필터 캐시
    spot_lot_size_cache: RwLock<HashMap<String, LotSizeFilter>>,
    /// 선물 심볼별 LOT_SIZE 필터 캐시
    futures_lot_size_cache: RwLock<HashMap<String, LotSizeFilter>>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct HedgedPair {
    /// 스팟 주문에 실제로 넣을 수량 (LOT_SIZE 만족)
    pub spot_order_qty: f64,
    /// 선물 주문에 실제로 넣을 수량 (LOT_SIZE 만족)
    pub fut_order_qty: f64,
    /// 수수료 반영 후 예상 스팟 순수량
    pub spot_net_qty_est: f64,
    /// 예상 잔여 델타 (spot_net - fut)
    pub delta_est: f64,
}

impl BinanceTrader {
    pub fn new() -> Result<Self, ExchangeError> {
        let spot_client = BinanceClient::with_credentials()
            .map_err(|e| ExchangeError::Other(format!("Failed to create spot client: {}", e)))?;
        let futures_client = BinanceClient::with_credentials()
            .map_err(|e| ExchangeError::Other(format!("Failed to create futures client: {}", e)))?;

        Ok(Self {
            spot_client,
            futures_client,
            spot_lot_size_cache: RwLock::new(HashMap::new()),
            futures_lot_size_cache: RwLock::new(HashMap::new()),
        })
    }

    /// 스팟 exchangeInfo를 로드하여 LOT_SIZE 필터를 캐시에 저장
    pub async fn load_spot_exchange_info(&self) -> Result<(), ExchangeError> {
        let url = format!("{}/api/v3/exchangeInfo", SPOT_BASE_URL);

        let response = self
            .spot_client
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Spot exchangeInfo API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let resp: serde_json::Value = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse exchangeInfo: {}", e)))?;

        let mut cache = self.spot_lot_size_cache.write().unwrap();
        cache.clear();

        if let Some(symbols) = resp.get("symbols").and_then(|v| v.as_array()) {
            for symbol_info in symbols {
                let symbol = match symbol_info.get("symbol").and_then(|v| v.as_str()) {
                    Some(sym) => sym.to_string(),
                    None => continue,
                };

                if let Some(filters) = symbol_info.get("filters").and_then(|v| v.as_array()) {
                    for filter in filters {
                        let filter_type = filter.get("filterType").and_then(|v| v.as_str());
                        if filter_type == Some("LOT_SIZE") {
                            let min_qty = filter
                                .get("minQty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);

                            let max_qty = filter
                                .get("maxQty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(f64::MAX);

                            let step_size = filter
                                .get("stepSize")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(1.0);

                            cache.insert(
                                symbol.clone(),
                                LotSizeFilter {
                                    min_qty,
                                    max_qty,
                                    step_size,
                                },
                            );
                            break;
                        }
                    }
                }
            }
        }

        tracing::info!("Loaded {} spot symbols LOT_SIZE filters", cache.len());
        Ok(())
    }

    /// 선물 exchangeInfo를 로드하여 LOT_SIZE 필터를 캐시에 저장
    pub async fn load_futures_exchange_info(&self) -> Result<(), ExchangeError> {
        let url = format!("{}/fapi/v1/exchangeInfo", FUTURES_BASE_URL);

        let response = self
            .futures_client
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Futures exchangeInfo API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let resp: serde_json::Value = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse exchangeInfo: {}", e)))?;

        let mut cache = self.futures_lot_size_cache.write().unwrap();
        cache.clear();

        if let Some(symbols) = resp.get("symbols").and_then(|v| v.as_array()) {
            for symbol_info in symbols {
                let symbol = match symbol_info.get("symbol").and_then(|v| v.as_str()) {
                    Some(sym) => sym.to_string(),
                    None => continue,
                };

                if let Some(filters) = symbol_info.get("filters").and_then(|v| v.as_array()) {
                    for filter in filters {
                        let filter_type = filter.get("filterType").and_then(|v| v.as_str());
                        if filter_type == Some("LOT_SIZE") {
                            let min_qty = filter
                                .get("minQty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(0.0);

                            let max_qty = filter
                                .get("maxQty")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(f64::MAX);

                            let step_size = filter
                                .get("stepSize")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<f64>().ok())
                                .unwrap_or(1.0);

                            cache.insert(
                                symbol.clone(),
                                LotSizeFilter {
                                    min_qty,
                                    max_qty,
                                    step_size,
                                },
                            );
                            break;
                        }
                    }
                }
            }
        }

        tracing::info!("Loaded {} futures symbols LOT_SIZE filters", cache.len());
        Ok(())
    }

    /// 스팟 심볼의 LOT_SIZE 필터 가져오기
    fn get_spot_lot_size(&self, symbol: &str) -> Option<LotSizeFilter> {
        self.spot_lot_size_cache
            .read()
            .unwrap()
            .get(symbol)
            .copied()
    }

    /// 선물 심볼의 LOT_SIZE 필터 가져오기
    fn get_futures_lot_size(&self, symbol: &str) -> Option<LotSizeFilter> {
        self.futures_lot_size_cache
            .read()
            .unwrap()
            .get(symbol)
            .copied()
    }

    /// LOT_SIZE 필터를 사용하여 수량을 clamp하는 헬퍼 함수
    // fn clamp_quantity_with_filter(filter: LotSizeFilter, qty: f64) -> f64 {
    //     if qty <= 0.0 {
    //         return 0.0;
    //     }

    //     let step = filter.step_size;
    //     if step <= 0.0 {
    //         return qty;
    //     }

    //     // step 단위로 내림
    //     let steps = (qty / step).floor();
    //     let clamped = steps * step;

    //     if clamped < filter.min_qty {
    //         0.0
    //     } else if clamped > filter.max_qty {
    //         filter.max_qty
    //     } else {
    //         clamped
    //     }
    // }

    fn clamp_quantity_with_filter(filter: LotSizeFilter, qty: f64) -> f64 {
        const BASE_PRECISION: u32 = 8;

        if qty <= 0.0 {
            return 0.0;
        }

        // 1) precision 잘라내기 (floor)
        let pow = 10f64.powi(BASE_PRECISION as i32);
        let mut qty = (qty * pow).floor() / pow;

        // 2) stepSize 처리
        if filter.step_size > 0.0 {
            let steps = (qty / filter.step_size).floor();
            qty = steps * filter.step_size;
        }

        // 3) minQty 미만이면 invalid → 0이 아니라 "그냥 에러"로 처리해야 맞음
        if qty < filter.min_qty {
            return 0.0; // ← but ideally, return Err(...)
        }

        // 4) maxQty clamp
        if qty > filter.max_qty {
            qty = filter.max_qty;
        }

        qty
    }

    /// 스팟 잔고 조회
    pub async fn get_spot_balance(&self, asset: &str) -> Result<f64, ExchangeError> {
        let assets = self
            .spot_client
            .fetch_assets()
            .await
            .map_err(|e| ExchangeError::Other(format!("Failed to fetch spot assets: {}", e)))?;

        let balance = assets
            .iter()
            .find(|a| a.currency == asset)
            .map(|a| a.available)
            .unwrap_or(0.0);

        Ok(balance)
    }

    /// 선물 잔고 조회 (USDT 마진)
    pub async fn get_futures_balance(&self) -> Result<f64, ExchangeError> {
        let api_key = self
            .futures_client
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API key not set".to_string()))?;
        let api_secret = self
            .futures_client
            .api_secret
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API secret not set".to_string()))?;

        let endpoint = "/fapi/v2/balance";
        let timestamp = exchanges::binance::get_timestamp();
        let query_string = format!("timestamp={}", timestamp);
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .futures_client
            .http
            .get(&url)
            .header("X-MBX-APIKEY", api_key.as_str())
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Futures balance API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct FuturesBalance {
            asset: String,
            balance: String,
        }

        let balances: Vec<FuturesBalance> = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse balance: {}", e)))?;

        let usdt_balance = balances
            .iter()
            .find(|b| b.asset == "USDT")
            .and_then(|b| b.balance.parse::<f64>().ok())
            .unwrap_or(0.0);

        Ok(usdt_balance)
    }

    /// 심볼에서 베이스 자산 추출 (예: "BTCUSDT" -> "BTC")
    pub fn base_asset_from_symbol(symbol: &str) -> String {
        if symbol.ends_with("USDT") {
            symbol[..symbol.len() - 4].to_string()
        } else if symbol.ends_with("USD") {
            symbol[..symbol.len() - 3].to_string()
        } else {
            symbol.to_string()
        }
    }

    /// 스팟 수량을 거래소 규칙에 맞게 조정 (LOT_SIZE)
    /// exchangeInfo에서 가져온 실제 LOT_SIZE 필터를 사용
    pub fn clamp_spot_quantity(&self, symbol: &str, qty: f64) -> f64 {
        if let Some(filter) = self.get_spot_lot_size(symbol) {
            Self::clamp_quantity_with_filter(filter, qty)
        } else {
            // LOT_SIZE 정보를 못 찾으면 원래 qty를 반환
            // (상위에서 에러 처리하거나, exchangeInfo를 다시 로드해야 함)
            tracing::warn!(
                "LOT_SIZE filter not found for spot symbol: {}. Using original quantity.",
                symbol
            );
            qty
        }
    }

    /// 선물 수량을 거래소 규칙에 맞게 조정 (LOT_SIZE)
    /// exchangeInfo에서 가져온 실제 LOT_SIZE 필터를 사용
    pub fn clamp_futures_quantity(&self, symbol: &str, qty: f64) -> f64 {
        if let Some(filter) = self.get_futures_lot_size(symbol) {
            Self::clamp_quantity_with_filter(filter, qty)
        } else {
            // LOT_SIZE 정보를 못 찾으면 원래 qty를 반환
            tracing::warn!(
                "LOT_SIZE filter not found for futures symbol: {}. Using original quantity.",
                symbol
            );
            qty
        }
    }

    /// target_net_qty 근처에서 스팟/선물 둘 다 LOT_SIZE를 만족하는 쌍을 찾는다.
    /// spot_fee_rate: 스팟 수수료율 (maker 또는 taker 중 선택)
    pub fn find_hedged_pair(
        &self,
        symbol: &str,
        target_net_qty: f64,
        spot_fee_rate: f64,
    ) -> Option<HedgedPair> {
        if target_net_qty <= 0.0 {
            return None;
        }

        // 선물 LOT_SIZE filter에서 stepSize를 가져와서 "한 스텝씩 줄여가며 탐색"에 사용
        let fut_lot = self.get_futures_lot_size(symbol)?;
        let fut_step = if fut_lot.step_size > 0.0 {
            fut_lot.step_size
        } else {
            // stepSize가 0이면 격자 정보가 없으니 그냥 한 번만 시도
            0.0
        };

        // 1) 먼저 target_net_qty를 기준으로 "선물 수량 후보"를 만든다.
        //    (선물 LOT_SIZE에 맞게 클램프)
        let mut fut_candidate = self.clamp_futures_quantity(symbol, target_net_qty);
        if fut_candidate <= 0.0 {
            return None;
        }

        // 허용 오차: 스팟/선물 스텝 중 더 작은 값의 절반 정도
        let spot_step = self
            .get_spot_lot_size(symbol)
            .map(|f| f.step_size)
            .unwrap_or(fut_step.max(1e-8)); // 그래도 0은 피하기

        let tol = spot_step.min(fut_step.max(spot_step)).abs() * 0.5;

        // 2) fut_candidate를 기준으로, 이에 맞는 스팟 주문 수량을 찾는다.
        //    안 맞으면 선물 수량을 한 step씩 줄여가며 재시도.
        let max_iters = 50;
        for _ in 0..max_iters {
            // 이 선물 수량을 "정확히" 덮고 싶다면, 스팟 순수량 == fut_candidate 여야 함.
            // spot_net = spot_order * (1 - fee) ⇒ spot_order = fut_candidate / (1 - fee)
            let ideal_spot_order = fut_candidate / (1.0 - spot_fee_rate);

            if !ideal_spot_order.is_finite() || ideal_spot_order <= 0.0 {
                break;
            }

            // 스팟 LOT_SIZE에 맞게 주문 수량 클램프
            let spot_order_qty = self.clamp_spot_quantity(symbol, ideal_spot_order);
            if spot_order_qty <= 0.0 {
                break;
            }

            // 클램프 후 "예상 스팟 순수량"
            let spot_net_qty_est = spot_order_qty * (1.0 - spot_fee_rate);

            // 이 조합에서의 예상 델타
            let delta = spot_net_qty_est - fut_candidate;

            // 델타가 허용 오차 내면 이 쌍을 채택
            if delta.abs() <= tol {
                return Some(HedgedPair {
                    spot_order_qty,
                    fut_order_qty: fut_candidate,
                    spot_net_qty_est,
                    delta_est: delta,
                });
            }

            // 더 안 맞으면 선물 수량을 한 step 줄여서 다시 시도
            if fut_step <= 0.0 {
                // step 정보가 없으면 더 이상 줄일 수 없음
                break;
            }

            let next_fut = fut_candidate - fut_step;
            let next_fut = self.clamp_futures_quantity(symbol, next_fut);
            if next_fut <= 0.0 || (next_fut - fut_candidate).abs() < 1e-12 {
                break;
            }
            fut_candidate = next_fut;
        }

        None
    }

    /// 레거시 호환성을 위한 정적 메서드 (deprecated)
    /// 실제로는 clamp_spot_quantity 또는 clamp_futures_quantity를 사용해야 함
    #[deprecated(note = "Use clamp_spot_quantity or clamp_futures_quantity instead")]
    pub fn clamp_quantity(_symbol: &str, qty: f64) -> f64 {
        // 하위 호환성을 위해 간단한 구현 유지
        // 실제 사용 시에는 인스턴스 메서드를 사용해야 함
        let step = 0.001;
        (qty / step).floor() * step
    }

    /// 스팟 시장가 주문
    pub async fn place_spot_order(
        &self,
        symbol: &str,
        side: &str, // "BUY" or "SELL"
        quantity: f64,
        test: bool,
    ) -> Result<OrderResponse, ExchangeError> {
        let api_key = self
            .spot_client
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API key not set".to_string()))?;
        let api_secret = self
            .spot_client
            .api_secret
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API secret not set".to_string()))?;

        let endpoint = if test {
            "/api/v3/order/test"
        } else {
            "/api/v3/order"
        };

        let timestamp = exchanges::binance::get_timestamp();
        let qty_str = format!("{:.8}", quantity);
        let query_string = format!(
            "symbol={}&side={}&type=MARKET&quantity={}&timestamp={}",
            symbol, side, qty_str, timestamp
        );
        info!("place_spot_order query_string: {}", query_string);
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            SPOT_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .spot_client
            .http
            .post(&url)
            .header("X-MBX-APIKEY", api_key.as_str())
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Spot order API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let order: OrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse order response: {}", e)))?;

        Ok(order)
    }

    /// 선물 시장가 주문
    pub async fn place_futures_order(
        &self,
        symbol: &str,
        side: &str, // "BUY" or "SELL"
        quantity: f64,
        reduce_only: bool,
    ) -> Result<OrderResponse, ExchangeError> {
        let api_key = self
            .futures_client
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API key not set".to_string()))?;
        let api_secret = self
            .futures_client
            .api_secret
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API secret not set".to_string()))?;

        let endpoint = "/fapi/v1/order";

        let timestamp = exchanges::binance::get_timestamp();
        let qty_str = format!("{:.8}", quantity);
        let mut query_string = format!(
            "symbol={}&side={}&type=MARKET&quantity={}&timestamp={}",
            symbol, side, qty_str, timestamp
        );

        info!("place_futures_order query_string: {}", query_string);

        if reduce_only {
            query_string.push_str("&reduceOnly=true");
        }

        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .futures_client
            .http
            .post(&url)
            .header("X-MBX-APIKEY", api_key.as_str())
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let response_text = response.text().await?;

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Futures order API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let order: OrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse order response: {}", e)))?;

        Ok(order)
    }

    /// 선물 마진 타입 및 레버리지 설정
    pub async fn ensure_futures_setup(
        &self,
        symbol: &str,
        leverage: u32,
        isolated: bool,
    ) -> Result<(), ExchangeError> {
        let api_key = self
            .futures_client
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API key not set".to_string()))?;
        let api_secret = self
            .futures_client
            .api_secret
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API secret not set".to_string()))?;

        // 1. 마진 타입 설정
        let endpoint = "/fapi/v1/marginType";
        let timestamp = exchanges::binance::get_timestamp();
        let margin_type = if isolated { "ISOLATED" } else { "CROSS" };
        let query_string = format!(
            "symbol={}&marginType={}&timestamp={}",
            symbol, margin_type, timestamp
        );
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .futures_client
            .http
            .post(&url)
            .header("X-MBX-APIKEY", api_key.as_str())
            .send()
            .await;

        // 마진 타입이 이미 설정되어 있으면 에러가 날 수 있음 (무시)
        if let Ok(resp) = response {
            if !resp.status().is_success() {
                let text = resp.text().await.unwrap_or_default();
                if !text.contains("-4046") {
                    // -4046은 "No need to change margin type" 에러
                    tracing::warn!("Failed to set margin type: {}", text);
                }
            }
        }

        // 2. 레버리지 설정
        let endpoint = "/fapi/v1/leverage";
        let timestamp = exchanges::binance::get_timestamp();
        let query_string = format!(
            "symbol={}&leverage={}&timestamp={}",
            symbol, leverage, timestamp
        );
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .futures_client
            .http
            .post(&url)
            .header("X-MBX-APIKEY", api_key.as_str())
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let response_text = response.text().await.unwrap_or_default();
            tracing::warn!(
                "Failed to set leverage: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            );
        }

        Ok(())
    }

    /// 스팟 현재가 조회
    pub async fn get_spot_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        let url = format!("{}/api/v3/ticker/price?symbol={}", SPOT_BASE_URL, symbol);

        #[derive(Debug, Deserialize)]
        struct PriceResponse {
            price: String,
        }

        let response: PriceResponse = self
            .spot_client
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?
            .json()
            .await
            .map_err(|e| ExchangeError::Other(format!("Failed to parse price: {}", e)))?;

        response
            .price
            .parse::<f64>()
            .map_err(|e| ExchangeError::Other(format!("Failed to parse price as f64: {}", e)))
    }

    /// 선물 마크 가격 조회
    pub async fn get_futures_mark_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        let url = format!(
            "{}/fapi/v1/premiumIndex?symbol={}",
            FUTURES_BASE_URL, symbol
        );

        #[derive(Debug, Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct MarkPriceResponse {
            mark_price: String,
        }

        let response: MarkPriceResponse = self
            .futures_client
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?
            .json()
            .await
            .map_err(|e| ExchangeError::Other(format!("Failed to parse mark price: {}", e)))?;

        response
            .mark_price
            .parse::<f64>()
            .map_err(|e| ExchangeError::Other(format!("Failed to parse mark price as f64: {}", e)))
    }
}
