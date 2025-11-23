use std::env;

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use uuid::Uuid;

use super::ExchangeError;

pub mod asset;
mod fee;
pub mod orderbook;

pub const BASE_URL: &str = "https://api.bithumb.com";

#[derive(Debug, Serialize)]
pub struct JwtPayload {
    pub access_key: String,
    pub nonce: String,
    pub timestamp: i64,
}

/// JWT 토큰 생성 (신버전 API /v1/* 엔드포인트용)
/// 파라미터가 없는 경우 (GET /v1/accounts)
pub fn generate_jwt_token(api_key: &str, api_secret: &str) -> Result<String, ExchangeError> {
    let payload = JwtPayload {
        access_key: api_key.to_string(),
        nonce: Uuid::new_v4().to_string(),
        timestamp: Utc::now().timestamp_millis(),
    };

    let header = Header::new(Algorithm::HS256);
    let encoding_key = EncodingKey::from_secret(api_secret.as_ref());

    encode(&header, &payload, &encoding_key)
        .map_err(|e| ExchangeError::Other(format!("Failed to generate JWT token: {}", e)))
}

/// 환경변수에서 API 키와 시크릿 가져오기
pub fn get_api_credentials() -> Result<(String, String), ExchangeError> {
    let api_key = env::var("BITHUMB_API_KEY")
        .map_err(|e| ExchangeError::Other(format!("BITHUMB_API_KEY not found: {}", e)))?;
    let api_secret = env::var("BITHUMB_API_SECRET")
        .map_err(|e| ExchangeError::Other(format!("BITHUMB_API_SECRET not found: {}", e)))?;
    Ok((api_key, api_secret))
}

/// 환경변수가 설정되어 있는지 확인
pub fn has_api_credentials() -> bool {
    env::var("BITHUMB_API_KEY").is_ok() && env::var("BITHUMB_API_SECRET").is_ok()
}

pub use asset::BithumbAssetClient;
pub use orderbook::BithumbOrderBookClient;
