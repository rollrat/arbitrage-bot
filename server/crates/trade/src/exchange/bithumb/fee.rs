use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use tokio::sync::RwLock;

use interface::{DepositWithdrawalFee, ExchangeId, FeeInfo, MarketType};

use super::super::FeeExchange;
use super::{BithumbOrderBookClient, BASE_URL};

const FEE_API_URL: &str = "/v2/fee/inout/ALL";

/// API 응답 구조체
#[derive(Debug, Deserialize)]
struct FeeApiResponse {
    name: String,
    currency: String,
    networks: Vec<NetworkFee>,
}

#[derive(Debug, Deserialize)]
struct NetworkFee {
    #[serde(rename = "net_name")]
    net_name: String,
    #[serde(rename = "deposit_fee_quantity")]
    deposit_fee_quantity: String,
    #[serde(rename = "deposit_minimum_quantity")]
    deposit_minimum_quantity: String,
    #[serde(rename = "withdraw_fee_quantity")]
    withdraw_fee_quantity: String,
    #[serde(rename = "withdraw_minimum_quantity")]
    withdraw_minimum_quantity: String,
}

/// 입출금 수수료 캐시
static FEE_CACHE: tokio::sync::OnceCell<Arc<RwLock<HashMap<String, DepositWithdrawalFee>>>> =
    tokio::sync::OnceCell::const_new();

/// 캐시 초기화 (한 번만 실행)
async fn init_fee_cache() -> Arc<RwLock<HashMap<String, DepositWithdrawalFee>>> {
    FEE_CACHE
        .get_or_init(|| async { Arc::new(RwLock::new(HashMap::new())) })
        .await
        .clone()
}

impl BithumbOrderBookClient {
    /// 입출금 수수료 캐시 초기화 및 업데이트
    pub async fn refresh_deposit_withdrawal_fees(
        &self,
    ) -> Result<HashMap<String, DepositWithdrawalFee>, super::super::ExchangeError> {
        let url = format!("{BASE_URL}{FEE_API_URL}");
        let http = reqwest::Client::new();
        let response = http.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(super::super::ExchangeError::Other(format!(
                "Failed to fetch fee API: status {}",
                response.status()
            )));
        }

        let response_text = response.text().await?;
        let api_responses: Vec<FeeApiResponse> =
            serde_json::from_str(&response_text).map_err(|e| {
                super::super::ExchangeError::Other(format!(
                    "Failed to parse fee API response: {}, response: {}",
                    e,
                    response_text.chars().take(200).collect::<String>()
                ))
            })?;

        let mut fees = HashMap::new();
        let now = Utc::now();

        for api_response in api_responses {
            let currency = api_response.currency.to_uppercase();

            // 여러 네트워크가 있는 경우, 첫 번째 네트워크 사용 (또는 평균 계산 가능)
            // 일단 첫 번째 네트워크의 수수료 사용
            if let Some(network) = api_response.networks.first() {
                let deposit_fee = network.deposit_fee_quantity.parse::<f64>().unwrap_or(0.0);
                let withdrawal_fee = network.withdraw_fee_quantity.parse::<f64>().unwrap_or(0.0);

                fees.insert(
                    currency.clone(),
                    DepositWithdrawalFee {
                        currency,
                        deposit_fee,
                        withdrawal_fee,
                        updated_at: now,
                    },
                );
            }
        }

        tracing::info!("Parsed {} deposit/withdrawal fees from API", fees.len());

        // 캐시 업데이트
        let cache = init_fee_cache().await;
        *cache.write().await = fees.clone();

        Ok(fees)
    }

    /// 입출금 수수료 캐시 가져오기 (없으면 초기화)
    async fn get_fee_cache(
        &self,
    ) -> Result<Arc<RwLock<HashMap<String, DepositWithdrawalFee>>>, super::super::ExchangeError>
    {
        let cache = init_fee_cache().await;

        // 캐시가 비어있으면 초기화
        if cache.read().await.is_empty() {
            self.refresh_deposit_withdrawal_fees().await?;
        }

        Ok(cache)
    }
}

#[async_trait]
impl FeeExchange for BithumbOrderBookClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bithumb
    }

    fn get_fee(&self, market_type: MarketType) -> FeeInfo {
        match market_type {
            MarketType::KRW => FeeInfo::new(0.0004, 0.0004), // 0.04% 메이커, 테이커
            MarketType::USDT => FeeInfo::new(0.0004, 0.0004), // 0.04% 메이커, 테이커
            MarketType::BTC => FeeInfo::free(),              // 수수료 무료
            MarketType::Other(_) => FeeInfo::new(0.0004, 0.0004), // 기본값: 0.04%
        }
    }

    async fn get_deposit_withdrawal_fee(
        &self,
        currency: &str,
    ) -> Result<DepositWithdrawalFee, super::super::ExchangeError> {
        let cache = self.get_fee_cache().await?;
        let cache_guard = cache.read().await;

        let currency_upper = currency.to_uppercase();
        cache_guard.get(&currency_upper).cloned().ok_or_else(|| {
            super::super::ExchangeError::Other(format!(
                "Fee information not found for currency: {}",
                currency
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_refresh_deposit_withdrawal_fees() {
        let client = BithumbOrderBookClient::new();
        let result = client.refresh_deposit_withdrawal_fees().await;

        match result {
            Ok(fees) => {
                println!(
                    "Successfully fetched {} deposit/withdrawal fees",
                    fees.len()
                );

                assert!(!fees.is_empty(), "Should fetch at least one fee");

                // 처음 10개 출력
                for (currency, fee) in fees.iter().take(10) {
                    println!(
                        "{} - Deposit: {}, Withdrawal: {}",
                        currency, fee.deposit_fee, fee.withdrawal_fee
                    );
                }

                // BTC 수수료 확인 (일반적으로 존재)
                if let Some(btc_fee) = fees.get("BTC") {
                    println!(
                        "\nBTC - Deposit: {}, Withdrawal: {}",
                        btc_fee.deposit_fee, btc_fee.withdrawal_fee
                    );
                }
            }
            Err(e) => {
                panic!("Failed to fetch fees: {:?}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_get_deposit_withdrawal_fee() {
        let client = BithumbOrderBookClient::new();

        // 먼저 캐시 초기화
        let _ = client.refresh_deposit_withdrawal_fees().await;

        // BTC 수수료 조회
        let result = client.get_deposit_withdrawal_fee("BTC").await;

        match result {
            Ok(fee) => {
                assert_eq!(fee.currency, "BTC");
                assert!(fee.deposit_fee >= 0.0);
                assert!(fee.withdrawal_fee >= 0.0);
                println!(
                    "BTC fees - Deposit: {}, Withdrawal: {}",
                    fee.deposit_fee, fee.withdrawal_fee
                );
            }
            Err(e) => {
                eprintln!("Warning: Failed to get BTC fee: {:?}", e);
            }
        }
    }
}
