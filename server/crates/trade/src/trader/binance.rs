use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as TokioRwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use exchanges::binance::{generate_signature, get_timestamp};
use exchanges::{AssetExchange, BinanceClient};
use interface::ExchangeError;

use crate::trader::{FuturesExchangeTrader, SpotExchangeTrader};

const SPOT_BASE_URL: &str = "https://api.binance.com";
const FUTURES_BASE_URL: &str = "https://fapi.binance.com";
const SPOT_WS_URL: &str = "wss://stream.binance.com:9443/ws";
const FUTURES_WS_URL: &str = "wss://fstream.binance.com/ws";
const WS_API_URL: &str = "wss://ws-api.binance.com/ws-api/v3";

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
        self.order_client
            .place_spot_order(symbol, "BUY", qty, None, PlaceOrderOptions { test: false })
            .await
    }

    async fn sell_spot(&self, symbol: &str, qty: f64) -> Result<OrderResponse, ExchangeError> {
        self.order_client
            .place_spot_order(symbol, "SELL", qty, None, PlaceOrderOptions { test: false })
            .await
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
        self.futures.ensure_setup(symbol, leverage, isolated).await
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
        self.order_client
            .place_futures_order(
                symbol,
                "BUY",
                qty,
                None,
                PlaceFuturesOrderOptions { reduce_only },
            )
            .await
    }

    async fn sell_futures(
        &self,
        symbol: &str,
        qty: f64,
        reduce_only: bool,
    ) -> Result<OrderResponse, ExchangeError> {
        self.order_client
            .place_futures_order(
                symbol,
                "SELL",
                qty,
                None,
                PlaceFuturesOrderOptions { reduce_only },
            )
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

/// 주문 옵션 (Spot 주문용)
#[derive(Debug, Clone, Default)]
pub struct PlaceOrderOptions {
    pub test: bool,
}

/// 주문 옵션 (Futures 주문용)
#[derive(Debug, Clone, Default)]
pub struct PlaceFuturesOrderOptions {
    pub reduce_only: bool,
}

/// BinanceTrader가 의존하는 주문 클라이언트 트레이트. 나중에 WebSocket 기반 구현체를 추가할 수 있다.
#[async_trait]
pub trait BinanceOrderClient: Send + Sync {
    async fn place_spot_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        price: Option<f64>,
        options: PlaceOrderOptions,
    ) -> Result<OrderResponse, ExchangeError>;

    async fn place_futures_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        price: Option<f64>,
        options: PlaceFuturesOrderOptions,
    ) -> Result<OrderResponse, ExchangeError>;

    async fn cancel_spot_order(&self, symbol: &str, order_id: &str) -> Result<(), ExchangeError>;

    async fn cancel_futures_order(&self, symbol: &str, order_id: &str)
    -> Result<(), ExchangeError>;
}

/// HTTP 기반으로 Binance Spot/Futures 주문을 보내는 구현체
pub struct HttpBinanceOrderClient {
    spot_client: BinanceClient,
    futures_client: BinanceClient,
}

impl HttpBinanceOrderClient {
    pub fn new(spot_client: BinanceClient, futures_client: BinanceClient) -> Self {
        Self {
            spot_client,
            futures_client,
        }
    }
}

#[async_trait]
impl BinanceOrderClient for HttpBinanceOrderClient {
    async fn place_spot_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        _price: Option<f64>,
        options: PlaceOrderOptions,
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

        let endpoint = if options.test {
            "/api/v3/order/test"
        } else {
            "/api/v3/order"
        };

        let timestamp = exchanges::binance::get_timestamp();
        let qty_str = format!("{:.8}", qty);
        let query_string = format!(
            "symbol={}&side={}&type=MARKET&quantity={}&timestamp={}&recvWindow=50000",
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

        info!("place_spot_order response: {}", response_text);

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Spot order API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let order: OrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse order response: {}", e)))?;

        // 거래 기록 저장 (test 모드가 아닐 때만)
        if !options.test {
            crate::record::save_trade_record_spot_order(
                "binance",
                symbol,
                side,
                qty,
                &query_string,
                &order,
                false,
            )
            .await;
        }

        Ok(order)
    }

