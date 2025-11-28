use color_eyre::eyre;
use tracing::info;

use exchanges::{AssetExchange, BinanceClient, BithumbClient};
use interface::{SpotAsset, UnifiedSnapshot};

const ORACLE_SERVER_URL: &str = "http://localhost:12090";

pub async fn fetch_unified_snapshots() -> eyre::Result<Vec<UnifiedSnapshot>> {
    let url = format!("{}/unified-snapshots", ORACLE_SERVER_URL);
    let response = reqwest::get(&url).await?;

    if !response.status().is_success() {
        return Err(eyre::eyre!("서버 응답 오류: {}", response.status()));
    }

    let snapshots: Vec<UnifiedSnapshot> = response.json().await?;
    Ok(snapshots)
}

pub fn print_unified_snapshots(snapshots: &[UnifiedSnapshot]) {
    info!("=== Unified Snapshots (총 {}개) ===", snapshots.len());

    for snapshot in snapshots {
        println!("\n[{:?}] {}", snapshot.exchange, snapshot.symbol);
        println!("  Currency: {:?}", snapshot.currency);

        if let Some(perp) = &snapshot.perp {
            println!("  Perp:");
            println!("    Mark Price: ${:.2}", perp.mark_price);
            println!("    OI USD: ${:.2}", perp.oi_usd);
            println!("    Vol 24h USD: ${:.2}", perp.vol_24h_usd);
            println!("    Funding Rate: {:.4}%", perp.funding_rate * 100.0);
            if let Some(next_funding) = perp.next_funding_time {
                println!("    Next Funding: {}", next_funding);
            }
        }

        if let Some(spot) = &snapshot.spot {
            println!("  Spot:");
            println!("    Price: ${:.2}", spot.price);
            println!("    Vol 24h USD: ${:.2}", spot.vol_24h_usd);
        }

        println!("  Exchange Rates:");
        println!("    USD/KRW: {:.2}", snapshot.exchange_rates.usd_krw);
        println!("    USDT/USD: {:.6}", snapshot.exchange_rates.usdt_usd);
        println!("    USDT/KRW: {:.2}", snapshot.exchange_rates.usdt_krw);

        println!("  Updated At: {}", snapshot.updated_at);
    }
}

pub async fn fetch_bithumb_assets() -> eyre::Result<Vec<SpotAsset>> {
    let client = BithumbClient::with_credentials()?;
    let assets = client
        .fetch_spots()
        .await
        .map_err(|e| eyre::eyre!("자산 조회 실패: {}", e))?;
    Ok(assets)
}

pub async fn fetch_binance_assets() -> eyre::Result<Vec<SpotAsset>> {
    let client = BinanceClient::with_credentials()?;
    let assets = client
        .fetch_spots()
        .await
        .map_err(|e| eyre::eyre!("자산 조회 실패: {}", e))?;
    Ok(assets)
}

pub fn print_assets(assets: &[SpotAsset]) {
    info!("=== Assets (총 {}개) ===", assets.len());

    for asset in assets {
        println!("\n[{}]", asset.currency);
        println!("  Total: {:.8}", asset.total);
        println!("  Available: {:.8}", asset.available);
        println!("  In Use: {:.8}", asset.in_use);
        println!("  Updated At: {}", asset.updated_at);
    }
}
