use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, Database, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Schema, Set,
};
use std::convert::TryInto;
use std::env;
use std::path::PathBuf;
use tracing::info;

use super::entities::position_record;
use super::entities::trade_record;
use super::{
    PositionRecordRepository, RecordError, StoredPositionRecord, StoredTradeRecord, TradeRecord,
    TradeRecordRepository,
};

/// SQLite 기반 거래 기록 저장소
pub struct SqliteTradeRecordRepository {
    db: DatabaseConnection,
}

impl SqliteTradeRecordRepository {
    /// 새로운 SQLite 저장소 인스턴스 생성
    /// DB 파일 경로는 환경 변수 DB_PATH로 지정 가능 (기본값: "trade_records.db")
    pub async fn new() -> Result<Self, RecordError> {
        let db_path = env::var("DB_PATH").unwrap_or_else(|_| "trade_records.db".to_string());

        // 절대 경로 또는 상대 경로 처리
        let mut path = PathBuf::from(&db_path);
        if !path.is_absolute() {
            // 상대 경로인 경우 현재 디렉토리 기준
            if let Ok(current_dir) = env::current_dir() {
                path = current_dir.join(&db_path);
            }
        }

        // 디렉토리가 없으면 생성
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RecordError::Other(format!("Failed to create DB directory: {}", e)))?;
        }

        let db_url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        info!("Connecting to SQLite database: {}", db_url);

        let db = Database::connect(&db_url)
            .await
            .map_err(|e| RecordError::Database(e))?;

        // SeaORM SchemaBuilder를 사용하여 테이블 및 인덱스 생성
        let backend = db.get_database_backend();
        let schema = Schema::new(backend);

        // 테이블 생성 (IF NOT EXISTS)
        let mut create_table_stmt = schema.create_table_from_entity(trade_record::Entity);
        create_table_stmt.if_not_exists();

        db.execute(backend.build(&create_table_stmt))
            .await
            .map_err(|e| RecordError::Database(e))?;

        // 인덱스 생성 - SeaORM 1.1에서는 sea_orm::sea_query::Index 사용
        use sea_orm::sea_query::Index;

        let mut executed_at_idx = Index::create()
            .name("idx_trade_records_executed_at")
            .table(trade_record::Entity)
            .col(trade_record::Column::ExecutedAt)
            .to_owned();
        executed_at_idx.if_not_exists();

        let mut exchange_idx = Index::create()
            .name("idx_trade_records_exchange")
            .table(trade_record::Entity)
            .col(trade_record::Column::Exchange)
            .to_owned();
        exchange_idx.if_not_exists();

        let mut symbol_idx = Index::create()
            .name("idx_trade_records_symbol")
            .table(trade_record::Entity)
            .col(trade_record::Column::Symbol)
            .to_owned();
        symbol_idx.if_not_exists();

        // 인덱스 생성 실행
        if let Err(e) = db.execute(backend.build(&executed_at_idx)).await {
            tracing::debug!(
                "Index idx_trade_records_executed_at creation skipped: {}",
                e
            );
        }
        if let Err(e) = db.execute(backend.build(&exchange_idx)).await {
            tracing::debug!("Index idx_trade_records_exchange creation skipped: {}", e);
        }
        if let Err(e) = db.execute(backend.build(&symbol_idx)).await {
            tracing::debug!("Index idx_trade_records_symbol creation skipped: {}", e);
        }

        info!("Trade records table initialized");

        Ok(Self { db })
    }
}

