use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha512;
use tracing::{info, warn};

use exchanges::{
    AssetExchange,
    bithumb::{self, BASE_URL, BithumbClient},
};
use interface::ExchangeError;

use super::{OrderResponse, SpotExchangeTrader};

type HmacSha512 = Hmac<Sha512>;

const MARKET_BUY_ENDPOINT: &str = "/trade/market_buy";
const MARKET_SELL_ENDPOINT: &str = "/trade/market_sell";
const TICKER_ENDPOINT: &str = "/public/ticker";
const DEFAULT_STEP_SIZE: f64 = 0.0001;

#[async_trait]
impl SpotExchangeTrader for BithumbTrader {
    async fn ensure_exchange_info(&self) -> Result<(), ExchangeError> {
        // 빗썸은 Binance 처럼 별도의 exchangeInfo 가 필요하지 않으므로 no-op.
        Ok(())
    }

    async fn get_spot_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        self.fetch_price(symbol).await
    }

    fn clamp_spot_quantity(&self, symbol: &str, qty: f64) -> f64 {
        let step = Self::step_size_for(&symbol.to_uppercase());
        let clamped = Self::clamp_quantity(qty, step);
        if clamped <= 0.0 {
            warn!(
                "Quantity too small after clamp for {} (step {}). Requested: {}",
                symbol, step, qty
            );
        }
        clamped
    }

    async fn buy_spot(&self, symbol: &str, qty: f64) -> Result<OrderResponse, ExchangeError> {
        self.place_market_order(symbol, qty, MARKET_BUY_ENDPOINT)
            .await
    }

    async fn sell_spot(&self, symbol: &str, qty: f64) -> Result<OrderResponse, ExchangeError> {
        self.place_market_order(symbol, qty, MARKET_SELL_ENDPOINT)
            .await
    }

    async fn get_spot_balance(&self, asset: &str) -> Result<f64, ExchangeError> {
        let assets = self.client.fetch_spots().await?;
        let target = asset.to_uppercase();
        Ok(assets
            .iter()
            .find(|a| a.currency == target)
            .map(|a| a.available)
            .unwrap_or(0.0))
    }
}

/// 빗썸 spot 전용 트레이더.
/// - BithumbClient 로 계좌/잔고 정보를 재사용하고,
/// - 신규 주문/호가 조회는 REST API를 직접 호출한다.
pub struct BithumbTrader {
    client: BithumbClient,
    http: reqwest::Client,
    api_key: String,
    api_secret: String,
}

impl BithumbTrader {
    pub fn new() -> Result<Self, ExchangeError> {
        let client = BithumbClient::with_credentials()?;
        let (api_key, api_secret) = bithumb::get_api_credentials()?;

        Ok(Self {
            client,
            http: reqwest::Client::new(),
            api_key,
            api_secret,
        })
    }

    fn split_symbol(symbol: &str) -> Result<(String, String), ExchangeError> {
        let cleaned = symbol.replace('-', "").to_uppercase();
        for quote in ["KRW", "USDT", "BTC", "USD"] {
            if cleaned.ends_with(quote) && cleaned.len() > quote.len() {
                let base = cleaned[..cleaned.len() - quote.len()].to_string();
                if base.is_empty() {
                    break;
                }
                return Ok((base, quote.to_string()));
            }
        }
        Err(ExchangeError::Other(format!(
            "Unsupported Bithumb symbol: {}",
            symbol
        )))
    }

    fn build_pair(symbol: &str) -> Result<String, ExchangeError> {
        let (base, quote) = Self::split_symbol(symbol)?;
        Ok(format!("{}_{}", base, quote))
    }

    fn step_size_for(symbol: &str) -> f64 {
        if symbol.ends_with("KRW") {
            DEFAULT_STEP_SIZE
        } else {
            0.000001
        }
    }

    fn clamp_quantity(qty: f64, step: f64) -> f64 {
        if qty <= 0.0 {
            return 0.0;
        }
        let steps = (qty / step).floor();
        let clamped = steps * step;
        if clamped.is_finite() { clamped } else { 0.0 }
    }

