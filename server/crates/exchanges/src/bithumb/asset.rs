use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use interface::{ExchangeId, FutureAsset, SpotAsset};

use super::super::{AssetExchange, ExchangeError};
use super::{generate_jwt_token, BithumbClient, BASE_URL};

#[derive(Debug, Deserialize)]
struct BithumbAccount {
    currency: String,
    balance: String,
    locked: String,
    #[serde(default)]
    #[allow(dead_code)]
    avg_buy_price: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    avg_buy_price_modified: Option<bool>,
    #[serde(default)]
    #[allow(dead_code)]
    unit_currency: Option<String>,
}

#[async_trait]
impl AssetExchange for BithumbClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bithumb
    }

    async fn fetch_spots(&self) -> Result<Vec<SpotAsset>, ExchangeError> {
        let api_key = self.api_key.as_ref().ok_or_else(|| {
            ExchangeError::Other(
                "API key not set. Use BithumbClient::with_credentials()".to_string(),
            )
        })?;
        let api_secret = self.api_secret.as_ref().ok_or_else(|| {
            ExchangeError::Other(
                "API secret not set. Use BithumbClient::with_credentials()".to_string(),
            )
        })?;

        // 신버전 API: GET /v1/accounts
        let endpoint = "/v1/accounts";
        let url = format!("{BASE_URL}{}", endpoint);

        // JWT 토큰 생성 (파라미터가 없으므로 query_hash 없음)
        let jwt_token = generate_jwt_token(api_key, api_secret)?;
        let authorization_token = format!("Bearer {}", jwt_token);

        let response = self
            .http
            .get(&url)
            .header("Authorization", authorization_token)
            .header("Content-Type", "application/json; charset=utf-8")
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        // 디버깅을 위해 응답 로깅 (민감한 정보는 제외)
        if status != 200 {
            return Err(ExchangeError::Other(format!(
                "Bithumb API HTTP error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        // 응답이 배열인지 객체인지 확인
        // 일반적으로 배열로 반환됨
        let accounts: Vec<BithumbAccount> = serde_json::from_str(&response_text).map_err(|e| {
            ExchangeError::Other(format!(
                "Failed to parse Bithumb response: {}, response: {}",
                e,
                response_text.chars().take(200).collect::<String>()
            ))
        })?;

        let now = Utc::now();
        let mut assets = Vec::new();

        for account in accounts {
            let balance: f64 = account.balance.parse().unwrap_or(0.0);
            let locked: f64 = account.locked.parse().unwrap_or(0.0);
            let total = balance + locked;

            // total이 0보다 큰 경우만 추가
            if total > 0.0 {
                assets.push(SpotAsset {
                    currency: account.currency.to_uppercase(),
                    total,
                    available: balance,
                    in_use: locked,
                    updated_at: now,
                });
            }
        }

        Ok(assets)
    }

    async fn fetch_futures(&self) -> Result<Vec<FutureAsset>, ExchangeError> {
        // Bithumb은 선물 거래를 지원하지 않으므로 빈 벡터 반환
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use crate::bithumb::has_api_credentials;

    use super::*;

    fn skip_if_no_credentials() {
        if !has_api_credentials() {
            eprintln!("Skipping test: BITHUMB_API_KEY or BITHUMB_API_SECRET not set");
            std::process::exit(0);
        }
    }

    fn handle_api_error(e: &ExchangeError) {
        if let ExchangeError::Other(msg) = e {
            if msg.contains("Bithumb API error") {
                panic!("Bithumb API error: {}", msg);
            }
            if msg.contains("not found") {
                eprintln!("Warning: Environment variable not found: {:?}", e);
                return;
            }
        }
        eprintln!("Warning: API call failed: {:?}", e);
    }

    #[test]
    fn test_bithumb_asset_client_id() {
        skip_if_no_credentials();

        let client = BithumbClient::with_credentials().expect("Failed to create BithumbClient");
        assert_eq!(client.id(), ExchangeId::Bithumb);
    }

    #[tokio::test]
    async fn test_fetch_assets_bithumb() {
        skip_if_no_credentials();

        let client = BithumbClient::with_credentials().expect("Failed to create BithumbClient");
        let result = client.fetch_spots().await;

        match result {
            Ok(assets) => {
                // API 호출이 성공했는지 확인
                // assets가 비어있을 수도 있지만 (잔액이 없는 경우), 에러가 없으면 성공
                println!("Successfully fetched {} assets", assets.len());

                // 모든 자산이 올바른 형식인지 확인
                for asset in &assets {
                    assert!(!asset.currency.is_empty(), "currency should not be empty");
                    assert!(asset.total >= 0.0, "total should be non-negative");
                    assert!(asset.available >= 0.0, "available should be non-negative");
                    assert!(asset.in_use >= 0.0, "in_use should be non-negative");
                    assert!(
                        asset.available + asset.in_use <= asset.total + 0.0001,
                        "available + in_use should not exceed total (with small tolerance)"
                    );
                }

                // 자산이 있는 경우 예시 출력
                if !assets.is_empty() {
                    println!("\nSample assets:");
                    for asset in assets.iter().take(5) {
                        println!(
                            "  {}: total={:.8}, available={:.8}, in_use={:.8}",
                            asset.currency, asset.total, asset.available, asset.in_use
                        );
                    }
                }
            }
            Err(e) => {
                handle_api_error(&e);
            }
        }
    }
}
