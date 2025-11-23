use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::time::sleep;
use tracing::{info, warn};

use crate::exchange::{PerpExchange, SpotExchange};
use crate::exchange_rate::fetch_all_exchange_rates;
use interface::{ExchangeId, PerpData, PerpSnapshot, SpotData, SpotSnapshot, UnifiedSnapshot};
use crate::server::AppState;

pub fn start_collect_loop(
    perp_exchanges: Vec<Arc<dyn PerpExchange>>,
    spot_exchanges: Vec<Arc<dyn SpotExchange>>,
    state: Arc<AppState>,
    interval: Duration,
) {
    tokio::spawn(async move {
        info!(
            "데이터 수집 루프 시작: {}개 선물 거래소, {}개 현물 거래소, {}초 간격",
            perp_exchanges.len(),
            spot_exchanges.len(),
            interval.as_secs()
        );
        loop {
            // 선물 데이터 수집
            let mut all_perp: Vec<PerpSnapshot> = Vec::new();
            for ex in &perp_exchanges {
                match ex.fetch_all().await {
                    Ok(mut v) => all_perp.append(&mut v),
                    Err(e) => {
                        warn!("perp fetch error from {:?}: {:?}", ex.id(), e);
                    }
                }
            }

            // 정렬: OI 기준 내림차순
            all_perp.sort_by(|a, b| {
                b.oi_usd
                    .partial_cmp(&a.oi_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let perp_count = all_perp.len();
            let perp_clone = all_perp.clone();
            {
                let mut guard = state.perp_snapshots.write().await;
                *guard = all_perp;
            }

            // 현물 데이터 수집
            let mut all_spot: Vec<SpotSnapshot> = Vec::new();
            for ex in &spot_exchanges {
                match ex.fetch_all().await {
                    Ok(mut v) => all_spot.append(&mut v),
                    Err(e) => {
                        warn!("spot fetch error from {:?}: {:?}", ex.id(), e);
                    }
                }
            }

            // 정렬: 거래량 기준 내림차순
            all_spot.sort_by(|a, b| {
                b.vol_24h_usd
                    .partial_cmp(&a.vol_24h_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let spot_count = all_spot.len();
            let spot_clone = all_spot.clone();
            {
                let mut guard = state.spot_snapshots.write().await;
                *guard = all_spot;
            }

            // 환율 정보 가져오기
            let exchange_rates = fetch_all_exchange_rates().await;

            // 통합 스냅샷 생성
            let mut unified_map: HashMap<(ExchangeId, String), UnifiedSnapshot> = HashMap::new();

            // 선물 데이터 추가
            for perp in perp_clone {
                let key = (perp.exchange, perp.symbol.clone());
                let unified = unified_map.entry(key).or_insert_with(|| UnifiedSnapshot {
                    exchange: perp.exchange,
                    symbol: perp.symbol.clone(),
                    currency: perp.currency,
                    perp: None,
                    spot: None,
                    exchange_rates: exchange_rates.clone(),
                    updated_at: perp.updated_at,
                });
                unified.perp = Some(PerpData {
                    currency: perp.currency,
                    mark_price: perp.mark_price,
                    oi_usd: perp.oi_usd,
                    vol_24h_usd: perp.vol_24h_usd,
                    funding_rate: perp.funding_rate,
                    next_funding_time: perp.next_funding_time,
                });
                // currency와 updated_at은 더 최신 것으로 업데이트
                unified.currency = perp.currency;
                unified.exchange_rates = exchange_rates.clone();
                if perp.updated_at > unified.updated_at {
                    unified.updated_at = perp.updated_at;
                }
            }

            // 현물 데이터 추가
            for spot in spot_clone {
                let key = (spot.exchange, spot.symbol.clone());
                let unified = unified_map.entry(key).or_insert_with(|| UnifiedSnapshot {
                    exchange: spot.exchange,
                    symbol: spot.symbol.clone(),
                    currency: spot.currency,
                    perp: None,
                    spot: None,
                    exchange_rates: exchange_rates.clone(),
                    updated_at: spot.updated_at,
                });
                unified.spot = Some(SpotData {
                    currency: spot.currency,
                    price: spot.price,
                    vol_24h_usd: spot.vol_24h_usd,
                });
                // currency와 updated_at은 더 최신 것으로 업데이트 (현물이 없으면 현물 currency 사용)
                if unified.perp.is_none() {
                    unified.currency = spot.currency;
                }
                unified.exchange_rates = exchange_rates.clone();
                if spot.updated_at > unified.updated_at {
                    unified.updated_at = spot.updated_at;
                }
            }

            let unified_snapshots: Vec<UnifiedSnapshot> = unified_map.into_values().collect();
            let unified_count = unified_snapshots.len();
            {
                let mut guard = state.unified_snapshots.write().await;
                *guard = unified_snapshots;
            }

            info!(
                "데이터 수집 완료: {}개 선물 스냅샷, {}개 현물 스냅샷, {}개 통합 스냅샷",
                perp_count, spot_count, unified_count
            );

            sleep(interval).await;
        }
    });
}