    fn sign_request(
        &self,
        endpoint: &str,
        params: &str,
        nonce: &str,
    ) -> Result<String, ExchangeError> {
        let payload = format!("{endpoint}\0{params}\0{nonce}");
        let mut mac = HmacSha512::new_from_slice(self.api_secret.as_bytes())
            .map_err(|e| ExchangeError::Other(format!("Failed to create HMAC signer: {}", e)))?;
        mac.update(payload.as_bytes());
        Ok(BASE64.encode(mac.finalize().into_bytes()))
    }

    async fn post_private(&self, endpoint: &str, params: &str) -> Result<Value, ExchangeError> {
        let nonce = Utc::now().timestamp_micros().to_string();
        let signature = self.sign_request(endpoint, params, &nonce)?;
        let url = format!("{}{}", BASE_URL, endpoint);

        info!("post_private url: {}", url);

        let response = self
            .http
            .post(&url)
            .header("Api-Key", &self.api_key)
            .header("Api-Sign", signature)
            .header("Api-Nonce", &nonce)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(params.to_string())
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let body = response.text().await?;

        info!("post_private response: {}", body);

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Bithumb API HTTP error: status {}, response: {}",
                status,
                body.chars().take(200).collect::<String>()
            )));
        }

        #[derive(Deserialize)]
        struct PrivateResponse {
            status: String,
            data: Option<Value>,
        }

        let parsed: PrivateResponse = serde_json::from_str(&body).map_err(|e| {
            ExchangeError::Other(format!(
                "Failed to parse Bithumb response: {}, payload: {}",
                e,
                body.chars().take(200).collect::<String>()
            ))
        })?;

        if parsed.status != "0000" {
            return Err(ExchangeError::Other(format!(
                "Bithumb API error: status {}, response: {}",
                parsed.status,
                body.chars().take(200).collect::<String>()
            )));
        }

        Ok(parsed.data.unwrap_or(Value::Null))
    }

    async fn place_market_order(
        &self,
        symbol: &str,
        qty: f64,
        endpoint: &str,
    ) -> Result<OrderResponse, ExchangeError> {
        if qty <= 0.0 {
            return Err(ExchangeError::Other(
                "Quantity must be positive".to_string(),
            ));
        }

        let (base, quote) = Self::split_symbol(symbol)?;
        let params = format!(
            "order_currency={}&payment_currency={}&units={:.8}",
            base, quote, qty
        );

        let data = self.post_private(endpoint, &params).await?;

        let order_id = data
            .get("order_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());
        let status = data
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let executed_qty = data
            .get("order_qty")
            .or_else(|| data.get("units"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let order_response = OrderResponse {
            symbol: symbol.to_string(),
            order_id,
            client_order_id: data
                .get("order_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            executed_qty,
            status,
            extra: data,
        };

        // 거래 기록 저장
        crate::record::save_trade_record_bithumb_order(
            "bithumb",
            symbol,
            endpoint,
            qty,
            &params,
            &order_response,
            false,
        )
        .await;

        Ok(order_response)
    }

    async fn fetch_price(&self, symbol: &str) -> Result<f64, ExchangeError> {
        let pair = Self::build_pair(symbol)?;
        let url = format!("{}{}/{}", BASE_URL, TICKER_ENDPOINT, pair);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ExchangeError::Other(format!("HTTP error: {}", e)))?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(ExchangeError::Other(format!(
                "Bithumb ticker error: status {}, response: {}",
                status,
                body.chars().take(200).collect::<String>()
            )));
        }

        #[derive(Deserialize)]
        struct TickerData {
            #[serde(rename = "closing_price")]
            closing_price: String,
        }

        #[derive(Deserialize)]
        struct TickerResponse {
            status: String,
            data: TickerData,
        }

        let parsed: TickerResponse = serde_json::from_str(&body).map_err(|e| {
            ExchangeError::Other(format!(
                "Failed to parse ticker response: {}, payload: {}",
                e,
                body.chars().take(200).collect::<String>()
            ))
        })?;

        if parsed.status != "0000" {
            return Err(ExchangeError::Other(format!(
                "Bithumb ticker API error: status {}",
                parsed.status
            )));
        }

        parsed
            .data
            .closing_price
            .parse::<f64>()
            .map_err(|e| ExchangeError::Other(format!("Invalid closing_price: {}", e)))
    }
}
