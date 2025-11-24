/// 현·선물 베이시스 전략에서 "양쪽 레그를 어떻게 실행할지"를 정의하는 상위 정책.
#[derive(Debug, Clone, Copy)]
pub enum ExecutionPolicy {
    /// 현재 파이썬 코드와 동일한 정책:
    /// 스팟과 선물 모두 시장가/공격적 지정가로 체결하는 완전 taker-taker.
    TakerTaker,
    /// 스팟은 maker(포스트 온리 지정가), 선물은 taker(시장가/공격적 지정가).
    /// "스팟 수수료 비싸고 선물 수수료 싸다"일 때 자주 쓰는 조합.
    SpotMakerFuturesTaker,
    /// 양쪽 모두 maker로 게시해서, 오더북에 걸려 있는 유동성만 먹게 하는 정책.
    /// 체결이 안 되면 포지션이 안 잡힐 수 있다는 전제를 깔고 쓰는 모드.
    MakerMaker,
    /// 우선 양쪽(or 한쪽) 다 maker로 시도하고,
    /// 일정 시간/슬리피지 한도를 넘으면 taker로 전환하는 하이브리드 정책.
    MakerFirstThenTaker,
    /// 기본은 taker인데, 스프레드가 충분히 넓을 때만
    /// maker 주문을 먼저 깔고 남은 잔량만 taker로 처리하는 정책.
    TakerWithOpportunisticMaker,
    /// 큰 사이즈를 여러 번 나눠서 시간 분할(TWAP)로 집행하는 정책.
    /// 각 슬라이스는 주로 taker로 실행.
    TakerTwap,
    /// 시간 분할(TWAP) + maker 기반: 일정 간격으로
    /// post-only limit 주문을 재배치하는 패시브 집행 정책.
    MakerTwap,
    /// 스프레드 구간에 격자/grid 형태로 maker 주문을 깔아두고
    /// 체결될 때마다 반대 레그를 taker로 맞추는 그리드형 실행 정책.
    MakerGrid,
}

/// 개별 레그(spot 또는 futures)에 대해 주문을 어떻게 집행할지 정의.
#[derive(Debug, Clone, Copy)]
pub enum LegExecutionPolicy {
    /// 완전한 taker: 시장가 또는 호가 안쪽으로 파고드는 공격적 지정가.
    MarketTaker,
    /// 지정가지만 사실상 taker가 되도록, 최우선 호가를 강하게 치고 들어가는 aggressive limit.
    AggressiveLimitTaker,
    /// 패시브 maker: 현재 스프레드 안쪽으로 들어가지 않고
    /// 호가 밖(혹은 mid보다 유리한 쪽)에 걸어두는 일반적인 maker 지정가.
    PassiveMaker,
    /// post-only 지정가: 거래소의 post-only 플래그를 사용해서
    /// maker로만 체결되도록 강제.
    PostOnlyMaker,
}

use interface::ExchangeId;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyMode {
    /// 스팟 롱 + 선물 숏
    Carry,
    /// 스팟 숏 + 선물 롱
    Reverse,
    /// 시장 상황에 따라 자동 선택
    Auto,
}

impl fmt::Display for StrategyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            StrategyMode::Carry => "carry",
            StrategyMode::Reverse => "reverse",
            StrategyMode::Auto => "auto",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone)]
pub struct StrategyParams {
    /// 거래할 심볼 (예: "BTCUSDT", "ETHUSDT")
    pub symbol: String,
    /// 전략 모드: carry (스팟 롱 + 선물 숏), reverse (스팟 숏 + 선물 롱), auto (자동 선택)
    pub mode: StrategyMode,
    /// 진입 임계값 (basis points). 베이시스가 이 값 이상 벌어지면 포지션 진입
    /// 예: 2.0 bps = 0.02% = (선물 - 스팟) / 스팟 * 10000 >= 2.0
    pub entry_bps: f64,
    /// 청산 임계값 (basis points). 베이시스가 이 값 이하로 좁혀지면 포지션 청산
    /// 예: 0.2 bps = 0.002%
    pub exit_bps: f64,
    /// 거래 명목가 (USDT 단위). 이 금액만큼의 포지션을 잡음
    /// 예: 100.0 USDT = 약 100 USDT 상당의 BTC를 거래
    pub notional: f64,
    /// 선물 레버리지 배수 (1 = 무레버리지, 2 = 2배 레버리지 등)
    pub leverage: u32,
    /// 선물 마진 타입: true = 격리 마진(ISOLATED), false = 교차 마진(CROSS)
    pub isolated: bool,
    /// 테스트 모드: true면 실제 주문을 넣지 않고 로그만 출력
    pub dry_run: bool,
    /// 양쪽 레그 실행 정책 (TakerTaker, SpotMakerFuturesTaker, MakerMaker 등)
    pub policy: ExecutionPolicy,
    /// 스팟 레그의 개별 실행 정책 (MarketTaker, AggressiveLimitTaker, PassiveMaker, PostOnlyMaker)
    pub spot_leg: LegExecutionPolicy,
    /// 선물 레그의 개별 실행 정책 (MarketTaker, AggressiveLimitTaker, PassiveMaker, PostOnlyMaker)
    pub futures_leg: LegExecutionPolicy,
}

