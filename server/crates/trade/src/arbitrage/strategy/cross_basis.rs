//! 두 개의 거래소 간 가격 격차(베이시스)를 동시에 이용하는 크로스 거래 전략.
//! 프리미엄 거래소(spot)와 헤지 거래소(선물)의 가격을 비교해 carry/reverse 포지션을 관리한다.

use serde_json;
use tracing::{info, warn};

use super::super::super::trader::{
    BinanceTrader, FuturesExchangeTrader, OrderResponse, SpotExchangeTrader,
};
use interface::ExchangeError;

use super::super::state::ArbitrageState;
use super::{CrossStrategyParams, StrategyMode};

/// 두 개의 서로 다른 거래소 간 베이시스(가격 격차)를 이용해
/// **크로스 거래소 델타-뉴트럴 포지션**을 자동으로 관리하는 전략 엔진.
///
/// 이 전략은 다음과 같은 상황을 다룬다:
/// - 프리미엄 거래소(보통 spot, 예: 빗썸 BTCKRW / 바이낸스 spot 등)
/// - 헤지 거래소(보통 선물, 예: 바이낸스/바이비트 perp 등)
/// - 또는 바이낸스 ↔ 바이비트처럼 둘 다 해외 거래소인 조합
///
/// 핵심 아이디어는 intra(한 거래소 내 현·선물) 아비트라지와 유사하지만,
/// **두 개의 서로 다른 계정/마진 시스템/통화 체계**를 다룬다는 점이 다르다:
/// - 각 거래소가 따로 청산/마진/ADL 정책을 가지므로,
///   한쪽에서만 청산이 나버리는 리스크를 고려해야 한다.
/// - primary_notional / hedge_notional, spot 재고(primary_base_asset),
///   레버리지·마진 여유를 모두 감안하여
///   **두 거래소가 동시에 감당 가능한 공통 수량**만큼만 포지션을 연다.
/// - 필요하다면 fx_adjustment 를 통해
///   KRW/USDT, USD/USDT, 테더 프리미엄 등 FX 요소를 반영해
///   “같은 기준 통화”에서의 베이시스를 계산할 수 있다.
///
/// 동작 개요:
/// - SpotExchangeTrader / FuturesExchangeTrader 추상화를 통해
///   프리미엄 거래소 spot 레그와 헤지 거래소 선물 레그를 제어한다.
/// - run_loop():
///   - 1초 주기로 primary_symbol spot 가격과 hedge_symbol mark 가격을 조회
///   - primary_price 에 fx_adjustment 를 곱해 헤지 통화 기준 가격으로 환산
///   - (hedge_mark - adjusted_primary) / adjusted_primary * 10_000 으로
///     bps 단위 베이시스(basis_bps)를 계산
///   - StrategyMode( Carry / Reverse / Auto )와 entry_bps / exit_bps 에 따라
///     포지션 진입/청산 여부를 판단
///
/// 포지션 구조:
/// - CARRY 포지션 (프리미엄 시장이 더 비쌀 때를 노림)
///   - primary(프리미엄 거래소): spot **BUY**
///   - hedge(헤지 거래소): futures **SELL**
///   - 김프 캐리, 선물-현물 베이시스 캐리 등 “비싼 시장을 숏, 싼 시장을 롱” 구조
///
/// - REVERSE 포지션 (프리미엄 시장이 과도하게 싸거나, 보유 재고를 활용할 때)
///   - primary: 보유한 primary_base_asset 재고 한도 내에서 spot **SELL**
///   - hedge: futures **BUY**
///   - 재고를 줄이면서 크로스 베이시스 역전 구간을 먹는 전략
///
/// 자금/수량 관리:
/// - primary_notional / hedge_notional 과 현재 가격으로 양쪽에서 목표 수량을 계산하고,
///   둘 중 더 작은 값만큼만 주문을 내어 **한쪽 계정의 한계를 넘지 않도록** 한다.
/// - clamp_spot_quantity / clamp_futures_quantity 를 통해
///   각 거래소의 LOT_SIZE 규칙을 반영하고,
///   clamp_cross_quantity() 에서 다시 양쪽 레그가 동시에 소화 가능한
///   최소 수량으로 맞춘다.
/// - REVERSE 진입 시에는 primary_base_asset spot 재고를 조회해
///   “재고가 허용하는 범위 안”에서만 포지션을 연다.
///
/// 상태 관리:
/// - ArbitrageState 를 파일로 읽고/쓰기 때문에,
///   프로세스를 재시작해도
///   - 현재 포지션 유무(open)
///   - 포지션 방향(dir = "carry" / "reverse")
///   - 진입 시 베이시스, 수량, 주문 내역(actions)
///   을 복원할 수 있다.
/// - 포지션이 열린 상태에서는 **청산 조건만** 감시하고,
///   포지션이 없을 때만 **진입 조건**을 평가한다.
///
/// 요약하면 이 타입은:
/// - intra(한 거래소 내) 아비트라지와 동일한 “베이시스 bps 기반 진입/청산 로직”을 유지하되,
/// - 두 개의 서로 다른 거래소/계정/통화/재고 제약을 동시에 고려하고,
/// - carry / reverse 두 방향의 크로스 포지션을 자동으로 열고 닫으면서
///   김프, 테더 프리미엄, 해외↔해외 perp 스프레드 등
///   **크로스 거래소 베이시스 기회**를 실전에서 운용할 수 있도록 해 주는
///   전략 오케스트레이터이다.
pub struct CrossBasisArbitrageStrategy<S = BinanceTrader, F = BinanceTrader>
where
    S: SpotExchangeTrader,
    F: FuturesExchangeTrader,
{
    spot_trader: S,
    hedge_trader: F,
    params: CrossStrategyParams,
}

