use color_eyre::eyre;
use exchanges::{AssetExchange, BinanceClient, BithumbClient};
use tracing::{error, info, warn};

use crate::trader::{
    binance::BinanceTrader, bithumb::BithumbTrader, SpotExchangeTrader,
};

/// Binance의 모든 자산을 USDT로 강제 청산
pub async fn liquidate_binance() -> eyre::Result<()> {
    info!("=== Binance 강제 청산 시작 ===");

    let trader = BinanceTrader::new()
        .map_err(|e| eyre::eyre!("BinanceTrader 생성 실패: {}", e))?;

    // ExchangeInfo 로드 (LOT_SIZE 필터 필요)
    trader
        .load_spot_exchange_info()
        .await
        .map_err(|e| eyre::eyre!("ExchangeInfo 로드 실패: {}", e))?;

    // 모든 자산 조회
    let client = BinanceClient::with_credentials()?;
    let assets = client
        .fetch_assets()
        .await
        .map_err(|e| eyre::eyre!("자산 조회 실패: {}", e))?;

    info!("총 {}개의 자산을 조회했습니다.", assets.len());

    // USDT가 아닌 자산만 필터링
    let non_usdt_assets: Vec<_> = assets
        .iter()
        .filter(|a| {
            let currency = &a.currency;
            currency != "USDT" && a.available > 0.0
        })
        .collect();

    if non_usdt_assets.is_empty() {
        info!("USDT로 변환할 자산이 없습니다.");
        return Ok(());
    }

    info!("{}개의 자산을 USDT로 변환합니다.", non_usdt_assets.len());

    // 각 자산을 USDT로 변환
    for asset in non_usdt_assets {
        let currency = &asset.currency;
        let available = asset.available;

        // 심볼 생성 (예: BTC -> BTCUSDT)
        let symbol = format!("{}USDT", currency);

        info!("{} {} -> USDT 변환 시도...", available, currency);

        // 수량 클램프
        let qty = trader.clamp_spot_quantity(&symbol, available);
        if qty <= 0.0 {
            warn!(
                "{}의 수량이 너무 작아서 거래할 수 없습니다. (available: {})",
                currency, available
            );
            continue;
        }

        // 시장가 매도 주문
        match trader.sell_spot(&symbol, qty).await {
            Ok(order) => {
                info!(
                    "{} {} 매도 성공: order_id={:?}, executed_qty={:?}",
                    currency,
                    qty,
                    order.order_id,
                    order.executed_qty
                );
            }
            Err(e) => {
                error!("{} {} 매도 실패: {}", currency, qty, e);
                // 에러가 발생해도 다음 자산 계속 처리
            }
        }

        // API 레이트 리밋 방지를 위한 짧은 대기
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    info!("=== Binance 강제 청산 완료 ===");
    Ok(())
}

/// Bithumb의 모든 자산을 KRW로 강제 청산
pub async fn liquidate_bithumb() -> eyre::Result<()> {
    info!("=== Bithumb 강제 청산 시작 ===");

    let trader = BithumbTrader::new()
        .map_err(|e| eyre::eyre!("BithumbTrader 생성 실패: {}", e))?;

    // 모든 자산 조회
    let client = BithumbClient::with_credentials()?;
    let assets = client
        .fetch_assets()
        .await
        .map_err(|e| eyre::eyre!("자산 조회 실패: {}", e))?;

    info!("총 {}개의 자산을 조회했습니다.", assets.len());

    // KRW가 아닌 자산만 필터링
    let non_krw_assets: Vec<_> = assets
        .iter()
        .filter(|a| {
            let currency = &a.currency;
            currency != "KRW" && a.available > 0.0
        })
        .collect();

    if non_krw_assets.is_empty() {
        info!("KRW로 변환할 자산이 없습니다.");
        return Ok(());
    }

    info!("{}개의 자산을 KRW로 변환합니다.", non_krw_assets.len());

    // 각 자산을 KRW로 변환
    for asset in non_krw_assets {
        let currency = &asset.currency;
        let available = asset.available;

        // 심볼 생성 (예: BTC -> BTC-KRW)
        let symbol = format!("{}-KRW", currency);

        info!("{} {} -> KRW 변환 시도...", available, currency);

        // 수량 클램프
        let qty = trader.clamp_spot_quantity(&symbol, available);
        if qty <= 0.0 {
            warn!(
                "{}의 수량이 너무 작아서 거래할 수 없습니다. (available: {})",
                currency, available
            );
            continue;
        }

        // 시장가 매도 주문
        match trader.sell_spot(&symbol, qty).await {
            Ok(order) => {
                info!(
                    "{} {} 매도 성공: order_id={:?}, executed_qty={:?}",
                    currency,
                    qty,
                    order.order_id,
                    order.executed_qty
                );
            }
            Err(e) => {
                error!("{} {} 매도 실패: {}", currency, qty, e);
                // 에러가 발생해도 다음 자산 계속 처리
            }
        }

        // API 레이트 리밋 방지를 위한 짧은 대기
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    info!("=== Bithumb 강제 청산 완료 ===");
    Ok(())
}

/// 모든 거래소의 자산을 강제 청산
pub async fn liquidate_all() -> eyre::Result<()> {
    info!("=== 전체 강제 청산 시작 ===");

    // Binance 청산
    match liquidate_binance().await {
        Ok(_) => info!("Binance 청산 완료"),
        Err(e) => error!("Binance 청산 실패: {}", e),
    }

    // Bithumb 청산
    match liquidate_bithumb().await {
        Ok(_) => info!("Bithumb 청산 완료"),
        Err(e) => error!("Bithumb 청산 실패: {}", e),
    }

    info!("=== 전체 강제 청산 완료 ===");
    Ok(())
}