#[async_trait]
impl TradeRecordRepository for SqliteTradeRecordRepository {
    async fn save(&self, record: &TradeRecord) -> Result<(), RecordError> {
        let model = trade_record::ActiveModel {
            executed_at: Set(record.executed_at.to_rfc3339()),
            exchange: Set(record.exchange.clone()),
            symbol: Set(record.symbol.clone()),
            market_type: Set(record.market_type.to_string()),
            side: Set(record.side.to_string()),
            trade_type: Set(record.trade_type.to_string()),
            executed_price: Set(record.executed_price),
            quantity: Set(record.quantity),
            request_query_string: Set(record.request_query_string.clone()),
            api_response: Set(record.api_response.clone()),
            metadata: Set(record.metadata.clone()),
            is_liquidation: Set(record.is_liquidation),
            ..Default::default()
        };

        trade_record::Entity::insert(model)
            .exec(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        Ok(())
    }

    async fn save_batch(&self, records: &[TradeRecord]) -> Result<(), RecordError> {
        if records.is_empty() {
            return Ok(());
        }

        let models: Vec<trade_record::ActiveModel> = records
            .iter()
            .map(|record| trade_record::ActiveModel {
                executed_at: Set(record.executed_at.to_rfc3339()),
                exchange: Set(record.exchange.clone()),
                symbol: Set(record.symbol.clone()),
                market_type: Set(record.market_type.to_string()),
                side: Set(record.side.to_string()),
                trade_type: Set(record.trade_type.to_string()),
                executed_price: Set(record.executed_price),
                quantity: Set(record.quantity),
                request_query_string: Set(record.request_query_string.clone()),
                api_response: Set(record.api_response.clone()),
                metadata: Set(record.metadata.clone()),
                is_liquidation: Set(record.is_liquidation),
                ..Default::default()
            })
            .collect();

        trade_record::Entity::insert_many(models)
            .exec(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        Ok(())
    }

    async fn find_by_id(&self, id: i64) -> Result<Option<StoredTradeRecord>, RecordError> {
        let model = trade_record::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        match model {
            Some(m) => Ok(Some(m.try_into()?)),
            None => Ok(None),
        }
    }

    async fn find_by_symbol(
        &self,
        symbol: &str,
        limit: Option<u64>,
    ) -> Result<Vec<StoredTradeRecord>, RecordError> {
        let mut query = trade_record::Entity::find()
            .filter(trade_record::Column::Symbol.eq(symbol))
            .order_by_desc(trade_record::Column::ExecutedAt);

        if let Some(limit_val) = limit {
            query = query.limit(limit_val);
        }

        let models = query
            .all(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        models.into_iter().map(|m| m.try_into()).collect()
    }

    async fn find_by_exchange(
        &self,
        exchange: &str,
        limit: Option<u64>,
    ) -> Result<Vec<StoredTradeRecord>, RecordError> {
        let mut query = trade_record::Entity::find()
            .filter(trade_record::Column::Exchange.eq(exchange))
            .order_by_desc(trade_record::Column::ExecutedAt);

        if let Some(limit_val) = limit {
            query = query.limit(limit_val);
        }

        let models = query
            .all(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        models.into_iter().map(|m| m.try_into()).collect()
    }

    async fn find_by_date_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: Option<u64>,
    ) -> Result<Vec<StoredTradeRecord>, RecordError> {
        let start_str = start.to_rfc3339();
        let end_str = end.to_rfc3339();

        let mut query = trade_record::Entity::find()
            .filter(trade_record::Column::ExecutedAt.gte(start_str))
            .filter(trade_record::Column::ExecutedAt.lte(end_str))
            .order_by_desc(trade_record::Column::ExecutedAt);

        if let Some(limit_val) = limit {
            query = query.limit(limit_val);
        }

        let models = query
            .all(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        models.into_iter().map(|m| m.try_into()).collect()
    }

    async fn find_all(&self, limit: Option<u64>) -> Result<Vec<StoredTradeRecord>, RecordError> {
        let mut query =
            trade_record::Entity::find().order_by_desc(trade_record::Column::ExecutedAt);

        if let Some(limit_val) = limit {
            query = query.limit(limit_val);
        }

        let models = query
            .all(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        models.into_iter().map(|m| m.try_into()).collect()
    }
}

// ============================================================================
// 포지션 기록 저장소
// ============================================================================

/// SQLite 기반 포지션 기록 저장소
pub struct SqlitePositionRecordRepository {
    db: DatabaseConnection,
}

impl SqlitePositionRecordRepository {
    /// 새로운 SQLite 저장소 인스턴스 생성
    /// DB 파일 경로는 환경 변수 DB_PATH로 지정 가능 (기본값: "trade_records.db")
    pub async fn new() -> Result<Self, RecordError> {
        let db_path = env::var("DB_PATH").unwrap_or_else(|_| "trade_records.db".to_string());

        let mut path = PathBuf::from(&db_path);
        if !path.is_absolute() {
            if let Ok(current_dir) = env::current_dir() {
                path = current_dir.join(&db_path);
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| RecordError::Other(format!("Failed to create DB directory: {}", e)))?;
        }

        let db_url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        info!(
            "Connecting to SQLite database for position records: {}",
            db_url
        );

        let db = Database::connect(&db_url)
            .await
            .map_err(|e| RecordError::Database(e))?;

        // SeaORM SchemaBuilder를 사용하여 테이블 생성
        let backend = db.get_database_backend();
        let schema = Schema::new(backend);

        // 테이블 생성 (IF NOT EXISTS)
        let mut create_table_stmt = schema.create_table_from_entity(position_record::Entity);
        create_table_stmt.if_not_exists();

        db.execute(backend.build(&create_table_stmt))
            .await
            .map_err(|e| RecordError::Database(e))?;

        info!("Position records table initialized");

        Ok(Self { db })
    }
}

#[async_trait]
impl PositionRecordRepository for SqlitePositionRecordRepository {
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
    ) -> Result<(), RecordError> {
        let model = position_record::ActiveModel {
            executed_at: Set(Utc::now().to_rfc3339()),
            bot_name: Set(bot_name.to_string()),
            carry: Set(carry.to_string()),
            action: Set(action.to_string()),
            symbol: Set(symbol.to_string()),
            spot_price: Set(spot_price),
            futures_mark: Set(futures_mark),
            buy_exchange: Set(buy_exchange.to_string()),
            sell_exchange: Set(sell_exchange.to_string()),
            ..Default::default()
        };

        position_record::Entity::insert(model)
            .exec(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        Ok(())
    }

    async fn find_all(&self, limit: Option<u64>) -> Result<Vec<StoredPositionRecord>, RecordError> {
        let mut query =
            position_record::Entity::find().order_by_desc(position_record::Column::ExecutedAt);

        if let Some(limit_val) = limit {
            query = query.limit(limit_val);
        }

        let models = query
            .all(&self.db)
            .await
            .map_err(|e| RecordError::Database(e))?;

        models.into_iter().map(|m| m.try_into()).collect()
    }
}