// TODO: 각 거래소별 taker/maker 수수료, 리베이트(VIP, MM 프로그램 등)를 반영해
//       진입/청산 기준 bps (entry_bps/exit_bps)를 "수수료 후 기준"으로 재계산하기.

// TODO: 펀딩비(해당 심볼의 funding rate)와 funding 시점(8시간/4시간 등)을 조회하여
//       - carry 포지션: 장기 홀드 시 기대 펀딩 이익/손실
//       - reverse 포지션: 펀딩 역전 시 리스크
//       를 고려한 "펀딩 조정 베이시스"를 정의하고, entry/exit 로직에 반영하기.

// TODO: FX 및 김프/테더 프리미엄 모델 고도화:
//       - fx_adjustment 를 고정 상수가 아니라 실시간 환율 + USDT 프리미엄 추정치로 갱신
//       - KRW↔USDT, USD↔USDT, 거래소별 USDT 프리미엄까지 포함한 "실질 cross 베이시스" 계산.

// TODO: 슬리피지/오더북 깊이를 고려한 체결 가능 수량 산출:
//       - 호가창 스냅샷(또는 depth API)을 사용해 원하는 가격 괴리 이내에서
//         실제로 체결 가능한 qty를 추정하고, target_quantity 에 반영하기.

// TODO: 부분 체결/미체결 주문 처리:
//       - spot/futures 한쪽만 부분 체결된 경우
//         * 남은 수량에 대해 재주문 또는 다른 레그 축소
//         * 타임아웃/최대 재시도 횟수 정책
//       - 주문 취소/정리 로직을 명시적으로 구현.

// TODO: 크로스 거래소 마진/레버리지 안전선 설정:
//       - 각 거래소별 "최소 유지 마진율" + 버퍼를 기준으로
//         레버리지 상한 및 포지션 크기 상한 계산
//       - 한쪽 계정 마진 여유가 줄어들면 자동으로 포지션 축소 또는 신규 진입 중단.

// TODO: 계정 간 잔고/재고 리밸런싱 전략:
//       - 한쪽 거래소에 수익/손실이 쌓여 잔고 불균형이 커지는 경우
//         * on-chain 전송 또는 내부 전송으로 재조정
//       - 전송 수수료/지연/리스크를 고려한 리밸런싱 주기/조건 정의.

// TODO: ExecutionPolicy / LegExecutionPolicy 고도화:
//       - 거래소별 fee tier, 오더북 특성에 따라
//         * primary: maker 위주, hedge: taker 위주
//         * 또는 반대 조합
//         을 자동으로 선택/튜닝할 수 있는 정책 엔진 추가.

// TODO: 장애/예외 상황 핸들링 강화:
//       - 한 거래소 API 장애/일시적 에러 시
//         * 백오프 + 재시도 정책
//         * 상대 거래소 포지션만 남지 않도록 방어 로직
//       - 청산/ADL 발생 시 해당 이벤트를 감지하고 상태/포지션을 강제 동기화.

