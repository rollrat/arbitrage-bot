use chrono::Utc;
use serde_json;
use std::str::FromStr;

use super::{MarketType, TradeRecord, TradeSide, TradeType};
use crate::trader::OrderResponse;

/// OrderResponse를 TradeRecord로 변환하는 헬퍼 함수
/// 거래 실행 후 호출하여 기록을 저장할 수 있습니다.
pub fn create_trade_record_from_order(
    exchange: String,
    symbol: String,
    market_type: MarketType,
    side: TradeSide,
    trade_type: TradeType,
    quantity: f64,
    request_query_string: Option<String>,
    order_response: &OrderResponse,
    is_liquidation: bool,
) -> TradeRecord {
    // OrderResponse에서 실행 가격 추출 시도
    let executed_price = extract_price_from_order_response(order_response);

    // OrderResponse를 JSON 문자열로 변환
    let api_response = serde_json::to_string(order_response).ok();

    TradeRecord {
        executed_at: Utc::now(),
        exchange,
        symbol,
        market_type,
        side,
        trade_type,
        executed_price,
        quantity,
        request_query_string,
        api_response,
        metadata: None,
        is_liquidation,
    }
}

/// Spot 주문 기록 저장 (편의 함수)
pub async fn save_trade_record_spot_order(
    exchange: &str,
    symbol: &str,
    side: &str,
    quantity: f64,
    query_string: &str,
    order_response: &OrderResponse,
    is_liquidation: bool,
) {
    use super::global::save_trade_record_safe;

    let trade_side = match TradeSide::from_str(side) {
        Ok(s) => s,
        Err(_) => return, // 잘못된 side는 무시
    };

    let record = create_trade_record_from_order(
        exchange.to_string(),
        symbol.to_string(),
        MarketType::Spot,
        trade_side,
        TradeType::Market,
        quantity,
        Some(query_string.to_string()),
        order_response,
        is_liquidation,
    );

    save_trade_record_safe(&record).await;
}

/// Futures 주문 기록 저장 (편의 함수)
pub async fn save_trade_record_futures_order(
    exchange: &str,
    symbol: &str,
    side: &str,
    quantity: f64,
    query_string: &str,
    order_response: &OrderResponse,
    _reduce_only: bool,
    is_liquidation: bool,
) {
    use super::global::save_trade_record_safe;

    let trade_side = match TradeSide::from_str(side) {
        Ok(s) => s,
        Err(_) => return, // 잘못된 side는 무시
    };

    let record = create_trade_record_from_order(
        exchange.to_string(),
        symbol.to_string(),
        MarketType::Futures,
        trade_side,
        TradeType::Market,
        quantity,
        Some(query_string.to_string()),
        order_response,
        is_liquidation,
    );

    save_trade_record_safe(&record).await;
}

/// Bithumb 주문 기록 저장 (편의 함수)
pub async fn save_trade_record_bithumb_order(
    exchange: &str,
    symbol: &str,
    endpoint: &str,
    quantity: f64,
    params: &str,
    order_response: &OrderResponse,
    is_liquidation: bool,
) {
    use super::global::save_trade_record_safe;

    let trade_side = if endpoint.contains("buy") {
        TradeSide::Buy
    } else if endpoint.contains("sell") {
        TradeSide::Sell
    } else {
        return; // 잘못된 endpoint는 무시
    };

    let record = create_trade_record_from_order(
        exchange.to_string(),
        symbol.to_string(),
        MarketType::Spot,
        trade_side,
        TradeType::Market,
        quantity,
        Some(params.to_string()),
        order_response,
        is_liquidation,
    );

    save_trade_record_safe(&record).await;
}

