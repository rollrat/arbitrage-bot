use interface::ExchangeError;
use serde_json;
use tracing::{info, warn};

use super::{
    binance_trader::{BinanceTrader, OrderResponse},
    state::ArbitrageState,
};

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

#[derive(Debug, Clone)]
pub struct StrategyParams {
    /// 거래할 심볼 (예: "BTCUSDT", "ETHUSDT")
    pub symbol: String,
    /// 전략 모드: "carry" (스팟 롱 + 선물 숏), "reverse" (스팟 숏 + 선물 롱), "auto" (자동 선택)
    pub mode: String,
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
            mode: "carry".to_string(),
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

pub struct BasisArbitrageStrategy {
    trader: BinanceTrader,
    params: StrategyParams,
}

impl BasisArbitrageStrategy {
    pub fn new(params: StrategyParams) -> Result<Self, ExchangeError> {
        let trader = BinanceTrader::new()?;
        Ok(Self { trader, params })
    }

    /// 베이시스 계산 (bps 단위)
    /// basis_bps = (futures_mark - spot_price) / spot_price * 10000
    pub fn compute_basis_bps(&self, spot_price: f64, futures_mark: f64) -> f64 {
        if spot_price <= 0.0 {
            return 0.0;
        }
        (futures_mark - spot_price) / spot_price * 10000.0
    }

    /// 명목가에서 수량 계산 (스팟 기준)
    pub fn size_from_notional(&self, spot_price: f64) -> f64 {
        let qty = self.params.notional / spot_price;
        self.trader.clamp_spot_quantity(&self.params.symbol, qty)
    }

    /// Carry 포지션 오픈: 스팟 롱 + 선물 숏
    pub async fn open_carry(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!(
            "Opening CARRY position: spot BUY {} {}, futures SELL {} {}",
            qty, self.params.symbol, qty, self.params.symbol
        );

        if self.params.dry_run {
            info!("DRY RUN: spot BUY {} {}", qty, self.params.symbol);
            info!("DRY RUN: futures SELL {} {}", qty, self.params.symbol);
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        // 스팟과 선물의 수량을 각각 clamp하고, 더 작은 쪽 사용
        let spot_qty = self.trader.clamp_spot_quantity(&self.params.symbol, qty);
        let fut_qty = self.trader.clamp_futures_quantity(&self.params.symbol, qty);
        let use_qty = spot_qty.min(fut_qty);

        if use_qty <= 0.0 {
            return Err(ExchangeError::Other(format!(
                "Quantity too small after clamping. Increase notional. spot_qty={}, fut_qty={}",
                spot_qty, fut_qty
            )));
        }

        // 스팟 매수
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "BUY", use_qty, false)
            .await?;

        // 선물 숏
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "SELL", use_qty, false)
            .await?;

        Ok((spot_order, futures_order))
    }

    /// Carry 포지션 클로즈: 스팟 매도 + 선물 매수 (reduceOnly)
    pub async fn close_carry(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!(
            "Closing CARRY position: spot SELL {} {}, futures BUY {} {} (reduceOnly)",
            qty, self.params.symbol, qty, self.params.symbol
        );

        if self.params.dry_run {
            info!(
                "DRY RUN: futures BUY {} {} (reduceOnly)",
                qty, self.params.symbol
            );
            info!("DRY RUN: spot SELL {} {}", qty, self.params.symbol);
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        // 선물 청산 (reduceOnly)
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "BUY", qty, true)
            .await?;

        // 스팟 매도
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "SELL", qty, false)
            .await?;