// TODO: PnL/리스크 모니터링 지표 추가:
//       - 거래소별 실현/미실현 PnL
//       - 포지션별 누적 펀딩 수익/비용
//       - 베이시스 히스토리, 평균 보유 기간, 최대 드로다운 등
//       을 주기적으로 계산/로그/외부 모니터링 시스템으로 내보내기.

// TODO: ArbitrageState 포맷/버전 관리:
//       - 필드 추가/변경 시 버전 번호를 두고
//         이전 버전 상태 파일을 안전하게 마이그레이션하는 로직 추가.
//       - 상태 파일 손상/누락 시 복구 전략 정의.

// TODO: 동시 다심볼 지원:
//       - struct 를 심볼 단위 인스턴스에서 전략 "매니저" 레이어로 확장해
//         여러 심볼을 병렬로 운용할 수 있도록 설계.
//       - 심볼 간 자금/마진 공유에 따른 리스크 관리 로직 추가.

// TODO: 전략 파라미터 튜닝/백테스트 경로 연결:
//       - dry_run 모드에서 실제 주문 대신 "가상 체결"을 기록해
//         ex-post 분석/백테스트에 사용할 수 있는 로그 포맷 정의.
//       - entry_bps/exit_bps/fx_adjustment/레버리지 등을 자동 탐색하는
//         오프라인 튜닝 도구와의 인터페이스 설계.

impl CrossBasisArbitrageStrategy {
    pub fn new(params: CrossStrategyParams) -> Result<Self, ExchangeError> {
        let spot_trader = BinanceTrader::new()?;
        let hedge_trader = BinanceTrader::new()?;
        Ok(Self::with_traders(spot_trader, hedge_trader, params))
    }
}

