# Arbitrage-2 Server (Rust)

암호화폐 거래소 간 가격 정보를 수집하고 베이시스(arbitrage) 전략을 실험하는 러스트 워크스페이스입니다. Oracle 서비스가 여러 거래소의 선물·현물 시세를 모아 HTTP로 제공하고, Trade CLI가 이 데이터를 활용해 자산 조회나 베이시스 전략 실행을 담당합니다.

## 프로젝트 구성
- `crates/interface`: 공용 데이터 모델(거래소 ID, 스냅샷, 환율, 수수료/자산 정보 등)과 에러 타입 정의.
- `crates/exchanges`: 거래소 클라이언트 모음. Binance, Bybit, OKX, Bitget, Bithumb의 선물(`PerpExchange`), 현물(`SpotExchange`), 자산, 호가창, 수수료 조회를 구현하고, 환율 조회(`exchange_rate`) 유틸을 포함합니다.
- `crates/oracle`: 백그라운드 루프가 주기적으로 각 거래소 시세를 수집·정렬하고 환율을 덧붙여 `UnifiedSnapshot`으로 합칩니다. Axum 기반 HTTP 서버로 스냅샷을 노출합니다.
- `crates/trade`: CLI 도구. `explore` 서브커맨드로 Oracle 스냅샷/거래소 자산을 조회하고, `arbitrage` 모듈에 Binance 기반 베이시스 전략(`BasisArbitrageStrategy`)을 담고 있습니다. 실행 상태는 `arb_state.json`에 기록됩니다.

## 필수 요건
- Rust 1.76+ (stable 권장)
- 네트워크로 거래소 공개/인증 API 접근 가능해야 합니다.
- `.env` 또는 환경변수에 거래소 키를 설정하세요 (실제 키는 버전에 올리지 마세요).
  - `BINANCE_API_KEY`, `BINANCE_API_SECRET` (선물·현물 둘 다 사용)
  - `BITHUMB_API_KEY`, `BITHUMB_API_SECRET`
  - 그 외 공개 API는 키 없이 동작하지만, 자산 조회나 주문 관련 기능은 키가 필요합니다.

## 실행 방법
1) Oracle 서버 기동 (시세 수집 + HTTP 제공)  
```bash
cargo run -p oracle
```
- 기본 포트: `12090`
- 엔드포인트:
  - `/health` : 상태 체크
  - `/snapshots` : 선물 스냅샷 목록
  - `/spot-snapshots` : 현물 스냅샷 목록
  - `/unified-snapshots` : 선물·현물·환율을 합친 스냅샷

2) Trade CLI 사용 예시  
```bash
# Bithumb / Binance 자산 조회 (인증 키 필요)
cargo run -p trade -- explore-test

# 베이시스 전략 파라미터 확인만 하는 드라이런
cargo run -p trade -- arbitrage-test
```
- `trade run` 커맨드는 베이시스 전략 실행을 위한 자리이며 현재 `todo!()`로 구현이 남아 있습니다. 실제 자동 매매를 붙일 때 `BasisArbitrageStrategy::run_loop`를 호출하도록 확장하면 됩니다.

## 동작 흐름 개요
1. Oracle(`crates/oracle`)이 10초 간격으로 각 거래소의 선물/현물 시세를 가져와 정렬 후 메모리에 보관합니다. 동시에 USD/KRW, USDT/USD, USDT/KRW 환율을 조회합니다.  
2. 수집된 데이터를 `/unified-snapshots` 등 HTTP 엔드포인트로 제공합니다.  
3. Trade CLI(`crates/trade`)는 Oracle을 조회하거나 거래소 인증 API를 직접 호출해 자산/주문을 처리하고, 베이시스 전략은 Binance 선물·현물 양쪽을 사용해 진입/청산을 결정합니다.

## 기타
- `build.sh`/`build.bat`, `Dockerfile`가 포함되어 있지만 현재 워크스페이스 구조와 완전히 맞지 않을 수 있으니 사용 전 경로를 검토하세요.
- 실계정 키가 담긴 `.env`는 절대 커밋하지 마세요.
