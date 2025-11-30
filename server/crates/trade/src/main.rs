use color_eyre::eyre;
use exchanges::BinanceClient;
use structopt::StructOpt;
use tracing::info;

mod explore;

use trade::arbitrage::{IntraBasisArbitrageStrategy, StrategyParams};

// lib.rs에서 자동으로 dotenv가 로드됨

#[derive(Debug, StructOpt)]
#[structopt(name = "trade", about = "베이시스 아비트라지 거래 봇")]
enum Command {
    /// 베이시스 아비트라지 전략 실행
    Run,
    /// Oracle 서버 및 거래소 데이터 조회 테스트
    ExploreTest,
    /// 베이시스 아비트라지 전략 테스트 (dry-run 모드)
    ArbitrageTest,
    /// 강제 청산 테스트 (모든 자산을 USDT/KRW로 변환)
    EmergencyTest,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // init error reporting
    color_eyre::install()?;

    // init logging
    let _guards = trade::logger::init_tracing();

    // init trade record repository
    trade::record::init_global_repository()
        .await
        .map_err(|e| eyre::eyre!("거래 기록 저장소 초기화 실패: {}", e))?;

    // dotenv는 lib.rs에서 자동으로 로드됨

    // API 서버를 백그라운드로 시작
    let server_port = std::env::var("TRADE_API_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(12091);

    let server_handle = tokio::spawn(async move {
        if let Err(e) = trade::server::start_server(server_port).await {
            tracing::error!("API 서버 실행 중 오류 발생: {}", e);
        }
    });

    info!("API 서버가 포트 {}에서 시작되었습니다", server_port);

    let cmd = Command::from_args();

    // 커맨드 실행 (서버는 백그라운드에서 계속 실행됨)
    let result = match cmd {
        Command::Run => run_bot().await,
        Command::ExploreTest => run_explore_test().await,
        Command::ArbitrageTest => run_arbitrage_test().await,
        Command::EmergencyTest => run_emergency_test().await,
    };

    // 커맨드가 완료되어도 서버는 계속 실행되도록 대기
    // 서버가 종료되면 프로그램도 종료됨
    if let Err(e) = server_handle.await {
        tracing::error!("서버 태스크 오류: {:?}", e);
    }

    result
}

async fn run_bot() -> eyre::Result<()> {
    info!("거래 봇 시작...");

    info!("Oracle에서 unified-snapshots 데이터 가져오는 중...");

    let snapshots = explore::fetch_unified_snapshots().await?;
    explore::print_unified_snapshots(&snapshots);

    todo!()
}

/// Oracle 서버 및 거래소 데이터 조회 테스트
async fn run_explore_test() -> eyre::Result<()> {
    let binance = BinanceClient::with_credentials()?;
    let fee = binance.get_trade_fee_for_symbol("XPLUSDT").await?;
    println!("fee: {:?}", fee);

    info!("\n=== Bithumb 자산 정보 조회 중... ===");
    let assets = explore::fetch_bithumb_assets().await?;
    explore::print_assets(&assets);

    info!("\n=== Binance 자산 정보 조회 중... ===");
    let assets = explore::fetch_binance_assets().await?;
    explore::print_assets(&assets);

    info!("완료!");

    Ok(())
}

/// 베이시스 아비트라지 전략 테스트 (dry-run 모드)
async fn run_arbitrage_test() -> eyre::Result<()> {
    info!("베이시스 아비트라지 전략 테스트 시작 (dry-run 모드)...");

    let mut params = StrategyParams::default();
    params.dry_run = false;

    info!("테스트 파라미터:");
    info!("  Symbol: {}", params.symbol);
    info!("  Mode: {}", params.mode);
    info!("  Entry BPS: {}", params.entry_bps);
    info!("  Exit BPS: {}", params.exit_bps);
    info!("  Notional: {} USDT", params.notional);
    info!("  Leverage: {}x", params.leverage);
    info!("  Isolated: {}", params.isolated);
    info!("  Dry Run: {}", params.dry_run);

    let strategy = IntraBasisArbitrageStrategy::new(params)
        .map_err(|e| eyre::eyre!("전략 초기화 실패: {}", e))?;

    info!("전략이 성공적으로 초기화되었습니다.");

    strategy.run_loop().await?;

    info!("전략이 성공적으로 실행되었습니다.");
    info!("실제 실행을 위해서는 'run' 커맨드를 사용하세요.");

    Ok(())
}

/// 강제 청산 테스트
async fn run_emergency_test() -> eyre::Result<()> {
    info!("강제 청산 테스트 시작...");
    info!("주의: 이 명령은 실제 거래를 실행합니다!");

    trade::emergency::liquidate_all().await?;

    info!("강제 청산 테스트 완료!");

    Ok(())
}
