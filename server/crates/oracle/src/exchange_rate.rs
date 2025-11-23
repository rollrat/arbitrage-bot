use chrono::Utc;
use eyre::Result;
use serde::Deserialize;
use tracing::info;

use interface::ExchangeRates;

const EXCHANGE_RATE_API_URL: &str = "https://api.exchangerate-api.com/v4/latest/USD";
const BITHUMB_API_URL: &str = "https://api.bithumb.com/public/ticker/USDT_KRW";
const FALLBACK_USD_KRW: f64 = 1300.0; // 대략 1 USD = 1300 KRW
const FALLBACK_USDT_USD: f64 = 1.0; // USDT는 보통 USD와 1:1
const FALLBACK_USDT_KRW: f64 = 1300.0; // 대략 1 USDT = 1300 KRW

#[derive(Debug, Deserialize)]
struct ExchangeRateApiResponse {
    rates: ExchangeRateRates,
}

#[derive(Debug, Deserialize)]
struct ExchangeRateRates {
    #[serde(rename = "KRW")]
    krw: Option<f64>,
}

/// USD/KRW 환율을 가져옵니다.
/// 1 USD = ? KRW 형식으로 반환합니다.
pub async fn fetch_usd_krw_rate() -> Result<f64> {
    let client = reqwest::Client::new();
    let response = client.get(EXCHANGE_RATE_API_URL).send().await?;
    let data: ExchangeRateApiResponse = response.json().await?;

    let usd_krw = data.rates.krw.unwrap_or(FALLBACK_USD_KRW);

    info!(
        "USD/KRW 환율 가져오기 성공: {} (1 USD = {} KRW)",
        usd_krw, usd_krw
    );
    Ok(usd_krw)
}

#[derive(Debug, Deserialize)]
struct BinancePriceResponse {
    price: String,
}

/// USDT/USD 환율을 가져옵니다.
/// Binance Spot에서 USDC/USDT 가격을 가져와서 역으로 계산합니다.
pub async fn fetch_usdt_usd_rate() -> Result<f64> {
    let client = reqwest::Client::new();
    let url = "https://api.binance.com/api/v3/ticker/price?symbol=USDCUSDT";

    let response = client.get(url).send().await?;
    let data: BinancePriceResponse = response.json().await?;
    let usdc_usdt = data.price.parse::<f64>()?;

    // USDC/USDT 가격의 역수 = USDT/USD (USDC는 USD와 1:1)
    let usdt_usd = 1.0 / usdc_usdt;
    info!(
        "Binance에서 USDT/USD 환율 가져오기 성공: {} (USDC/USDT: {})",
        usdt_usd, usdc_usdt
    );
    Ok(usdt_usd)
}

#[derive(Debug, Deserialize)]
struct BithumbTickerResponse {
    status: String,
    data: BithumbTickerData,
}

#[derive(Debug, Deserialize)]
struct BithumbTickerData {
    #[serde(rename = "closing_price")]
    closing_price: String,
}

/// USDT/KRW 환율을 가져옵니다.
/// Bithumb에서 USDT/KRW 가격을 가져옵니다.
/// 1 USDT = ? KRW 형식으로 반환합니다.
pub async fn fetch_usdt_krw_rate() -> Result<f64> {
    let client = reqwest::Client::new();
    let response = client.get(BITHUMB_API_URL).send().await?;
    let data: BithumbTickerResponse = response.json().await?;

    if data.status != "0000" {
        return Ok(FALLBACK_USDT_KRW);
    }

    let usdt_krw = data.data.closing_price.parse::<f64>()?;
    info!(
        "Bithumb에서 USDT/KRW 환율 가져오기 성공: {} (1 USDT = {} KRW)",
        usdt_krw, usdt_krw
    );
    Ok(usdt_krw)
}

/// 모든 환율 정보를 가져옵니다.
pub async fn fetch_all_exchange_rates() -> ExchangeRates {
    let usd_krw = fetch_usd_krw_rate().await.unwrap_or(FALLBACK_USD_KRW);
    let usdt_usd = fetch_usdt_usd_rate().await.unwrap_or(FALLBACK_USDT_USD);
    let usdt_krw = fetch_usdt_krw_rate().await.unwrap_or(FALLBACK_USDT_KRW);

    ExchangeRates {
        usd_krw,
        usdt_usd,
        usdt_krw,
        updated_at: Utc::now(),
    }
}
