use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::fmt::Display;
use std::str::FromStr;

/// 거래 유형
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeType {
    /// 시장가 주문
    Market,
    /// 지정가 주문
    Limit,
    /// 기타
    Other,
}

impl Display for TradeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeType::Market => write!(f, "MARKET"),
            TradeType::Limit => write!(f, "LIMIT"),
            TradeType::Other => write!(f, "OTHER"),
        }
    }
}

impl FromStr for TradeType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "MARKET" => Ok(TradeType::Market),
            "LIMIT" => Ok(TradeType::Limit),
            "OTHER" => Ok(TradeType::Other),
            _ => Err(format!("Invalid TradeType: {}", s)),
        }
    }
}

/// 선물/현물 구분
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketType {
    /// 현물
    Spot,
    /// 선물
    Futures,
}

impl Display for MarketType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketType::Spot => write!(f, "SPOT"),
            MarketType::Futures => write!(f, "FUTURES"),
        }
    }
}

impl FromStr for MarketType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SPOT" => Ok(MarketType::Spot),
            "FUTURES" => Ok(MarketType::Futures),
            _ => Err(format!("Invalid MarketType: {}", s)),
        }
    }
}

/// 거래 방향 (매수/매도)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeSide {
    /// 매수
    Buy,
    /// 매도
    Sell,
}

impl Display for TradeSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeSide::Buy => write!(f, "BUY"),
            TradeSide::Sell => write!(f, "SELL"),
        }
    }
}

impl FromStr for TradeSide {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "BUY" => Ok(TradeSide::Buy),
            "SELL" => Ok(TradeSide::Sell),
            _ => Err(format!("Invalid TradeSide: {}", s)),
        }
    }
}

/// 거래 기록 데이터 구조
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    /// 거래 UTC 시간
    pub executed_at: DateTime<Utc>,
    /// 거래소 이름 (예: "binance", "bithumb")
    pub exchange: String,
    /// 코인 이름/심볼 (예: "BTCUSDT", "BTC-KRW")
    pub symbol: String,
    /// 선/현물 정보
    pub market_type: MarketType,
    /// 거래 방향
    pub side: TradeSide,
    /// 거래 유형
    pub trade_type: TradeType,
    /// 실행 가격 (None일 수 있음, 시장가 등)
    pub executed_price: Option<f64>,
    /// 거래 수량
    pub quantity: f64,
    /// 요청 쿼리 스트링 전문
    pub request_query_string: Option<String>,
    /// API 요청 응답 전문 (JSON 문자열)
    pub api_response: Option<String>,
    /// 추가 메타데이터 (JSON 문자열)
    pub metadata: Option<String>,
    /// 청산 실행 기록 여부
    pub is_liquidation: bool,
}

/// 거래 기록 저장소 인터페이스
/// 확장성을 위해 트레이트로 정의하여 나중에 다른 DB로 전환 가능
#[async_trait]
pub trait TradeRecordRepository: Send + Sync {
    /// 거래 기록 저장
    async fn save(&self, record: &TradeRecord) -> Result<(), RecordError>;

    /// 거래 기록 여러 개 일괄 저장
    async fn save_batch(&self, records: &[TradeRecord]) -> Result<(), RecordError>;

    /// ID로 거래 기록 조회
    async fn find_by_id(&self, id: i64) -> Result<Option<StoredTradeRecord>, RecordError>;

    /// 심볼로 거래 기록 조회
    async fn find_by_symbol(
        &self,
        symbol: &str,
        limit: Option<u64>,
    ) -> Result<Vec<StoredTradeRecord>, RecordError>;

    /// 거래소로 거래 기록 조회
    async fn find_by_exchange(
        &self,
        exchange: &str,
        limit: Option<u64>,
    ) -> Result<Vec<StoredTradeRecord>, RecordError>;

    /// 날짜 범위로 거래 기록 조회
    async fn find_by_date_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: Option<u64>,
    ) -> Result<Vec<StoredTradeRecord>, RecordError>;

    /// 모든 거래 기록 조회
    async fn find_all(&self, limit: Option<u64>) -> Result<Vec<StoredTradeRecord>, RecordError>;
}