    async fn place_futures_order(
        &self,
        symbol: &str,
        side: &str,
        qty: f64,
        _price: Option<f64>,
        options: PlaceFuturesOrderOptions,
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
        let qty_str = format!("{:.8}", qty);
        let mut query_string = format!(
            "symbol={}&side={}&type=MARKET&quantity={}&timestamp={}&recvWindow=50000",
            symbol, side, qty_str, timestamp
        );

        info!("place_futures_order query_string: {}", query_string);

        if options.reduce_only {
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

        info!("place_futures_order response: {}", response_text);

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Futures order API error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let order: OrderResponse = serde_json::from_str(&response_text)
            .map_err(|e| ExchangeError::Other(format!("Failed to parse order response: {}", e)))?;

        // 거래 기록 저장
        crate::record::save_trade_record_futures_order(
            "binance",
            symbol,
            side,
            qty,
            &query_string,
            &order,
            options.reduce_only,
            false, // is_liquidation: reduce_only는 정상 포지션 청산이지 강제 청산이 아님
        )
        .await;

        Ok(order)
    }

    async fn cancel_spot_order(&self, _symbol: &str, _order_id: &str) -> Result<(), ExchangeError> {
        // TODO: 구현 필요
        Err(ExchangeError::Other("Not implemented".to_string()))
    }

    async fn cancel_futures_order(
        &self,
        _symbol: &str,
        _order_id: &str,
    ) -> Result<(), ExchangeError> {
        // TODO: 구현 필요
        Err(ExchangeError::Other("Not implemented".to_string()))
    }
}

/// Binance LOT_SIZE 필터 정보
#[derive(Debug, Clone, Copy)]
pub struct LotSizeFilter {
    pub min_qty: f64,
    pub max_qty: f64,
    pub step_size: f64,
}

/// 실시간 가격 상태 (WebSocket에서 업데이트)
#[derive(Debug, Clone, Default)]
struct PriceState {
    spot_price: Option<f64>,
    futures_mark_price: Option<f64>,
    last_updated: Option<std::time::SystemTime>,
}

/// Binance Spot API: Spot 주문, exchangeInfo, LOT_SIZE 캐시 관리
pub struct BinanceSpotApi {
    client: BinanceClient,
    lot_size_cache: RwLock<HashMap<String, LotSizeFilter>>,
}

/// Binance Futures API: Futures 주문, exchangeInfo, LOT_SIZE 캐시 관리
pub struct BinanceFuturesApi {
    client: BinanceClient,
    lot_size_cache: RwLock<HashMap<String, LotSizeFilter>>,
}

/// Binance Price Feed: WebSocket 가격 스트림 관리
pub struct BinancePriceFeed {
    price_state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    spot_client: BinanceClient,
    futures_client: BinanceClient,
}

/// Binance User Stream: User Data Stream WebSocket 관리
pub struct BinanceUserStream {
    spot_client: BinanceClient,
}

pub struct BinanceTrader {
    order_client: Arc<dyn BinanceOrderClient>,
    spot: Arc<BinanceSpotApi>,
    futures: Arc<BinanceFuturesApi>,
    price_feed: Arc<BinancePriceFeed>,
    user_stream: Option<Arc<BinanceUserStream>>,
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

impl BinanceSpotApi {
    pub fn new(client: BinanceClient) -> Self {
        Self {
            client,
            lot_size_cache: RwLock::new(HashMap::new()),
        }
    }

    /// 스팟 exchangeInfo를 로드하여 LOT_SIZE 필터를 캐시에 저장
    pub async fn load_exchange_info(&self) -> Result<(), ExchangeError> {
        let url = format!("{}/api/v3/exchangeInfo", SPOT_BASE_URL);

        let response = self
            .client
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

        let mut cache = self.lot_size_cache.write().unwrap();
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

    /// 스팟 심볼의 LOT_SIZE 필터 가져오기
    pub fn get_lot_size(&self, symbol: &str) -> Option<LotSizeFilter> {
        self.lot_size_cache.read().unwrap().get(symbol).copied()
    }

    /// 스팟 수량을 거래소 규칙에 맞게 조정 (LOT_SIZE)
    pub fn clamp_quantity(&self, symbol: &str, qty: f64) -> f64 {
        if let Some(filter) = self.get_lot_size(symbol) {
            BinanceTrader::clamp_quantity_with_filter(filter, qty)
        } else {
            tracing::warn!(
                "LOT_SIZE filter not found for spot symbol: {}. Using original quantity.",
                symbol
            );
            qty
        }
    }

    /// 스팟 잔고 조회
    pub async fn get_balance(&self, asset: &str) -> Result<f64, ExchangeError> {
        let assets = self
            .client
            .fetch_spots()
            .await
            .map_err(|e| ExchangeError::Other(format!("Failed to fetch spot assets: {}", e)))?;

        let balance = assets
            .iter()
            .find(|a| a.currency == asset)
            .map(|a| a.available)
            .unwrap_or(0.0);

        Ok(balance)
    }

    pub fn client(&self) -> &BinanceClient {
        &self.client
    }
}

impl BinanceFuturesApi {
    pub fn new(client: BinanceClient) -> Self {
        Self {
            client,
            lot_size_cache: RwLock::new(HashMap::new()),
        }
    }

