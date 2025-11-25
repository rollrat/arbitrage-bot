use interface::ExchangeError;
use serde_json;
use tracing::{info, warn};

use super::super::state::ArbitrageState;
use super::{StrategyMode, StrategyParams};
use crate::trader::binance::HedgedPair;
use crate::trader::{BinanceTrader, OrderResponse};

/// 단일 거래소(Binance) 안에서 스팟/선물 간 베이시스(가격 격차)를 이용해
/// 델타-뉴트럴 포지션을 자동으로 관리하는 인트라(intra) 베이시스 아비트라지 전략.
///
/// 구조적으로는 CrossBasisArbitrageStrategy 와 매우 비슷하지만,
/// - 모든 주문/포지션/마진/청산이 **하나의 거래소·하나의 계정** 안에서 일어나고
/// - FX(원화/김프, USDT 프리미엄 등)나 거래소 간 전송/규제 리스크가 없다는 점에서
///   훨씬 단순한 형태의 베이시스 캐리/리버스 전략을 구현한다.
///
/// 핵심 역할:
/// - `trader: BinanceTrader`
///   - Binance 현물(spot) + 선물(futures) API 를 둘 다 다루는 공용 트레이더.
///   - 스팟/선물 exchangeInfo 로부터 LOT_SIZE/stepSize 등을 읽어와 수량을 clamp 하고,
///     스팟/선물 주문, 잔고 조회, 마크 가격 조회 등을 담당.
/// - `params: StrategyParams`
///   - 대상 심볼(symbol), 진입/청산 기준 bps(entry_bps/exit_bps),
///     명목가(notional), 레버리지(leverage), 마진 모드(isolated), 전략 모드(mode),
///     dry_run 여부 등을 포함하는 런타임 설정값.
///
/// 전략 개념:
/// - 한 거래소 안에서 **spot vs futures** 베이시스를 보고,
///   - 선물이 스팟보다 충분히 비싸면 → 캐리(CARRY) 포지션
///   - 선물이 스팟보다 충분히 싸면 → 리버스(REVERSE) 포지션
///   에 진입한 뒤, 베이시스가 다시 좁혀지면 청산하는 **mean-reversion 전략**.
/// - 모든 것이 같은 기축(USDT) 기준이기 때문에,
///   CrossBasis 와 달리 fx_adjustment, cross 계정 재고, 출금/전송 리스크를
///   따로 고려할 필요가 없다.
///
/// 포지션 구조:
/// - CARRY (선물 프리미엄을 먹는 방향)
///   - 진입: `open_carry()`
///     - 스팟: symbol **BUY**
///     - 선물: symbol **SELL**
///     - 스팟 롱 + 선물 숏 → 가격 방향(델타)은 중립에 가깝고,
///       베이시스 축소 및 펀딩 구조를 수익원으로 본다.
///   - 청산: `close_carry()`
///     - 선물: **BUY reduce-only** 로 숏 포지션 해소
///     - 스팟: **SELL** 로 롱 포지션 해소
///
/// - REVERSE (선물 디스카운트를 먹는 방향, 보유 스팟을 활용)
///   - 진입: `open_reverse()`
///     - 스팟: 보유 중인 base 자산(free balance) 한도 내에서 **SELL**
///       (공매도는 하지 않고, inventory-based short 만 허용)
///     - 선물: symbol **BUY**
///     - 스팟 숏(보유분) + 선물 롱 → 마찬가지로 델타는 중립에 가까우며,
///       디스카운트 축소 및 펀딩 구조에 베팅.
///   - 청산: `close_reverse()`
///     - 선물: **SELL reduce-only** 로 롱 포지션 해소
///     - 스팟: **BUY** 로 초기 매도분을 다시 매수
///
/// 주요 메서드:
/// - `compute_basis_bps(spot_price, futures_mark)`
///   - basis_bps = (futures_mark - spot_price) / spot_price * 10_000
///   - 양수(+)면 선물 프리미엄, 음수(-)면 선물 디스카운트 상태를 의미.
/// - `size_from_notional(spot_price)`
///   - params.notional(USDT 기준 명목가)을 spot_price 로 나누어 기준 수량을 계산하고,
///     거래소 LOT_SIZE 규칙에 맞게 clamp 한 최종 주문 수량을 리턴.
/// - `open_carry` / `close_carry`
///   - CARRY 포지션의 진입/청산을 담당.
///   - 스팟/선물 각각의 clamp 결과 중 더 작은 수량을 사용해
///     델타를 최대한 중립에 맞춘다.
/// - `open_reverse` / `close_reverse`
///   - REVERSE 포지션의 진입/청산을 담당.
///   - 스팟은 **현재 보유 중인 base 자산(free balance)** 한도 내에서만 매도하며,
///     선물 레그와의 clamp 결과를 다시 최소값으로 맞춰 델타를 줄인다.
///
/// 메인 루프(`run_loop`):
/// - 1초 간격으로 spot / futures mark 가격을 조회하고,
///   `compute_basis_bps` 로 베이시스를 계산한다.
/// - ArbitrageState 를 파일로 읽고/쓰면서,
///   - 현재 포지션 유무(open),
///   - 방향(dir = "carry"/"reverse"),
///   - 수량(qty),
///   - 진입/청산 시점의 basis_bps,
///   - 마지막 주문 응답(actions: {spot, futures})
///   를 지속적으로 기록한다.
/// - 포지션이 없을 때:
///   - StrategyMode::Carry / Reverse / Auto 에 따라
///     entry_bps 초과/미만 조건을 만족하면 `open_carry` 또는 `open_reverse` 호출.
/// - 포지션이 있을 때:
///   - dir 에 따라 exit_bps 조건(basis_bps <= exit_bps, 또는 >= -exit_bps)을 체크하고
///     `close_carry` / `close_reverse` 를 호출해 포지션을 닫는다.
pub struct IntraBasisArbitrageStrategy {
    trader: BinanceTrader,
    params: StrategyParams,
}

