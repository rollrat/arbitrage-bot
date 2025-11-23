use color_eyre::eyre;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

use interface::{Asset, UnifiedSnapshot};

// lib.rs에서 자동으로 dotenv가 로드됨
use trade::{AssetExchange, BithumbAssetClient};

const ORACLE_SERVER_URL: &str = "http://localhost:12090";

async fn fetch_unified_snapshots() -> eyre::Result<Vec<UnifiedSnapshot>> {
    let url = format!("{}/unified-snapshots", ORACLE_SERVER_URL);
    let response = reqwest::get(&url).await?;

    if !response.status().is_success() {
        return Err(eyre::eyre!("서버 응답 오류: {}", response.status()));
    }

    let snapshots: Vec<UnifiedSnapshot> = response.json().await?;
    Ok(snapshots)
}

fn print_unified_snapshots(snapshots: &[UnifiedSnapshot]) {
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

async fn fetch_bithumb_assets() -> eyre::Result<Vec<Asset>> {
    let client = BithumbAssetClient::new()
        .map_err(|e| eyre::eyre!("BithumbAssetClient 생성 실패: {}", e))?;
    let assets = client
        .fetch_assets()
        .await
        .map_err(|e| eyre::eyre!("자산 조회 실패: {}", e))?;
    Ok(assets)
}

fn print_assets(assets: &[Asset]) {
    info!("=== Assets (총 {}개) ===", assets.len());

    for asset in assets {
        println!("\n[{}]", asset.currency);
        println!("  Total: {:.8}", asset.total);
        println!("  Available: {:.8}", asset.available);
        println!("  In Use: {:.8}", asset.in_use);
        println!("  Updated At: {}", asset.updated_at);
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // init error reporting
    color_eyre::install()?;

    // init logging
    fmt().with_env_filter(EnvFilter::from_default_env()).init();

    // dotenv는 lib.rs에서 자동으로 로드됨

    info!("Oracle 서버에서 unified-snapshots 데이터 가져오는 중...");
    info!("서버 URL: {}", ORACLE_SERVER_URL);

    let snapshots = fetch_unified_snapshots().await?;
    print_unified_snapshots(&snapshots);

    info!("\n=== Bithumb 자산 정보 조회 중... ===");
    let assets = fetch_bithumb_assets().await?;
    print_assets(&assets);

    info!("완료!");

    Ok(())
}