    /// 선물 exchangeInfo를 로드하여 LOT_SIZE 필터를 캐시에 저장
    pub async fn load_exchange_info(&self) -> Result<(), ExchangeError> {
        let url = format!("{}/fapi/v1/exchangeInfo", FUTURES_BASE_URL);

        let response = self
            .client
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

        let mut cache = self.lot_size_cache.write().unwrap();
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

    /// 선물 심볼의 LOT_SIZE 필터 가져오기
    pub fn get_lot_size(&self, symbol: &str) -> Option<LotSizeFilter> {
        self.lot_size_cache.read().unwrap().get(symbol).copied()
    }

    /// 선물 수량을 거래소 규칙에 맞게 조정 (LOT_SIZE)
    pub fn clamp_quantity(&self, symbol: &str, qty: f64) -> f64 {
        if let Some(filter) = self.get_lot_size(symbol) {
            BinanceTrader::clamp_quantity_with_filter(filter, qty)
        } else {
            tracing::warn!(
                "LOT_SIZE filter not found for futures symbol: {}. Using original quantity.",
                symbol
            );
            qty
        }
    }

    /// 선물 마진 타입 및 레버리지 설정
    pub async fn ensure_setup(
        &self,
        symbol: &str,
        leverage: u32,
        isolated: bool,
    ) -> Result<(), ExchangeError> {
        let api_key = self
            .client
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API key not set".to_string()))?;
        let api_secret = self
            .client
            .api_secret
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API secret not set".to_string()))?;

        // 1. 마진 타입 설정
        let endpoint = "/fapi/v1/marginType";
        let timestamp = exchanges::binance::get_timestamp();
        let margin_type = if isolated { "ISOLATED" } else { "CROSS" };
        let query_string = format!(
            "symbol={}&marginType={}&timestamp={}&recvWindow=50000",
            symbol, margin_type, timestamp
        );
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .client
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
            "symbol={}&leverage={}&timestamp={}&recvWindow=50000",
            symbol, leverage, timestamp
        );
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .client
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

    /// 선물 잔고 조회 (USDT 마진)
    pub async fn get_balance(&self) -> Result<f64, ExchangeError> {
        let api_key = self
            .client
            .api_key
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API key not set".to_string()))?;
        let api_secret = self
            .client
            .api_secret
            .as_ref()
            .ok_or_else(|| ExchangeError::Other("API secret not set".to_string()))?;

        let endpoint = "/fapi/v2/balance";
        let timestamp = exchanges::binance::get_timestamp();
        let query_string = format!("timestamp={}&recvWindow=50000", timestamp);
        let signature = exchanges::binance::generate_signature(&query_string, api_secret);

        let url = format!(
            "{}{}?{}&signature={}",
            FUTURES_BASE_URL, endpoint, query_string, signature
        );

        let response = self
            .client
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

    pub fn client(&self) -> &BinanceClient {
        &self.client
    }
}

impl BinancePriceFeed {
    pub fn new(spot_client: BinanceClient, futures_client: BinanceClient) -> Self {
        Self {
            price_state: Arc::new(TokioRwLock::new(HashMap::new())),
            spot_client,
            futures_client,
        }
    }

    /// 특정 심볼에 대한 WebSocket 리스너 시작
    /// 스팟 ticker와 선물 markPrice를 동시에 구독
    pub fn start_symbol(&self, symbol: &str) {
        let price_state = Arc::clone(&self.price_state);

        // 스팟 ticker WebSocket
        let spot_state = Arc::clone(&price_state);
        let spot_symbol = symbol.to_string();
        tokio::spawn(async move {
            Self::start_spot_websocket(&spot_symbol, spot_state).await;
        });

        // 선물 markPrice WebSocket
        let fut_state = Arc::clone(&price_state);
        let fut_symbol = symbol.to_string();
        tokio::spawn(async move {
            Self::start_futures_websocket(&fut_symbol, fut_state).await;
        });

        info!("WebSocket 리스너 시작: {}", symbol);
    }

    /// 스팟 현재가 조회 (메모리에서 읽기, 없으면 HTTP 폴백)
    pub async fn get_spot_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        // 먼저 메모리에서 읽기 시도
        {
            let state_map = self.price_state.read().await;
            if let Some(price_state) = state_map.get(symbol) {
                if let Some(price) = price_state.spot_price {
                    return Ok(price);
                }
            }
        }

        // 메모리에 없으면 HTTP 폴백
        warn!(
            "WebSocket에서 스팟 가격을 찾을 수 없어 HTTP로 조회합니다 (symbol: {})",
            symbol
        );
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

        let price = response
            .price
            .parse::<f64>()
            .map_err(|e| ExchangeError::Other(format!("Failed to parse price as f64: {}", e)))?;

        // HTTP로 가져온 가격도 메모리에 저장
        {
            let mut state_map = self.price_state.write().await;
            let price_state = state_map
                .entry(symbol.to_string())
                .or_insert_with(PriceState::default);
            price_state.spot_price = Some(price);
            price_state.last_updated = Some(std::time::SystemTime::now());
        }

        Ok(price)
    }

    /// 선물 마크 가격 조회 (메모리에서 읽기, 없으면 HTTP 폴백)
    pub async fn get_futures_mark_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        // 먼저 메모리에서 읽기 시도
        {
            let state_map = self.price_state.read().await;
            if let Some(price_state) = state_map.get(symbol) {
                if let Some(price) = price_state.futures_mark_price {
                    return Ok(price);
                }
            }
        }

        // 메모리에 없으면 HTTP 폴백
        warn!(
            "WebSocket에서 선물 마크 가격을 찾을 수 없어 HTTP로 조회합니다 (symbol: {})",
            symbol
        );
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

        let price = response.mark_price.parse::<f64>().map_err(|e| {
            ExchangeError::Other(format!("Failed to parse mark price as f64: {}", e))
        })?;

        // HTTP로 가져온 가격도 메모리에 저장
        {
            let mut state_map = self.price_state.write().await;
            let price_state = state_map
                .entry(symbol.to_string())
                .or_insert_with(PriceState::default);
            price_state.futures_mark_price = Some(price);
            price_state.last_updated = Some(std::time::SystemTime::now());
        }

        Ok(price)
    }