impl<S, F> CrossBasisArbitrageStrategy<S, F>
where
    S: SpotExchangeTrader,
    F: FuturesExchangeTrader,
{
    pub fn with_traders(spot_trader: S, hedge_trader: F, params: CrossStrategyParams) -> Self {
        Self {
            spot_trader,
            hedge_trader,
            params,
        }
    }

    pub fn params(&self) -> &CrossStrategyParams {
        &self.params
    }

    fn state_symbol(&self) -> String {
        format!(
            "{}@{:?}|{}@{:?}",
            self.params.primary_symbol,
            self.params.primary_exchange,
            self.params.hedge_symbol,
            self.params.hedge_exchange
        )
    }

    fn clamp_cross_quantity(&self, qty: f64) -> f64 {
        let spot_qty = self
            .spot_trader
            .clamp_spot_quantity(&self.params.primary_symbol, qty);
        let fut_qty = self
            .hedge_trader
            .clamp_futures_quantity(&self.params.hedge_symbol, qty);
        spot_qty.min(fut_qty)
    }

    fn target_quantity(&self, primary_price: f64, hedge_price: f64) -> f64 {
        let primary_qty = if primary_price > 0.0 {
            self.params.primary_notional / primary_price
        } else {
            0.0
        };

        let hedge_qty = if hedge_price > 0.0 {
            self.params.hedge_notional / hedge_price
        } else {
            0.0
        };

        primary_qty.min(hedge_qty)
    }

    /// 크로스 베이시스 메인 루프.
    ///
    /// 이 루프는 1초 간격으로 두 거래소의 가격을 모니터링하면서,
    /// 기준(bps)이 설정한 임계값을 넘나들 때 carry / reverse 포지션을
    /// 자동으로 진입·청산한다.
    ///
    /// 동작 개요:
    /// 1. 초기화
    ///    - 프리미엄 거래소(spot)와 헤지 거래소(선물)의 exchange info 를 로드한다.
    ///    - 헤지 선물 계정에 대해 레버리지/격리 여부를 ensure_account_setup 으로 설정한다.
    ///    - (primary_symbol, primary_exchange, hedge_symbol, hedge_exchange) 조합으로
    ///      ArbitrageState 키를 만들고, 이전 실행에서 저장된 포지션 상태를 복원한다.
    ///
    /// 2. 가격 수집 및 베이시스(basis) 계산
    ///    - primary_symbol 의 spot 가격(primary_price)과
    ///      hedge_symbol 의 선물 mark price(hedge_mark)를 조회한다.
    ///    - primary_price 에 fx_adjustment 를 곱해 헤지 통화 기준 가격(adjusted_primary)을 만든다.
    ///    - 두 가격 차이를 adjusted_primary 로 나눈 뒤 10_000 을 곱해
    ///      basis_bps = (hedge_mark - adjusted_primary) / adjusted_primary * 10_000
    ///      형태로 bps 단위 스프레드를 계산하고 로그로 출력한다.
    ///
    /// 3. 포지션이 열려 있을 때(state.open == true)
    ///    - state.dir == "carry" 인 경우:
    ///        * basis_bps 가 exit_bps 이하로 내려오면 캐리 청산 조건으로 본다.
    ///    - state.dir == "reverse" 인 경우:
    ///        * basis_bps 가 -exit_bps 이상으로 올라오면 리버스 청산 조건으로 본다.
    ///    - 청산 조건이 만족되면:
    ///        * carry → close_carry(state.qty):
    ///            - 선물 레그: reduce-only BUY
    ///            - spot 레그: SELL
    ///        * reverse → close_reverse(state.qty):
    ///            - 선물 레그: reduce-only SELL
    ///            - spot 레그: BUY
    ///        * 두 레그의 OrderResponse 를 JSON(actions) 으로 기록하고,
    ///          ArbitrageState 를 닫힌 상태(open=false, dir=None, qty=0)로 갱신 후 디스크에 저장한다.
    ///
    /// 4. 포지션이 없을 때(state.open == false)
    ///    - 현재 basis_bps 와 params.mode( Carry / Reverse / Auto ) 를 기준으로
    ///      어느 방향으로 진입할지 판단한다.
    ///        * Carry 진입 조건:
    ///            - mode 가 Carry 또는 Auto 이고
    ///            - basis_bps > entry_bps
    ///        * Reverse 진입 조건:
    ///            - mode 가 Reverse 또는 Auto 이고
    ///            - basis_bps < -entry_bps
    ///    - primary_notional / hedge_notional 과 현재 가격을 이용해
    ///      양쪽 거래소가 동시에 소화 가능한 공통 수량(target_quantity)을 계산하고,
    ///      clamp_cross_quantity 로 각 거래소의 최소 수량 규칙에 맞게 보정한다.
    ///    - carry 진입(open_carry):
    ///        * 프리미엄 거래소 spot: BUY
    ///        * 헤지 거래소 선물: SELL
    ///        * 델타 뉴트럴 캐리 포지션을 구성한다.
    ///    - reverse 진입(open_reverse):
    ///        * primary_base_asset 의 spot 보유량을 조회해 재고 한도를 적용한 뒤,
    ///        * 프리미엄 거래소 spot: SELL
    ///        * 헤지 거래소 선물: BUY
    ///        * 보유 재고를 활용한 리버스 포지션을 구성한다.
    ///    - 진입이 성공하면 체결 결과와 filled_qty, 진입 시점 basis_bps 를
    ///      ArbitrageState 에 기록하고 open=true, dir="carry"/"reverse" 로 설정해 저장한다.
    ///
    /// 5. 예외 및 dry-run 처리
    ///    - 가격 조회나 주문 요청이 실패하면 경고 로그를 남기고 해당 에러를 그대로 전파하여
    ///      run_loop 를 종료한다(상위에서 재시작 여부를 결정할 수 있도록).
    ///    - params.dry_run == true 인 경우 실제 주문 대신 “어떤 주문을 실행했을지”만 로그로 남기며,
    ///      open/close_* 함수가 "Dry run mode" 에러를 반환하므로 실거래 없이 전략 로직만 검증할 수 있다.
    ///
    /// 이 함수는 정상 동작 시 무한 루프로 계속 실행되며,
    /// 네트워크/거래소 에러 또는 호출자가 반환된 에러를 처리할 때까지 종료되지 않는다.
    pub async fn run_loop(&self) -> Result<(), ExchangeError> {
        self.spot_trader.ensure_exchange_info().await?;
        self.hedge_trader.ensure_exchange_info().await?;
        self.hedge_trader
            .ensure_account_setup(
                &self.params.hedge_symbol,
                self.params.leverage,
                self.params.isolated,
            )
            .await?;

        let mut state = ArbitrageState::read()?;
        let state_symbol = self.state_symbol();
        if state.symbol != state_symbol {
            state = ArbitrageState::new(state_symbol.clone());
        }

        info!("Starting cross-exchange basis arbitrage strategy");
        info!(
            "Premium Exchange: {:?} {}, Hedge Exchange: {:?} {}",
            self.params.primary_exchange,
            self.params.primary_symbol,
            self.params.hedge_exchange,
            self.params.hedge_symbol
        );
        info!(
            "Mode: {:?}, Entry BPS: {}, Exit BPS: {}",
            self.params.mode, self.params.entry_bps, self.params.exit_bps
        );

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            let primary_price = self
                .spot_trader
                .get_spot_price(&self.params.primary_symbol)
                .await
                .map_err(|e| {
                    warn!("Failed to get primary spot price: {}", e);
                    e
                })?;

            let hedge_mark = self
                .hedge_trader
                .get_mark_price(&self.params.hedge_symbol)
                .await
                .map_err(|e| {
                    warn!("Failed to get hedge mark price: {}", e);
                    e
                })?;

            let adjusted_primary = primary_price * self.params.fx_adjustment;
            if adjusted_primary <= 0.0 {
                warn!(
                    "Adjusted primary price invalid ({}). Skipping iteration.",
                    adjusted_primary
                );
                continue;
            }

            let basis_bps = (hedge_mark - adjusted_primary) / adjusted_primary * 10_000.0;

            info!(
                "Primary: {:.2}, Hedge: {:.2}, Adjusted Basis: {:.2} bps",
                primary_price, hedge_mark, basis_bps
            );

            if state.open {
                // 이미 포지션이 있을 경우 청산 조건만 감시
                let should_close = match state.dir.as_deref() {
                    Some("carry") => basis_bps <= self.params.exit_bps,
                    Some("reverse") => basis_bps >= -self.params.exit_bps,
                    _ => false,
                };

                if should_close {
                    info!("Exit condition met. Closing position...");
                    let result = match state.dir.as_deref() {
                        Some("carry") => self.close_carry(state.qty).await,
                        Some("reverse") => self.close_reverse(state.qty).await,
                        _ => {
                            warn!("Unknown position direction: {:?}", state.dir);
                            continue;
                        }
                    };

                    match result {
                        Ok((hedge_order, spot_order)) => {
                            let actions = serde_json::json!({
                                "hedge": hedge_order,
                                "spot": spot_order,
                            });
                            state.update_position(false, None, 0.0, Some(basis_bps), Some(actions));
                            state.write()?;
                            info!("Position closed successfully");
                        }
                        Err(e) => {
                            warn!("Failed to close position: {}", e);
                        }
                    }
                }
            } else {
                // 포지션이 없을 때만 carry/reverse 진입 여부 판단
                let should_open_carry =
                    matches!(self.params.mode, StrategyMode::Carry | StrategyMode::Auto)
                        && basis_bps > self.params.entry_bps;

                let should_open_reverse =
                    matches!(self.params.mode, StrategyMode::Reverse | StrategyMode::Auto)
                        && basis_bps < -self.params.entry_bps;

                let qty = self.target_quantity(primary_price, hedge_mark);
                if qty <= 0.0 {
                    warn!(
                        "Target quantity too small. primary/hedge prices: {}/{}",
                        primary_price, hedge_mark
                    );
                    continue;
                }

                if should_open_carry {
                    info!("Entry condition met for cross-exchange CARRY. Opening position...");
                    match self.open_carry(qty).await {
                        Ok((spot_order, hedge_order, filled_qty)) => {
                            let actions = serde_json::json!({
                                "spot": spot_order,
                                "hedge": hedge_order,
                            });
                            state.update_position(
                                true,
                                Some("carry".to_string()),
                                filled_qty,
                                Some(basis_bps),
                                Some(actions),
                            );
                            state.write()?;
                            info!("Cross-exchange CARRY position opened successfully");
                        }
                        Err(e) => {
                            warn!("Failed to open CARRY position: {}", e);
                        }
                    }
                } else if should_open_reverse {
                    info!("Entry condition met for cross-exchange REVERSE. Opening position...");
                    match self.open_reverse(qty).await {
                        Ok((spot_order, hedge_order, filled_qty)) => {
                            let actions = serde_json::json!({
                                "spot": spot_order,
                                "hedge": hedge_order,
                            });
                            state.update_position(
                                true,
                                Some("reverse".to_string()),
                                filled_qty,
                                Some(basis_bps),
                                Some(actions),
                            );
                            state.write()?;
                            info!("Cross-exchange REVERSE position opened successfully");
                        }
                        Err(e) => {
                            warn!("Failed to open REVERSE position: {}", e);
                        }
                    }
                }
            }
        }
    }

    async fn open_carry(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse, f64), ExchangeError> {
        info!(
            "Opening cross CARRY: buy {} {} on {:?}, sell futures {} {} on {:?}",
            qty,
            self.params.primary_symbol,
            self.params.primary_exchange,
            qty,
            self.params.hedge_symbol,
            self.params.hedge_exchange
        );

        if self.params.dry_run {
            info!(
                "DRY RUN: Would BUY spot {} {}, SELL futures {} {}",
                qty, self.params.primary_symbol, qty, self.params.hedge_symbol
            );
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        let trade_qty = self.clamp_cross_quantity(qty);
        if trade_qty <= 0.0 {
            return Err(ExchangeError::Other(format!(
                "Quantity too small after clamping. Requested={}",
                qty
            )));
        }

        let spot_order = self
            .spot_trader
            .buy_spot(&self.params.primary_symbol, trade_qty)
            .await?;
        let hedge_order = self
            .hedge_trader
            .sell_futures(&self.params.hedge_symbol, trade_qty, false)
            .await?;

        Ok((spot_order, hedge_order, trade_qty))
    }

    async fn close_carry(&self, qty: f64) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!("Closing cross CARRY position (reduce-only) qty {}", qty);

        if self.params.dry_run {
            info!(
                "DRY RUN: Would BUY futures {} {} (reduce only) and SELL spot {} {}",
                qty, self.params.hedge_symbol, qty, self.params.primary_symbol
            );
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        let trade_qty = self.clamp_cross_quantity(qty);
        if trade_qty <= 0.0 {
            return Err(ExchangeError::Other(
                "Quantity too small after clamping".to_string(),
            ));
        }

        let hedge_order = self
            .hedge_trader
            .buy_futures(&self.params.hedge_symbol, trade_qty, true)
            .await?;
        let spot_order = self
            .spot_trader
            .sell_spot(&self.params.primary_symbol, trade_qty)
            .await?;

        Ok((hedge_order, spot_order))
    }

    async fn open_reverse(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse, f64), ExchangeError> {
        info!(
            "Opening cross REVERSE: sell {} {} on {:?}, buy futures {} {} on {:?}",
            qty,
            self.params.primary_symbol,
            self.params.primary_exchange,
            qty,
            self.params.hedge_symbol,
            self.params.hedge_exchange
        );

        if self.params.dry_run {
            info!(
                "DRY RUN: Would SELL spot {} {}, BUY futures {} {}",
                qty, self.params.primary_symbol, qty, self.params.hedge_symbol
            );
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        let spot_balance = self
            .spot_trader
            .get_spot_balance(&self.params.primary_base_asset)
            .await?;
        if spot_balance <= 0.0 {
            return Err(ExchangeError::Other(format!(
                "Insufficient spot inventory on {:?}. balance={}",
                self.params.primary_exchange, spot_balance
            )));
        }

        let max_qty = spot_balance.min(qty);
        let trade_qty = self.clamp_cross_quantity(max_qty);
        if trade_qty <= 0.0 {
            return Err(ExchangeError::Other(
                "Quantity too small after inventory clamp".to_string(),
            ));
        }

        let spot_order = self
            .spot_trader
            .sell_spot(&self.params.primary_symbol, trade_qty)
            .await?;
        let hedge_order = self
            .hedge_trader
            .buy_futures(&self.params.hedge_symbol, trade_qty, false)
            .await?;

        Ok((spot_order, hedge_order, trade_qty))
    }

    async fn close_reverse(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!("Closing cross REVERSE position qty {}", qty);

        if self.params.dry_run {
            info!(
                "DRY RUN: Would SELL futures {} {} (reduce only), BUY spot {} {}",
                qty, self.params.hedge_symbol, qty, self.params.primary_symbol
            );
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        let trade_qty = self.clamp_cross_quantity(qty);
        if trade_qty <= 0.0 {
            return Err(ExchangeError::Other(
                "Quantity too small after clamping".to_string(),
            ));
        }

        let hedge_order = self
            .hedge_trader
            .sell_futures(&self.params.hedge_symbol, trade_qty, true)
            .await?;
        let spot_order = self
            .spot_trader
            .buy_spot(&self.params.primary_symbol, trade_qty)
            .await?;

        Ok((hedge_order, spot_order))
    }
}