impl Default for StrategyParams {
    fn default() -> Self {
        Self {
            symbol: "BTCUSDT".to_string(),
            mode: StrategyMode::Carry,
            entry_bps: 2.0,
            exit_bps: 0.2,
            notional: 100.0,
            leverage: 1,
            isolated: false,
            dry_run: false,
            policy: ExecutionPolicy::TakerTaker,
            spot_leg: LegExecutionPolicy::MarketTaker,
            futures_leg: LegExecutionPolicy::MarketTaker,
        }
    }
}

pub mod cross_basis;
pub mod intra_basis;

#[derive(Debug, Clone)]
pub struct CrossStrategyParams {
    /// 프리미엄 거래소(spot) 심볼 (예: "BTCKRW")
    pub primary_symbol: String,
    /// 헤지 거래소(선물) 심볼 (예: "BTCUSDT")
    pub hedge_symbol: String,
    /// 프리미엄 거래소 ID
    pub primary_exchange: ExchangeId,
    /// 헤지 거래소 ID
    pub hedge_exchange: ExchangeId,
    /// 프리미엄 거래소 spot 포지션 기준 모드 (carry/reverse/auto)
    pub mode: StrategyMode,
    /// 진입 임계값 (basis points)
    pub entry_bps: f64,
    /// 청산 임계값 (basis points)
    pub exit_bps: f64,
    /// 프리미엄 거래소에서 사용할 명목가 (현지 통화 단위)
    pub primary_notional: f64,
    /// 헤지 거래소에서 사용할 명목가 (USDT 등)
    pub hedge_notional: f64,
    /// 헤지 선물 레버리지
    pub leverage: u32,
    /// 헤지 선물 마진 타입 (true = 격리)
    pub isolated: bool,
    /// 테스트 모드 여부
    pub dry_run: bool,
    /// 양쪽 레그 실행 정책
    pub policy: ExecutionPolicy,
    /// 프리미엄 거래소 spot 주문 정책
    pub spot_leg: LegExecutionPolicy,
    /// 헤지 거래소 선물 주문 정책
    pub futures_leg: LegExecutionPolicy,
    /// 프리미엄 가격을 헤지 통화 기준으로 환산하기 위한 계수 (예: KRW->USDT)
    pub fx_adjustment: f64,
    /// 프리미엄 거래소에서 보유해야 하는 베이스 자산명 (예: "BTC")
    pub primary_base_asset: String,
}

impl Default for CrossStrategyParams {
    fn default() -> Self {
        Self {
            primary_symbol: "BTCKRW".to_string(),
            hedge_symbol: "BTCUSDT".to_string(),
            primary_exchange: ExchangeId::Bithumb,
            hedge_exchange: ExchangeId::Binance,
            mode: StrategyMode::Carry,
            entry_bps: 50.0,
            exit_bps: 5.0,
            primary_notional: 5_000_000.0, // KRW 단위 예시
            hedge_notional: 5_000.0,       // USDT 단위 예시
            leverage: 1,
            isolated: false,
            dry_run: true,
            policy: ExecutionPolicy::TakerTaker,
            spot_leg: LegExecutionPolicy::MarketTaker,
            futures_leg: LegExecutionPolicy::MarketTaker,
            fx_adjustment: 1.0,
            primary_base_asset: "BTC".to_string(),
        }
    }
}