    /// 스팟 ticker WebSocket 연결 및 수신
    async fn start_spot_websocket(
        symbol: &str,
        state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    ) {
        let symbol_lower = symbol.to_lowercase();
        let stream_name = format!("{}@ticker", symbol_lower);
        let url = format!("{}/{}", SPOT_WS_URL, stream_name);

        loop {
            match Self::connect_spot_websocket(&url, symbol, state.clone()).await {
                Ok(_) => {
                    warn!(
                        "스팟 WebSocket 연결이 종료되었습니다. 재연결 시도... (symbol: {})",
                        symbol
                    );
                }
                Err(e) => {
                    warn!(
                        "스팟 WebSocket 오류: {:?}. 재연결 시도... (symbol: {})",
                        e, symbol
                    );
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    /// 선물 markPrice WebSocket 연결 및 수신
    async fn start_futures_websocket(
        symbol: &str,
        state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    ) {
        let symbol_lower = symbol.to_lowercase();
        let stream_name = format!("{}@markPrice", symbol_lower);
        let url = format!("{}/{}", FUTURES_WS_URL, stream_name);

        loop {
            match Self::connect_futures_websocket(&url, symbol, state.clone()).await {
                Ok(_) => {
                    warn!(
                        "선물 WebSocket 연결이 종료되었습니다. 재연결 시도... (symbol: {})",
                        symbol
                    );
                }
                Err(e) => {
                    warn!(
                        "선물 WebSocket 오류: {:?}. 재연결 시도... (symbol: {})",
                        e, symbol
                    );
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    /// 스팟 ticker WebSocket 연결 및 메시지 처리
    async fn connect_spot_websocket(
        url: &str,
        symbol: &str,
        state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    ) -> Result<(), ExchangeError> {
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| ExchangeError::Other(format!("WebSocket 연결 실패: {}", e)))?;
        let (_write, mut read) = ws_stream.split();

        info!("스팟 WebSocket 연결 성공: {} (symbol: {})", url, symbol);

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) =
                        Self::handle_spot_ticker_message(&text, symbol, state.clone()).await
                    {
                        warn!("스팟 ticker 메시지 처리 오류: {:?}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("스팟 WebSocket 연결이 닫혔습니다 (symbol: {})", symbol);
                    break;
                }
                Err(e) => {
                    warn!(
                        "스팟 WebSocket 메시지 수신 오류: {:?} (symbol: {})",
                        e, symbol
                    );
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// 선물 markPrice WebSocket 연결 및 메시지 처리
    async fn connect_futures_websocket(
        url: &str,
        symbol: &str,
        state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    ) -> Result<(), ExchangeError> {
        let (ws_stream, _) = connect_async(url)
            .await
            .map_err(|e| ExchangeError::Other(format!("WebSocket 연결 실패: {}", e)))?;
        let (_write, mut read) = ws_stream.split();

        info!("선물 WebSocket 연결 성공: {} (symbol: {})", url, symbol);

        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) =
                        Self::handle_futures_mark_price_message(&text, symbol, state.clone()).await
                    {
                        warn!("선물 markPrice 메시지 처리 오류: {:?}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("선물 WebSocket 연결이 닫혔습니다 (symbol: {})", symbol);
                    break;
                }
                Err(e) => {
                    warn!(
                        "선물 WebSocket 메시지 수신 오류: {:?} (symbol: {})",
                        e, symbol
                    );
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// 스팟 ticker 메시지 처리
    async fn handle_spot_ticker_message(
        text: &str,
        symbol: &str,
        state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    ) -> Result<(), ExchangeError> {
        #[derive(Debug, Deserialize)]
        struct SpotTicker {
            #[serde(rename = "s")]
            symbol: String,
            #[serde(rename = "c")]
            last_price: String,
        }

        let ticker: SpotTicker = serde_json::from_str(text).map_err(|e| {
            ExchangeError::Other(format!("스팟 ticker 파싱 실패: {} (text: {})", e, text))
        })?;

        if ticker.symbol != symbol {
            return Ok(());
        }

        let price: f64 = ticker.last_price.parse().map_err(|e| {
            ExchangeError::Other(format!(
                "가격 파싱 실패: {} (price: {})",
                e, ticker.last_price
            ))
        })?;

        let mut state_map = state.write().await;
        let price_state = state_map
            .entry(symbol.to_string())
            .or_insert_with(PriceState::default);
        price_state.spot_price = Some(price);
        price_state.last_updated = Some(std::time::SystemTime::now());

        Ok(())
    }

    /// 선물 markPrice 메시지 처리
    async fn handle_futures_mark_price_message(
        text: &str,
        symbol: &str,
        state: Arc<TokioRwLock<HashMap<String, PriceState>>>,
    ) -> Result<(), ExchangeError> {
        #[derive(Debug, Deserialize)]
        struct FuturesMarkPrice {
            #[serde(rename = "s")]
            symbol: String,
            #[serde(rename = "p")]
            mark_price: String,
        }

        let mark_price_data: FuturesMarkPrice = serde_json::from_str(text).map_err(|e| {
            ExchangeError::Other(format!("선물 markPrice 파싱 실패: {} (text: {})", e, text))
        })?;

        if mark_price_data.symbol != symbol {
            return Ok(());
        }

        let price: f64 = mark_price_data.mark_price.parse().map_err(|e| {
            ExchangeError::Other(format!(
                "가격 파싱 실패: {} (price: {})",
                e, mark_price_data.mark_price
            ))
        })?;

        let mut state_map = state.write().await;
        let price_state = state_map
            .entry(symbol.to_string())
            .or_insert_with(PriceState::default);
        price_state.futures_mark_price = Some(price);
        price_state.last_updated = Some(std::time::SystemTime::now());

        Ok(())
    }
}

impl BinanceUserStream {
    pub fn new(spot_client: BinanceClient) -> Self {
        Self { spot_client }
    }

    /// User Data Stream 시작 및 이벤트 수신
    pub async fn start<F>(&self, mut event_handler: F) -> Result<(), ExchangeError>
    where
        F: FnMut(UserDataEvent) + Send + 'static,
    {
        loop {
            match self.connect(&mut event_handler).await {
                Ok(_) => {
                    warn!("User Data Stream WebSocket 연결이 종료되었습니다. 재연결 시도...");
                }
                Err(e) => {
                    error!("User Data Stream WebSocket 오류: {:?}. 재연결 시도...", e);
                }
            }

            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    /// WebSocket 연결 및 메시지 수신
    async fn connect<F>(&self, event_handler: &mut F) -> Result<(), ExchangeError>
    where
        F: FnMut(UserDataEvent) + Send + 'static,
    {
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

        let (ws_stream, _) = connect_async(WS_API_URL)
            .await
            .map_err(|e| ExchangeError::Other(format!("WebSocket 연결 실패: {}", e)))?;

        let (mut write, mut read) = ws_stream.split();

        info!("User Data Stream WebSocket 연결 성공: {}", WS_API_URL);

        // 구독 요청 전송
        let _request_id = Self::subscribe_user_data_stream(&mut write, api_key, api_secret).await?;

        // 구독 응답 대기
        if let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    info!("Subscribe response: {}", text);
                    let response: WsResponse = serde_json::from_str(&text)
                        .map_err(|e| ExchangeError::Other(format!("응답 파싱 실패: {}", e)))?;

                    if let Some(error) = response.error {
                        return Err(ExchangeError::Other(format!(
                            "구독 실패: code={:?}, msg={:?}",
                            error.code, error.msg
                        )));
                    }

                    if let Some(result) = response.result {
                        info!(
                            "구독 성공: subscriptionId={:?}",
                            result.get("subscriptionId")
                        );
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("WebSocket 연결이 닫혔습니다");
                    return Ok(());
                }
                Err(e) => {
                    return Err(ExchangeError::Other(format!(
                        "구독 응답 수신 오류: {:?}",
                        e
                    )));
                }
                _ => {}
            }
        }

        info!("User Data Stream 이벤트 수신 대기 중...");

        // 이벤트 수신 루프
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = Self::handle_user_data_message(&text, event_handler) {
                        warn!("메시지 처리 오류: {:?}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    warn!("WebSocket 연결이 닫혔습니다");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    // Ping에 대한 Pong 응답
                    if let Err(e) = write.send(Message::Pong(data)).await {
                        error!("Pong 전송 실패: {:?}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("WebSocket 메시지 수신 오류: {:?}", e);
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// WebSocket API용 서명 생성
    /// signature를 제외한 params를 key 알파벳 순으로 정렬하여 서명
    fn sign_user_data_params(params: &mut BTreeMap<String, String>, secret: &str) -> String {
        // signature 제외한 params를 key 알파벳 순 정렬
        let mut items: Vec<(&String, &String)> = params.iter().collect();
        items.sort_by_key(|(k, _)| *k);

        let payload = items
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");

        generate_signature(&payload, secret)
    }

    /// User Data Stream 구독 요청 전송
    async fn subscribe_user_data_stream<S>(
        write: &mut S,
        api_key: &str,
        api_secret: &str,
    ) -> Result<String, ExchangeError>
    where
        S: SinkExt<Message> + Unpin,
        <S as futures_util::Sink<Message>>::Error: std::fmt::Debug,
    {
        let timestamp = get_timestamp().to_string();

        let mut params = BTreeMap::new();
        params.insert("apiKey".to_string(), api_key.to_string());
        params.insert("timestamp".to_string(), timestamp);
        // 필요하면 recvWindow 추가 가능
        // params.insert("recvWindow".to_string(), "5000".to_string());

        let signature = Self::sign_user_data_params(&mut params, api_secret);
        params.insert("signature".to_string(), signature);

        let request = WsRequest {
            id: "user-stream-1".to_string(),
            method: "userDataStream.subscribe.signature".to_string(),
            params,
        };

        let request_json = serde_json::to_string(&request)
            .map_err(|e| ExchangeError::Other(format!("Failed to serialize request: {}", e)))?;

        info!("Sending subscribe request: {}", request_json);

        write.send(Message::Text(request_json)).await.map_err(|e| {
            ExchangeError::Other(format!("Failed to send subscribe request: {:?}", e))
        })?;

        Ok("user-stream-1".to_string())
    }

    /// 메시지 처리 및 이벤트 파싱
    fn handle_user_data_message<F>(text: &str, event_handler: &mut F) -> Result<(), ExchangeError>
    where
        F: FnMut(UserDataEvent),
    {
        // 먼저 WsResponse로 파싱 시도
        if let Ok(response) = serde_json::from_str::<WsResponse>(text) {
            // 응답 형식인 경우
            if let Some(result) = response.result {
                // result 안에 이벤트가 있을 수 있음
                if let Some(event) = Self::parse_user_data_event(result) {
                    event_handler(event);
                }
            }
            return Ok(());
        }

        // 직접 이벤트 형식인 경우
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(parsed_event) = Self::parse_user_data_event(event) {
                event_handler(parsed_event);
            }
        }

        Ok(())
    }

    /// JSON Value에서 이벤트 파싱
    fn parse_user_data_event(value: serde_json::Value) -> Option<UserDataEvent> {
        // executionReport 이벤트 확인
        if let Some(event_type) = value.get("e").and_then(|v| v.as_str()) {
            match event_type {
                "executionReport" => {
                    if let Ok(report) = serde_json::from_value::<ExecutionReport>(value.clone()) {
                        return Some(UserDataEvent::ExecutionReport(report));
                    } else {
                        warn!("Failed to parse executionReport: {:?}", value);
                    }
                }
                "outboundAccountPosition" => {
                    if let Ok(position) =
                        serde_json::from_value::<OutboundAccountPosition>(value.clone())
                    {
                        return Some(UserDataEvent::OutboundAccountPosition(position));
                    } else {
                        warn!("Failed to parse outboundAccountPosition: {:?}", value);
                    }
                }
                "balanceUpdate" => {
                    if let Ok(update) = serde_json::from_value::<BalanceUpdate>(value.clone()) {
                        return Some(UserDataEvent::BalanceUpdate(update));
                    } else {
                        warn!("Failed to parse balanceUpdate: {:?}", value);
                    }
                }
                _ => {
                    info!("Unknown event type: {}", event_type);
                }
            }
        }

        // 파싱 실패한 경우 Unknown으로 처리
        Some(UserDataEvent::Unknown(value))
    }
}

impl BinanceTrader {
    pub fn new() -> Result<Self, ExchangeError> {
        let spot_client = BinanceClient::with_credentials()
            .map_err(|e| ExchangeError::Other(format!("Failed to create spot client: {}", e)))?;
        let futures_client = BinanceClient::with_credentials()
            .map_err(|e| ExchangeError::Other(format!("Failed to create futures client: {}", e)))?;

        let order_client = Arc::new(HttpBinanceOrderClient::new(
            spot_client.clone(),
            futures_client.clone(),
        ));
        let spot = Arc::new(BinanceSpotApi::new(spot_client.clone()));
        let futures = Arc::new(BinanceFuturesApi::new(futures_client.clone()));
        let price_feed = Arc::new(BinancePriceFeed::new(
            spot_client.clone(),
            futures_client.clone(),
        ));
        let user_stream = Some(Arc::new(BinanceUserStream::new(spot_client)));

        Ok(Self {
            order_client,
            spot,
            futures,
            price_feed,
            user_stream,
        })
    }

    /// 거래소 이름 반환
    pub fn exchange_name(&self) -> &'static str {
        "binance"
    }

    /// 특정 심볼에 대한 WebSocket 리스너 시작
    /// 스팟 ticker와 선물 markPrice를 동시에 구독
    pub fn start_websocket_listener(&self, symbol: &str) {
        self.price_feed.start_symbol(symbol);
    }

    /// 스팟 현재가 조회 (메모리에서 읽기, 없으면 HTTP 폴백)
    pub async fn get_spot_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        self.price_feed.get_spot_price(symbol).await
    }

    /// 선물 마크 가격 조회 (메모리에서 읽기, 없으면 HTTP 폴백)
    pub async fn get_futures_mark_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        self.price_feed.get_futures_mark_price(symbol).await
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
        self.spot.get_balance(asset).await
    }

    /// 특정 심볼의 거래 수수료 조회
    pub async fn get_trade_fee_for_symbol(
        &self,
        symbol: &str,
    ) -> Result<interface::FeeInfo, ExchangeError> {
        self.spot.client().get_trade_fee_for_symbol(symbol).await
    }

    /// 선물 잔고 조회 (USDT 마진)
    pub async fn get_futures_balance(&self) -> Result<f64, ExchangeError> {
        self.futures.get_balance().await
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
        self.spot.clamp_quantity(symbol, qty)
    }

    /// 선물 수량을 거래소 규칙에 맞게 조정 (LOT_SIZE)
    /// exchangeInfo에서 가져온 실제 LOT_SIZE 필터를 사용
    pub fn clamp_futures_quantity(&self, symbol: &str, qty: f64) -> f64 {
        self.futures.clamp_quantity(symbol, qty)
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
        let fut_lot = self.futures.get_lot_size(symbol)?;
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
            .spot
            .get_lot_size(symbol)
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

    /// 스팟 exchangeInfo를 로드하여 LOT_SIZE 필터를 캐시에 저장
    pub async fn load_spot_exchange_info(&self) -> Result<(), ExchangeError> {
        self.spot.load_exchange_info().await
    }

    /// 선물 exchangeInfo를 로드하여 LOT_SIZE 필터를 캐시에 저장
    pub async fn load_futures_exchange_info(&self) -> Result<(), ExchangeError> {
        self.futures.load_exchange_info().await
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
        self.order_client
            .place_spot_order(symbol, side, quantity, None, PlaceOrderOptions { test })
            .await
    }

    /// 선물 시장가 주문
    pub async fn place_futures_order(
        &self,
        symbol: &str,
        side: &str, // "BUY" or "SELL"
        quantity: f64,
        reduce_only: bool,
    ) -> Result<OrderResponse, ExchangeError> {
        self.order_client
            .place_futures_order(
                symbol,
                side,
                quantity,
                None,
                PlaceFuturesOrderOptions { reduce_only },
            )
            .await
    }
    /// User Data Stream 시작 및 이벤트 수신
    pub async fn start_user_data_stream<F>(&self, event_handler: F) -> Result<(), ExchangeError>
    where
        F: FnMut(UserDataEvent) + Send + 'static,
    {
        if let Some(user_stream) = &self.user_stream {
            user_stream.start(event_handler).await
        } else {
            Err(ExchangeError::Other(
                "User stream not initialized".to_string(),
            ))
        }
    }
}

// ========== User Data Stream 관련 타입 정의 ==========

/// WebSocket API 요청 메시지
#[derive(Debug, Serialize)]
struct WsRequest {
    id: String,
    method: String,
    params: BTreeMap<String, String>,
}

/// WebSocket API 응답 메시지
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WsResponse {
    id: Option<String>,
    status: Option<u16>,
    result: Option<serde_json::Value>,
    error: Option<WsError>,
}

/// WebSocket API 에러
#[derive(Debug, Deserialize)]
struct WsError {
    code: Option<i32>,
    msg: Option<String>,
}

/// 주문 실행 리포트 (executionReport)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    /// 이벤트 타입
    #[serde(rename = "e")]
    pub event_type: String,
    /// 이벤트 시간
    #[serde(rename = "E")]
    pub event_time: u64,
    /// 심볼
    #[serde(rename = "s")]
    pub symbol: String,
    /// 클라이언트 주문 ID
    #[serde(rename = "c")]
    pub client_order_id: String,
    /// 주문 방향 (BUY/SELL)
    #[serde(rename = "S")]
    pub side: String,
    /// 주문 타입
    #[serde(rename = "o")]
    pub order_type: String,
    /// 시장가 주문 시 사용 (MARKET)
    #[serde(rename = "f")]
    pub time_in_force: String,
    /// 주문 수량
    #[serde(rename = "q")]
    pub order_quantity: String,
    /// 주문 가격
    #[serde(rename = "p")]
    pub order_price: String,
    /// 현재 주문 상태
    #[serde(rename = "X")]
    pub current_order_status: String,
    /// 마지막 실행 수량
    #[serde(rename = "l")]
    pub last_executed_quantity: String,
    /// 누적 실행 수량
    #[serde(rename = "z")]
    pub cumulative_filled_quantity: String,
    /// 마지막 실행 가격
    #[serde(rename = "L")]
    pub last_executed_price: String,
    /// 수수료
    #[serde(rename = "n")]
    pub commission_amount: String,
    /// 수수료 자산
    #[serde(rename = "N")]
    pub commission_asset: Option<String>,
    /// 주문 생성 시간
    #[serde(rename = "O")]
    pub order_create_time: u64,
    /// 거래 ID
    #[serde(rename = "T")]
    pub transaction_time: u64,
    /// 주문 ID
    #[serde(rename = "i")]
    pub order_id: u64,
    /// 누적 인용 수량
    #[serde(rename = "Z")]
    pub cumulative_quote_quantity: Option<String>,
    /// 마지막 인용 수량
    #[serde(rename = "Y")]
    pub last_quote_transacted: Option<String>,
    /// 주문 리스트 ID
    #[serde(rename = "g")]
    pub order_list_id: Option<i64>,
    /// 원래 클라이언트 주문 ID
    #[serde(rename = "C")]
    pub original_client_order_id: Option<String>,
    /// 스톱 가격
    #[serde(rename = "P")]
    pub stop_price: Option<String>,
    /// 거부된 수량
    #[serde(rename = "d")]
    pub rejected_quantity: Option<String>,
    /// 거부된 수량의 원인
    #[serde(rename = "j")]
    pub reject_reason: Option<String>,
}

/// User Data Stream 이벤트 타입
#[derive(Debug, Clone)]
pub enum UserDataEvent {
    ExecutionReport(ExecutionReport),
    OutboundAccountPosition(OutboundAccountPosition),
    BalanceUpdate(BalanceUpdate),
    Unknown(serde_json::Value),
}

/// 계정 정보 업데이트 (outboundAccountPosition)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundAccountPosition {
    /// 이벤트 타입
    #[serde(rename = "e")]
    pub event_type: String,
    /// 이벤트 시간
    #[serde(rename = "E")]
    pub event_time: u64,
    /// 마지막 업데이트 시간
    #[serde(rename = "u")]
    pub last_update_time: u64,
    /// 잔고 정보
    #[serde(rename = "B")]
    pub balances: Vec<BalanceInfo>,
}

/// 잔고 정보
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceInfo {
    /// 자산
    #[serde(rename = "a")]
    pub asset: String,
    /// 사용 가능한 잔고
    #[serde(rename = "f")]
    pub free: String,
    /// 잠긴 잔고
    #[serde(rename = "l")]
    pub locked: String,
}

/// 잔고 업데이트 (balanceUpdate)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceUpdate {
    /// 이벤트 타입
    #[serde(rename = "e")]
    pub event_type: String,
    /// 이벤트 시간
    #[serde(rename = "E")]
    pub event_time: u64,
    /// 자산
    #[serde(rename = "a")]
    pub asset: String,
    /// 잔고 변화량
    #[serde(rename = "d")]
    pub balance_delta: String,
    /// 지갑 타입
    #[serde(rename = "w")]
    pub wallet_type: Option<String>,
}