        Ok((futures_order, spot_order))
    }

    /// Reverse 포지션 오픈: 스팟 숏(보유분만) + 선물 롱
    pub async fn open_reverse(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!(
            "Opening REVERSE position: spot SELL {} {}, futures BUY {} {}",
            qty, self.params.symbol, qty, self.params.symbol
        );

        if self.params.dry_run {
            info!("DRY RUN: spot SELL {} {}", qty, self.params.symbol);
            info!("DRY RUN: futures BUY {} {}", qty, self.params.symbol);
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        // 스팟 잔고 확인
        let base_asset = BinanceTrader::base_asset_from_symbol(&self.params.symbol);
        let free = self.trader.get_spot_balance(&base_asset).await?;
        let available_qty = qty.min(free);
        let use_qty = self
            .trader
            .clamp_spot_quantity(&self.params.symbol, available_qty);

        if use_qty <= 0.0 {
            return Err(ExchangeError::Other(format!(
                "Insufficient spot inventory to sell. free={}, requested={}",
                free, qty
            )));
        }

        // 선물 수량도 clamp
        let fut_qty = self.trader.clamp_futures_quantity(&self.params.symbol, qty);
        let final_qty = use_qty.min(fut_qty);

        if final_qty <= 0.0 {
            return Err(ExchangeError::Other(format!(
                "Quantity too small after clamping. use_qty={}, fut_qty={}",
                use_qty, fut_qty
            )));
        }

        // 스팟 매도
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "SELL", final_qty, false)
            .await?;

        // 선물 롱
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "BUY", final_qty, false)
            .await?;

        Ok((spot_order, futures_order))
    }

    /// Reverse 포지션 클로즈: 스팟 매수 + 선물 매도 (reduceOnly)
    pub async fn close_reverse(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!(
            "Closing REVERSE position: spot BUY {} {}, futures SELL {} {} (reduceOnly)",
            qty, self.params.symbol, qty, self.params.symbol
        );

        if self.params.dry_run {
            info!(
                "DRY RUN: futures SELL {} {} (reduceOnly)",
                qty, self.params.symbol
            );
            info!("DRY RUN: spot BUY {} {}", qty, self.params.symbol);
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        // 선물 청산 (reduceOnly)
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "SELL", qty, true)
            .await?;

        // 스팟 매수
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "BUY", qty, false)
            .await?;

        Ok((futures_order, spot_order))
    }

    /// 메인 베이시스 아비트라지 루프.
    ///
    /// 이 루프는 다음과 같은 순서로 동작한다:
    /// 1. 주기적으로(현재 1초 간격) 스팟 가격과 선물 마크 가격을 조회한다.
    /// 2. 베이시스(basis_bps)를 계산한다.
    ///    - basis_bps = (futures_mark - spot_price) / spot_price * 10000
    ///    - 양수(+)이면 선물이 스팟보다 프리미엄, 음수(-)이면 디스카운트 상태.
    /// 3. 현재 포지션 상태(ArbitrageState)를 참고해서:
    ///    - 포지션이 **없으면** 진입 조건(entry_bps)을,
    ///    - 포지션이 **있으면** 청산 조건(exit_bps)을 체크한다.
    ///
    /// 전략적으로 보면 이 루프는 "현·선물 베이시스 mean-reversion" 전략을 구현한다:
    ///
    /// - carry 모드("carry"):
    ///   - 조건: basis_bps > entry_bps
    ///     - 선물이 스팟보다 일정 bps 이상 비쌀 때 진입.
    ///   - 진입 시: 스팟 롱 + 선물 숏(open_carry)
    ///     - 스팟에서 symbol을 BUY
    ///     - 선물에서 같은 symbol을 SELL (숏)
    ///   - 청산 조건: basis_bps <= exit_bps
    ///     - 베이시스가 충분히 좁혀지면 포지션 정리.
    ///   - 청산 시: 스팟 매도 + 선물 롱으로 숏 reduce-only 청산(close_carry).
    ///
    /// - reverse 모드("reverse"):
    ///   - 조건: basis_bps < -entry_bps
    ///     - 선물이 스팟보다 일정 bps 이상 싸게(디스카운트) 거래될 때 진입.
    ///   - 진입 시: 스팟 숏(보유분만) + 선물 롱(open_reverse)
    ///     - 스팟은 보유 중인 base 자산만큼 SELL (공매도는 하지 않음)
    ///     - 선물에서 같은 symbol을 BUY (롱)
    ///   - 청산 조건: basis_bps >= -exit_bps
    ///     - 디스카운트가 줄어들면 포지션 정리.
    ///   - 청산 시: 스팟 매수 + 선물 숏으로 롱 reduce-only 청산(close_reverse).
    ///
    /// - auto 모드("auto"):
    ///   - basis_bps >  entry_bps 이면 carry 조건으로 진입,
    ///   - basis_bps < -entry_bps 이면 reverse 조건으로 진입.
    ///   - 이미 포지션이 열려 있을 때의 청산 로직은 상태(state.dir)가 "carry"/"reverse" 중
    ///     어느 방향인지에 따라 위와 동일하게 적용된다.
    ///
    /// 델타 관점:
    /// - carry: 스팟 롱 + 선물 숏 → 기본적으로 가격 방향성(델타)에 중립에 가깝고,
    ///   베이시스 축소와 펀딩(대부분 롱 → 숏 지불 구조)을 수익원으로 본다.
    /// - reverse: 스팟 숏(보유분만) + 선물 롱 → 마찬가지로 델타 중립에 가깝게 유지하면서
    ///   디스카운트 축소 및 펀딩 구조에 베팅한다.
    ///
    /// 상태 관리:
    /// - ArbitrageState를 통해 다음을 디스크(예: JSON 파일)로 유지한다.
    ///   - open: 포지션 보유 여부
    ///   - dir: "carry" 또는 "reverse"
    ///   - qty: 오픈 시 사용한 기준 수량
    ///   - last_open_basis_bps / last_close_basis_bps: 진입·청산 시점의 베이시스
    ///   - actions: 마지막 주문 응답(spot/futures)을 JSON으로 저장
    /// - run_loop 시작 시 기존 state를 읽어와서, 재시작해도 이전 포지션 상태를 이어간다.
    ///
    /// 실행 정책:
    /// - 어떤 주문 타입(시장가/지정가/post-only 등)으로 실제 주문을 집행할지는
    ///   StrategyParams.policy / spot_leg / futures_leg 및 BinanceTrader 구현에 위임한다.
    ///   (현재 open_carry/open_reverse는 전달받은 qty를 clamp 한 뒤
    ///    trader.place_spot_order / place_futures_order를 호출하는 형태로 동작하며,
    ///    dry_run 모드일 때는 실제 주문 대신 로그만 남기고 에러를 반환한다.)
    ///
    /// 주의사항:
    /// - 손절 조건(베이시스가 더 벌어질 때 강제 청산 등)은 포함되어 있지 않으며,
    ///   베이시스가 장기간 확장되는 경우 선물 측 마진 부족으로 청산 위험이 존재한다.
    /// - 수수료, 슬리피지, 펀딩 비용은 별도로 추적하지 않고, entry_bps/exit_bps 설정에
    ///   간접적으로 녹여서 사용해야 한다.
    pub async fn run_loop(&self) -> Result<(), ExchangeError> {
        // exchangeInfo 로드 (스팟 및 선물 LOT_SIZE 필터 캐싱)
        info!("Loading spot exchangeInfo...");
        self.trader.load_spot_exchange_info().await.map_err(|e| {
            ExchangeError::Other(format!("Failed to load spot exchangeInfo: {}", e))
        })?;

        info!("Loading futures exchangeInfo...");
        self.trader
            .load_futures_exchange_info()
            .await
            .map_err(|e| {
                ExchangeError::Other(format!("Failed to load futures exchangeInfo: {}", e))
            })?;

        // 선물 설정 확인
        self.trader
            .ensure_futures_setup(
                &self.params.symbol,
                self.params.leverage,
                self.params.isolated,
            )
            .await?;

        // 상태 로드
        let mut state = ArbitrageState::read()?;
        if state.symbol != self.params.symbol {
            state = ArbitrageState::new(self.params.symbol.clone());
        }

        info!("Starting basis arbitrage strategy");
        info!("Symbol: {}", self.params.symbol);
        info!("Mode: {}", self.params.mode);
        info!("Entry BPS: {}", self.params.entry_bps);
        info!("Exit BPS: {}", self.params.exit_bps);
        info!("Notional: {} USDT", self.params.notional);
        info!(
            "Current state: open={}, dir={:?}, qty={}",
            state.open, state.dir, state.qty
        );

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

            // 가격 조회
            let spot_price = self
                .trader
                .get_spot_price(&self.params.symbol)
                .await
                .map_err(|e| {
                    warn!("Failed to get spot price: {}", e);
                    e
                })?;

            let futures_mark = self
                .trader
                .get_futures_mark_price(&self.params.symbol)
                .await
                .map_err(|e| {
                    warn!("Failed to get futures mark price: {}", e);
                    e
                })?;

            let basis_bps = self.compute_basis_bps(spot_price, futures_mark);

            info!(
                "Spot: {:.2}, Futures: {:.2}, Basis: {:.2} bps",
                spot_price, futures_mark, basis_bps
            );

            if state.open {
                // 포지션이 열려있으면 청산 조건 확인
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
                        Ok((futures_order, spot_order)) => {
                            let actions = serde_json::json!({
                                "futures": futures_order,
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
                // 포지션이 없으면 진입 조건 확인
                let should_open_carry = (self.params.mode == "carry" || self.params.mode == "auto")
                    && basis_bps > self.params.entry_bps;

                let should_open_reverse = (self.params.mode == "reverse"
                    || self.params.mode == "auto")
                    && basis_bps < -self.params.entry_bps;

                if should_open_carry {
                    info!("Entry condition met for CARRY. Opening position...");
                    let qty = self.size_from_notional(spot_price);
                    match self.open_carry(qty).await {
                        Ok((spot_order, futures_order)) => {
                            let actions = serde_json::json!({
                                "spot": spot_order,
                                "futures": futures_order,
                            });

                            state.update_position(
                                true,
                                Some("carry".to_string()),
                                qty,
                                Some(basis_bps),
                                Some(actions),
                            );
                            state.write()?;
                            info!("CARRY position opened successfully");
                        }
                        Err(e) => {
                            warn!("Failed to open CARRY position: {}", e);
                        }
                    }
                } else if should_open_reverse {
                    info!("Entry condition met for REVERSE. Opening position...");
                    let qty = self.size_from_notional(spot_price);
                    match self.open_reverse(qty).await {
                        Ok((spot_order, futures_order)) => {
                            let actions = serde_json::json!({
                                "spot": spot_order,
                                "futures": futures_order,
                            });

                            state.update_position(
                                true,
                                Some("reverse".to_string()),
                                qty,
                                Some(basis_bps),
                                Some(actions),
                            );
                            state.write()?;
                            info!("REVERSE position opened successfully");
                        }
                        Err(e) => {
                            warn!("Failed to open REVERSE position: {}", e);
                        }
                    }
                }
            }
        }
    }
}
