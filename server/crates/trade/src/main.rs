use std::{fs::OpenOptions, path::PathBuf};

use chrono::Local;
use color_eyre::eyre;
use exchanges::BinanceClient;
use structopt::StructOpt;
use tracing::{info, level_filters::LevelFilter};
use tracing_appender::non_blocking;
use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

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
    let _guards = init_tracing();

    // init trade record repository
    trade::record::init_global_repository()
        .await
        .map_err(|e| eyre::eyre!("거래 기록 저장소 초기화 실패: {}", e))?;

    // dotenv는 lib.rs에서 자동으로 로드됨

    let cmd = Command::from_args();

    match cmd {
        Command::Run => run_bot().await,
        Command::ExploreTest => run_explore_test().await,
        Command::ArbitrageTest => run_arbitrage_test().await,
        Command::EmergencyTest => run_emergency_test().await,
    }
}

pub struct TracingGuards {
    _file: tracing_appender::non_blocking::WorkerGuard,
    _stdout: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init_tracing() -> TracingGuards {
    // 1) 파일 appender
    let (file_writer, file_guard) = custom_daily_file_appender("logs", "trading");

    // 2) stdout도 non-blocking
    let (stdout_writer, stdout_guard) = non_blocking(std::io::stdout());

    // 3) EnvFilter
    let env_filter = EnvFilter::from_default_env().add_directive("info".parse().unwrap());

    // 4) 레이어 조립
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer)
                .with_filter(LevelFilter::INFO),
        )
        .with(fmt::layer().with_writer(stdout_writer).with_ansi(true))
        .init();

    // guards를 리턴해서 main에서 들고 있게 만들기
    TracingGuards {
        _file: file_guard,
        _stdout: stdout_guard,
    }
}

fn custom_daily_file_appender(
    base_dir: &str,
    prefix: &str,
) -> (
    non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
) {
    // 날짜 문자열 생성: 2025-11-29
    let date = Local::now().format("%Y-%m-%d").to_string();

    // 최종 파일 이름: trading.2025-11-29.log
    let filename = format!("{prefix}.{date}.log");

    // logs/trading.2025-11-29.log
    let mut path = PathBuf::from(base_dir);
    path.push(filename);

    // 파일 오픈
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("Failed to open custom log file");

    non_blocking(file)
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