/// OrderResponse에서 가격 정보를 추출 (가능한 경우)
fn extract_price_from_order_response(order_response: &OrderResponse) -> Option<f64> {
    // 1. fills 배열에서 가격 추출 (Binance 시장가 주문의 경우)
    if let Some(fills) = order_response.extra.get("fills").and_then(|v| v.as_array()) {
        if !fills.is_empty() {
            // 첫 번째 fill의 price 사용
            if let Some(price_str) = fills[0].get("price").and_then(|v| v.as_str()) {
                if let Ok(price) = price_str.parse::<f64>() {
                    if price > 0.0 {
                        return Some(price);
                    }
                }
            }
        }
    }

    // 2. cummulativeQuoteQty와 executedQty로 평균 가격 계산 (Binance)
    if let Some(cum_qty_str) = order_response
        .extra
        .get("cummulativeQuoteQty")
        .and_then(|v| v.as_str())
        .or_else(|| {
            // 누적 거래 금액 필드명 변형들 시도
            order_response
                .extra
                .get("cumulativeQuoteQty")
                .and_then(|v| v.as_str())
        })
        .or_else(|| {
            order_response
                .extra
                .get("cumQuote")
                .and_then(|v| v.as_str())
        })
    {
        if let Some(executed_qty_str) = order_response.executed_qty.as_ref() {
            if let (Ok(cum_quote), Ok(executed_qty)) =
                (cum_qty_str.parse::<f64>(), executed_qty_str.parse::<f64>())
            {
                if executed_qty > 0.0 && cum_quote > 0.0 {
                    return Some(cum_quote / executed_qty);
                }
            }
        }
    }

    // 3. extra 필드에서 직접 가격 정보 찾기
    if let Some(price_str) = order_response.extra.get("price").and_then(|v| v.as_str()) {
        if let Ok(price) = price_str.parse::<f64>() {
            if price > 0.0 {
                return Some(price);
            }
        }
    }

    // 4. avgPrice 필드 확인
    if let Some(avg_price_str) = order_response
        .extra
        .get("avgPrice")
        .and_then(|v| v.as_str())
    {
        if let Ok(price) = avg_price_str.parse::<f64>() {
            if price > 0.0 {
                return Some(price);
            }
        }
    }

    None
}

/// 메타데이터를 JSON 문자열로 변환하여 추가
pub fn add_metadata(record: &mut TradeRecord, metadata: serde_json::Value) {
    record.metadata = serde_json::to_string(&metadata).ok();
}

// ============================================================================
// 포지션 기록 관련 헬퍼 함수
// ============================================================================

use super::global::save_position_record_safe;

/// 포지션 기록 저장 (position_records 테이블 사용)
/// spot_price, futures_mark, 봇 이름, carry/reverse, open/close, symbol, buy/sell 거래소만 기록
/// 내부에서 거래소를 자동으로 결정합니다
pub async fn save_position_record(
    bot_name: &str,
    carry: &str,  // "CARRY" or "REVERSE"
    action: &str, // "OPEN" or "CLOSE"
    symbol: &str,
    spot_price: f64,
    futures_mark: f64,
    exchange_name: &str, // "binance", "bithumb", "bybit" 등
) {
    // carry를 소문자로 변환
    let carry_lower = carry.to_lowercase();
    let (buy_exchange, sell_exchange) =
        determine_exchanges_for_intra_basis(exchange_name, &carry_lower, action);

    save_position_record_safe(
        bot_name,
        carry,
        action,
        symbol,
        spot_price,
        futures_mark,
        &buy_exchange,
        &sell_exchange,
    )
    .await;
}

/// 포지션 열기/닫기 시 buy_exchange, sell_exchange 결정
/// intra_basis의 경우 같은 거래소 내에서 스팟과 선물을 거래
/// exchange_name: "binance", "bithumb", "bybit" 등 거래소 이름
pub fn determine_exchanges_for_intra_basis(
    exchange_name: &str, // "binance", "bithumb", "bybit" 등
    carry: &str,         // "carry" or "reverse"
    action: &str,        // "OPEN" or "CLOSE"
) -> (String, String) {
    let spot_exchange = format!("{}_spot", exchange_name);
    let futures_exchange = format!("{}_futures", exchange_name);

    match (carry, action) {
        // CARRY open: 스팟 BUY, 선물 SELL
        ("carry", "OPEN") => (spot_exchange.clone(), futures_exchange.clone()),
        // CARRY close: 스팟 SELL, 선물 BUY
        ("carry", "CLOSE") => (futures_exchange.clone(), spot_exchange.clone()),
        // REVERSE open: 스팟 SELL, 선물 BUY
        ("reverse", "OPEN") => (futures_exchange.clone(), spot_exchange.clone()),
        // REVERSE close: 스팟 BUY, 선물 SELL
        ("reverse", "CLOSE") => (spot_exchange.clone(), futures_exchange.clone()),
        _ => ("unknown".to_string(), "unknown".to_string()),
    }
}
