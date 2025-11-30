use std::sync::Arc;
use std::sync::OnceLock;

use super::{
    PositionRecordRepository, SqlitePositionRecordRepository, SqliteTradeRecordRepository,
    TradeRecordRepository,
};

/// 전역 거래 기록 저장소
/// 애플리케이션 전체에서 하나의 Repository 인스턴스를 공유
static GLOBAL_REPOSITORY: OnceLock<Arc<dyn TradeRecordRepository + Send + Sync>> = OnceLock::new();

/// 전역 포지션 기록 저장소
static GLOBAL_POSITION_REPOSITORY: OnceLock<Arc<dyn PositionRecordRepository + Send + Sync>> =
    OnceLock::new();

/// 전역 Repository 초기화
pub async fn init_global_repository() -> Result<(), super::RecordError> {
    let repo = SqliteTradeRecordRepository::new().await?;
    GLOBAL_REPOSITORY
        .set(Arc::new(repo))
        .map_err(|_| super::RecordError::Other("Repository already initialized".to_string()))?;

    let position_repo = SqlitePositionRecordRepository::new().await?;
    GLOBAL_POSITION_REPOSITORY
        .set(Arc::new(position_repo))
        .map_err(|_| {
            super::RecordError::Other("Position repository already initialized".to_string())
        })?;

    Ok(())
}

/// 전역 Repository 가져오기
pub fn get_repository() -> Option<Arc<dyn TradeRecordRepository + Send + Sync>> {
    GLOBAL_REPOSITORY.get().cloned()
}

/// 전역 포지션 Repository 가져오기
pub fn get_position_repository() -> Option<Arc<dyn PositionRecordRepository + Send + Sync>> {
    GLOBAL_POSITION_REPOSITORY.get().cloned()
}

/// 거래 기록 저장 (전역 Repository 사용)
/// Repository가 초기화되지 않았으면 에러 없이 무시
pub async fn save_trade_record_safe(record: &super::TradeRecord) {
    if let Some(repo) = get_repository() {
        if let Err(e) = repo.save(record).await {
            tracing::warn!("Failed to save trade record: {}", e);
        }
    }
}

/// 포지션 기록 저장 (전역 Repository 사용)
/// Repository가 초기화되지 않았으면 에러 없이 무시
pub async fn save_position_record_safe(
    bot_name: &str,
    carry: &str,
    action: &str,
    symbol: &str,
    spot_price: f64,
    futures_mark: f64,
    buy_exchange: &str,
    sell_exchange: &str,
) {
    if let Some(repo) = get_position_repository() {
        if let Err(e) = repo
            .save(
                bot_name,
                carry,
                action,
                symbol,
                spot_price,
                futures_mark,
                buy_exchange,
                sell_exchange,
            )
            .await
        {
            tracing::warn!("Failed to save position record: {}", e);
        }
    }
}
