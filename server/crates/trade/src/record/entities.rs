/// 거래 기록 엔티티 모듈
pub mod trade_record {
    use sea_orm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "trade_records")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = true)]
        pub id: i64,

        /// 거래 UTC 시간 (ISO 8601 형식)
        #[sea_orm(column_type = "Text")]
        pub executed_at: String,

        /// 거래소 이름
        #[sea_orm(column_type = "Text")]
        pub exchange: String,

        /// 코인 이름/심볼
        #[sea_orm(column_type = "Text")]
        pub symbol: String,

        /// 선/현물 정보 (SPOT, FUTURES)
        #[sea_orm(column_type = "Text")]
        pub market_type: String,

        /// 거래 방향 (BUY, SELL)
        #[sea_orm(column_type = "Text")]
        pub side: String,

        /// 거래 유형 (MARKET, LIMIT, OTHER)
        #[sea_orm(column_type = "Text")]
        pub trade_type: String,

        /// 실행 가격 (NULL 가능)
        #[sea_orm(column_type = "Double", nullable)]
        pub executed_price: Option<f64>,

        /// 거래 수량
        #[sea_orm(column_type = "Double")]
        pub quantity: f64,

        /// 요청 쿼리 스트링 전문 (NULL 가능, TEXT)
        #[sea_orm(column_type = "Text", nullable)]
        pub request_query_string: Option<String>,

        /// API 요청 응답 전문 (NULL 가능, TEXT)
        #[sea_orm(column_type = "Text", nullable)]
        pub api_response: Option<String>,

        /// 추가 메타데이터 (NULL 가능, TEXT, JSON)
        #[sea_orm(column_type = "Text", nullable)]
        pub metadata: Option<String>,

        /// 청산 실행 기록 여부
        #[sea_orm(column_type = "Boolean")]
        pub is_liquidation: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// 포지션 기록 엔티티 모듈
pub mod position_record {
    use sea_orm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "position_records")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = true)]
        pub id: i64,

        /// 포지션 UTC 시간 (ISO 8601 형식)
        #[sea_orm(column_type = "Text")]
        pub executed_at: String,

        /// 봇 이름 (예: "intra_basis", "cross_basis")
        #[sea_orm(column_type = "Text")]
        pub bot_name: String,

        /// 포지션 방향 (CARRY, REVERSE)
        #[sea_orm(column_type = "Text")]
        pub carry: String, // "CARRY" or "REVERSE"

        /// 포지션 액션 (OPEN, CLOSE)
        #[sea_orm(column_type = "Text")]
        pub action: String, // "OPEN" or "CLOSE"

        /// 코인 심볼
        #[sea_orm(column_type = "Text")]
        pub symbol: String,

        /// 스팟 가격
        #[sea_orm(column_type = "Double")]
        pub spot_price: f64,

        /// 선물 마크 가격
        #[sea_orm(column_type = "Double")]
        pub futures_mark: f64,

        /// 매수 거래소 이름
        #[sea_orm(column_type = "Text")]
        pub buy_exchange: String,

        /// 매도 거래소 이름
        #[sea_orm(column_type = "Text")]
        pub sell_exchange: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