/// 저장소에 저장된 거래 기록 (ID 포함)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTradeRecord {
    /// 데이터베이스 ID
    pub id: i64,
    /// 거래 기록 데이터
    #[serde(flatten)]
    pub record: TradeRecord,
}

/// SeaORM trade_record::Model을 StoredTradeRecord로 변환
impl TryFrom<super::entities::trade_record::Model> for StoredTradeRecord {
    type Error = RecordError;

    fn try_from(model: super::entities::trade_record::Model) -> Result<Self, Self::Error> {
        let executed_at = DateTime::parse_from_rfc3339(&model.executed_at)
            .map_err(|e| RecordError::Other(format!("Failed to parse executed_at: {}", e)))?
            .with_timezone(&Utc);

        let market_type =
            MarketType::from_str(&model.market_type).map_err(|e| RecordError::Other(e))?;

        let side = TradeSide::from_str(&model.side).map_err(|e| RecordError::Other(e))?;

        let trade_type =
            TradeType::from_str(&model.trade_type).map_err(|e| RecordError::Other(e))?;

        let record = TradeRecord {
            executed_at,
            exchange: model.exchange,
            symbol: model.symbol,
            market_type,
            side,
            trade_type,
            executed_price: model.executed_price,
            quantity: model.quantity,
            request_query_string: model.request_query_string,
            api_response: model.api_response,
            metadata: model.metadata,
            is_liquidation: model.is_liquidation,
        };

        Ok(StoredTradeRecord {
            id: model.id,
            record,
        })
    }
}

/// 포지션 기록 데이터 구조
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRecord {
    /// 포지션 UTC 시간
    pub executed_at: DateTime<Utc>,
    /// 봇 이름
    pub bot_name: String,
    /// 포지션 방향 (CARRY, REVERSE)
    pub carry: String,
    /// 포지션 액션 (OPEN, CLOSE)
    pub action: String,
    /// 코인 심볼
    pub symbol: String,
    /// 스팟 가격
    pub spot_price: f64,
    /// 선물 마크 가격
    pub futures_mark: f64,
    /// 매수 거래소 이름
    pub buy_exchange: String,
    /// 매도 거래소 이름
    pub sell_exchange: String,
}

/// 저장소에 저장된 포지션 기록 (ID 포함)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPositionRecord {
    /// 데이터베이스 ID
    pub id: i64,
    /// 포지션 기록 데이터
    #[serde(flatten)]
    pub record: PositionRecord,
}

/// SeaORM position_record::Model을 StoredPositionRecord로 변환
impl TryFrom<super::entities::position_record::Model> for StoredPositionRecord {
    type Error = RecordError;

    fn try_from(model: super::entities::position_record::Model) -> Result<Self, Self::Error> {
        let executed_at = DateTime::parse_from_rfc3339(&model.executed_at)
            .map_err(|e| RecordError::Other(format!("Failed to parse executed_at: {}", e)))?
            .with_timezone(&Utc);

        let record = PositionRecord {
            executed_at,
            bot_name: model.bot_name,
            carry: model.carry,
            action: model.action,
            symbol: model.symbol,
            spot_price: model.spot_price,
            futures_mark: model.futures_mark,
            buy_exchange: model.buy_exchange,
            sell_exchange: model.sell_exchange,
        };

        Ok(StoredPositionRecord {
            id: model.id,
            record,
        })
    }
}

/// 포지션 기록 저장소 인터페이스
/// 확장성을 위해 트레이트로 정의하여 나중에 다른 DB로 전환 가능
#[async_trait]
pub trait PositionRecordRepository: Send + Sync {
    /// 포지션 기록 저장
    async fn save(
        &self,
        bot_name: &str,
        carry: &str,  // "CARRY" or "REVERSE"
        action: &str, // "OPEN" or "CLOSE"
        symbol: &str,
        spot_price: f64,
        futures_mark: f64,
        buy_exchange: &str,
        sell_exchange: &str,
    ) -> Result<(), RecordError>;

    /// 모든 포지션 기록 조회
    async fn find_all(&self, limit: Option<u64>) -> Result<Vec<StoredPositionRecord>, RecordError>;
}

/// 기록 저장소 에러 타입
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Other error: {0}")]
    Other(String),
}