impl IntraBasisArbitrageStrategy {
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
    ) -> Result<(OrderResponse, OrderResponse, HedgedPair), ExchangeError> {
        info!(
            "Opening CARRY position: spot BUY {} {}, futures SELL {} {}",
            qty, self.params.symbol, qty, self.params.symbol
        );

        let fee = self
            .trader
            .spot_client
            .get_trade_fee_for_symbol(&self.params.symbol)
            .await?;

        let spot_fee_rate = match self.params.mode {
            StrategyMode::Carry => fee.taker,
            _ => fee.maker,
        };

        // 스팟과 선물의 수량을 각각 clamp하고, 더 작은 쪽 사용
        let pair = self
            .trader
            .find_hedged_pair(&self.params.symbol, qty, spot_fee_rate)
            .ok_or_else(|| ExchangeError::Other("Failed to find hedged pair".into()))?;

        if self.params.dry_run {
            info!("DRY RUN: pair: {:?}", pair);
            info!("DRY RUN: spot BUY {} {}", qty, self.params.symbol);
            info!("DRY RUN: futures SELL {} {}", qty, self.params.symbol);
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        if pair.spot_net_qty_est <= 0.0 {
            return Err(ExchangeError::Other(format!(
                "Quantity too small after clamping. Increase notional. spot_qty={}, fut_qty={}",
                pair.spot_order_qty, pair.fut_order_qty
            )));
        }

        // 스팟 매수
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "BUY", pair.spot_order_qty, false)
            .await?;

        // 선물 숏
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "SELL", pair.fut_order_qty, false)
            .await?;

        // TODO: 선물 실패 처리, 트랜잭션

        // TODO: delta_est 어떻게 처리할 지 고민하기

        Ok((spot_order, futures_order, pair))
    }

    /// Carry 포지션 클로즈: 스팟 매도 + 선물 매수 (reduceOnly)
    pub async fn close_carry(
        &self,
        pair: HedgedPair,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!(
            "Closing CARRY position: spot SELL {} {}, futures BUY {} {} (reduceOnly)",
            pair.spot_order_qty, self.params.symbol, pair.fut_order_qty, self.params.symbol
        );

        if self.params.dry_run {
            info!(
                "DRY RUN: futures BUY {} {} (reduceOnly)",
                pair.fut_order_qty, self.params.symbol
            );
            info!(
                "DRY RUN: spot SELL {} {}",
                pair.spot_order_qty, self.params.symbol
            );
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        let spot_sell_qty = self
            .trader
            .clamp_spot_quantity(&self.params.symbol, pair.spot_net_qty_est);

        // 스팟 매도
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "SELL", spot_sell_qty, false)
            .await?;

        // 선물 청산 (reduceOnly)
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "BUY", pair.fut_order_qty, true)
            .await?;

        Ok((futures_order, spot_order))
    }

    /// Reverse 포지션 오픈: 스팟 숏(보유분만) + 선물 롱
    pub async fn open_reverse(
        &self,
        qty: f64,
    ) -> Result<(OrderResponse, OrderResponse, HedgedPair), ExchangeError> {
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

        Ok((spot_order, futures_order, todo!()))
    }

    /// Reverse 포지션 클로즈: 스팟 매수 + 선물 매도 (reduceOnly)
    pub async fn close_reverse(
        &self,
        pair: HedgedPair,
    ) -> Result<(OrderResponse, OrderResponse), ExchangeError> {
        info!(
            "Closing REVERSE position: spot BUY {} {}, futures SELL {} {} (reduceOnly)",
            pair.spot_order_qty, self.params.symbol, pair.fut_order_qty, self.params.symbol
        );

        if self.params.dry_run {
            info!(
                "DRY RUN: futures SELL {} {} (reduceOnly)",
                pair.fut_order_qty, self.params.symbol
            );
            info!(
                "DRY RUN: spot BUY {} {}",
                pair.spot_order_qty, self.params.symbol
            );
            return Err(ExchangeError::Other("Dry run mode".to_string()));
        }

        // 선물 청산 (reduceOnly)
        let futures_order = self
            .trader
            .place_futures_order(&self.params.symbol, "SELL", pair.fut_order_qty, true)
            .await?;

        // 스팟 매수
        let spot_order = self
            .trader
            .place_spot_order(&self.params.symbol, "BUY", pair.spot_order_qty, false)
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
            "Current state: open={}, dir={:?}, pair={:?}",
            state.open, state.dir, state.pair
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
                        Some("carry") => self.close_carry(state.pair).await,
                        Some("reverse") => self.close_reverse(state.pair).await,
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

                            state.update_position(
                                false,
                                None,
                                Default::default(),
                                Some(basis_bps),
                                Some(actions),
                            );
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
                let should_open_carry =
                    matches!(self.params.mode, StrategyMode::Carry | StrategyMode::Auto)
                        && basis_bps > self.params.entry_bps;

                let should_open_reverse =
                    matches!(self.params.mode, StrategyMode::Reverse | StrategyMode::Auto)
                        && basis_bps < -self.params.entry_bps;

                if should_open_carry {
                    info!("Entry condition met for CARRY. Opening position...");
                    let qty = self.size_from_notional(spot_price);
                    match self.open_carry(qty).await {
                        Ok((spot_order, futures_order, pair)) => {
                            let actions = serde_json::json!({
                                "spot": spot_order,
                                "futures": futures_order,
                            });

                            state.update_position(
                                true,
                                Some("carry".to_string()),
                                pair,
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
                        Ok((spot_order, futures_order, pair)) => {
                            let actions = serde_json::json!({
                                "spot": spot_order,
                                "futures": futures_order,
                            });

                            state.update_position(
                                true,
                                Some("reverse".to_string()),
                                pair,
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
