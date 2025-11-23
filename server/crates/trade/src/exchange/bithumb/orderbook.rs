use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use interface::{ExchangeId, OrderBook, OrderBookEntry};

use super::super::{ExchangeError, OrderBookExchange};
use super::BASE_URL;

/// Bithumb Orderbook 클라이언트 (공개 API, 인증 불필요)
#[derive(Clone)]
pub struct BithumbOrderBookClient {
    http: reqwest::Client,
}

impl BithumbOrderBookClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// 심볼을 Bithumb 형식으로 변환
    /// 예: "BTC-KRW" -> "BTC_KRW"
    fn normalize_symbol(&self, symbol: &str) -> String {
        symbol.replace("-", "_").to_uppercase()
    }
}

#[derive(Debug, Deserialize)]
struct BithumbOrderBookResponse {
    status: String,
    data: BithumbOrderBookData,
}

#[derive(Debug, Deserialize)]
struct BithumbOrderBookData {
    bids: Vec<BithumbOrderBookEntry>,
    asks: Vec<BithumbOrderBookEntry>,
}

#[derive(Debug, Deserialize)]
struct BithumbOrderBookEntry {
    price: String,
    quantity: String,
}

#[async_trait]
impl OrderBookExchange for BithumbOrderBookClient {
    fn id(&self) -> ExchangeId {
        ExchangeId::Bithumb
    }

    async fn fetch_orderbook(&self, symbol: &str) -> Result<OrderBook, ExchangeError> {
        // Bithumb 공개 API: GET /public/orderbook/{order_currency}_{payment_currency}
        // 예: /public/orderbook/BTC_KRW
        let normalized_symbol = self.normalize_symbol(symbol);
        let endpoint = format!("/public/orderbook/{}", normalized_symbol);
        let url = format!("{BASE_URL}{}", endpoint);

        let response = self.http.get(&url).send().await?;

        let status = response.status();
        let response_text = response.text().await?;

        if status != 200 {
            return Err(ExchangeError::Other(format!(
                "Bithumb API HTTP error: status {}, response: {}",
                status,
                response_text.chars().take(200).collect::<String>()
            )));
        }

        let orderbook_response: BithumbOrderBookResponse = serde_json::from_str(&response_text)
            .map_err(|e| {
                ExchangeError::Other(format!(
                    "Failed to parse Bithumb orderbook response: {}, response: {}",
                    e,
                    response_text.chars().take(200).collect::<String>()
                ))
            })?;

        if orderbook_response.status != "0000" {
            return Err(ExchangeError::Other(format!(
                "Bithumb API error: status {}",
                orderbook_response.status
            )));
        }

        let now = Utc::now();

        // bids: 매수 주문 (가격 높은 순으로 정렬)
        let mut bids: Vec<OrderBookEntry> = orderbook_response
            .data
            .bids
            .into_iter()
            .filter_map(|entry| {
                let price: f64 = entry.price.parse().ok()?;
                let quantity: f64 = entry.quantity.parse().ok()?;
                if price > 0.0 && quantity > 0.0 {
                    Some(OrderBookEntry { price, quantity })
                } else {
                    None
                }
            })
            .collect();

        // asks: 매도 주문 (가격 낮은 순으로 정렬)
        let mut asks: Vec<OrderBookEntry> = orderbook_response
            .data
            .asks
            .into_iter()
            .filter_map(|entry| {
                let price: f64 = entry.price.parse().ok()?;
                let quantity: f64 = entry.quantity.parse().ok()?;
                if price > 0.0 && quantity > 0.0 {
                    Some(OrderBookEntry { price, quantity })
                } else {
                    None
                }
            })
            .collect();

        // bids는 가격 내림차순, asks는 가격 오름차순으로 정렬
        bids.sort_by(|a, b| {
            b.price
                .partial_cmp(&a.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        asks.sort_by(|a, b| {
            a.price
                .partial_cmp(&b.price)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(OrderBook {
            exchange: ExchangeId::Bithumb,
            symbol: symbol.to_string(),
            bids,
            asks,
            updated_at: now,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exchange::ExchangeError;

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

    #[tokio::test]
    async fn test_fetch_orderbook_bithumb() {
        let client = BithumbOrderBookClient::new();
        let result = client.fetch_orderbook("BTC-KRW").await;

        match result {
            Ok(orderbook) => {
                assert_eq!(orderbook.exchange, ExchangeId::Bithumb);
                assert_eq!(orderbook.symbol, "BTC-KRW");
                assert!(!orderbook.bids.is_empty(), "bids should not be empty");
                assert!(!orderbook.asks.is_empty(), "asks should not be empty");

                // bids는 가격 내림차순인지 확인
                for i in 1..orderbook.bids.len() {
                    assert!(
                        orderbook.bids[i - 1].price >= orderbook.bids[i].price,
                        "bids should be sorted in descending order by price"
                    );
                }

                // asks는 가격 오름차순인지 확인
                for i in 1..orderbook.asks.len() {
                    assert!(
                        orderbook.asks[i - 1].price <= orderbook.asks[i].price,
                        "asks should be sorted in ascending order by price"
                    );
                }

                // 모든 entry가 유효한지 확인
                for bid in &orderbook.bids {
                    assert!(bid.price > 0.0, "bid price should be positive");
                    assert!(bid.quantity > 0.0, "bid quantity should be positive");
                }

                for ask in &orderbook.asks {
                    assert!(ask.price > 0.0, "ask price should be positive");
                    assert!(ask.quantity > 0.0, "ask quantity should be positive");
                }

                println!("\nOrderbook for {}:", orderbook.symbol);
                println!(
                    "  Best bid: {} @ {}",
                    orderbook.bids[0].price, orderbook.bids[0].quantity
                );
                println!(
                    "  Best ask: {} @ {}",
                    orderbook.asks[0].price, orderbook.asks[0].quantity
                );
                println!(
                    "  Spread: {}",
                    orderbook.asks[0].price - orderbook.bids[0].price
                );
            }
            Err(e) => {
                handle_api_error(&e);
            }
        }
    }
}
