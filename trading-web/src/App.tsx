import { useState, useRef, useEffect, useMemo } from "react";
import {
  Container,
  Title,
  Paper,
  Button,
  Stack,
  Tabs,
  Group,
  Text,
  Modal,
  Code,
  ScrollArea,
  Divider,
  Badge,
  Grid,
  Loader,
  Alert,
} from "@mantine/core";
import { IconTrash, IconAlertCircle } from "@tabler/icons-react";
import TradeTable from "./components/TradeTable";
import PositionTable from "./components/PositionTable";
import ConsolePanel from "./components/ConsolePanel";
import type { TradeRecord, PositionRecord, ConsoleLog } from "./types";

// 더미 콘솔 로그 생성 함수 (서버에서 제공하지 않으므로 유지)
function generateDummyConsoleLogs(): ConsoleLog[] {
  const levels: ConsoleLog["level"][] = ["info", "warn", "error", "success"];
  const messages = [
    "거래 실행 완료: BTC/USDT 매수 0.5개 @ 45,000 USDT",
    "포지션 오픈: intra_basis 봇이 ETH 포지션을 열었습니다",
    "경고: Binance API 응답 지연 감지",
    "에러: 거래소 연결 실패 - 재시도 중...",
    "성공: 모든 포지션이 정상적으로 청산되었습니다",
    "정보: 새로운 차익거래 기회 발견",
    "경고: 손실 한도에 근접했습니다",
    "정보: 자산 잔액 확인 완료",
  ];

  return Array.from({ length: 20 }, (_, i) => ({
    timestamp: new Date(Date.now() - i * 30000).toISOString(),
    level: levels[Math.floor(Math.random() * levels.length)],
    message: messages[Math.floor(Math.random() * messages.length)],
  }));
}

function App() {
  const [tradeRecords, setTradeRecords] = useState<TradeRecord[]>([]);
  const [positionRecords, setPositionRecords] = useState<PositionRecord[]>([]);
  const [consoleLogs] = useState<ConsoleLog[]>(generateDummyConsoleLogs());
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isLiquidationLoading, setIsLiquidationLoading] = useState(false);
  const [consoleHeight, setConsoleHeight] = useState(200);
  const [isResizing, setIsResizing] = useState(false);
  const [selectedPositionId, setSelectedPositionId] = useState<number | null>(
    null
  );
  const [selectedTradeRecord, setSelectedTradeRecord] =
    useState<TradeRecord | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const resizeBarRef = useRef<HTMLDivElement>(null);
  const startYRef = useRef<number>(0);
  const startHeightRef = useRef<number>(200);

  // API에서 데이터 로드
  useEffect(() => {
    const loadData = async () => {
      setIsLoading(true);
      setError(null);
      try {
        const [tradeResponse, positionResponse] = await Promise.all([
          fetch("/api/trade-records"),
          fetch("/api/position-records"),
        ]);

        if (!tradeResponse.ok) {
          throw new Error(`거래 기록 조회 실패: ${tradeResponse.statusText}`);
        }
        if (!positionResponse.ok) {
          throw new Error(
            `포지션 기록 조회 실패: ${positionResponse.statusText}`
          );
        }

        const tradeData: TradeRecord[] = await tradeResponse.json();
        const positionData: PositionRecord[] = await positionResponse.json();

        setTradeRecords(tradeData);
        setPositionRecords(positionData);
      } catch (err) {
        const errorMessage =
          err instanceof Error
            ? err.message
            : "데이터를 불러오는데 실패했습니다";
        setError(errorMessage);
        console.error("데이터 로드 오류:", err);
      } finally {
        setIsLoading(false);
      }
    };

    loadData();
  }, []);

  const handleForceLiquidation = async () => {
    setIsLiquidationLoading(true);
    // 실제로는 서버 API를 호출할 예정
    await new Promise((resolve) => setTimeout(resolve, 2000));
    setIsLiquidationLoading(false);
    alert("모든 자산이 강제 청산되었습니다.");
  };

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isResizing || !containerRef.current || !resizeBarRef.current) return;

      const containerRect = containerRef.current.getBoundingClientRect();
      const containerHeight = containerRect.height;

      // 마우스 위치에서 컨테이너 하단까지의 거리 = 콘솔 높이
      const containerBottom = containerRect.bottom;
      const newHeight = containerBottom - e.clientY;

      // 최소/최대 높이 제한
      const minHeight = 100;
      const maxHeight = containerHeight * 0.7; // 최대 70%까지
      const clampedHeight = Math.max(minHeight, Math.min(maxHeight, newHeight));

      setConsoleHeight(clampedHeight);
    };

    const handleMouseUp = () => {
      setIsResizing(false);
    };

    if (isResizing) {
      document.addEventListener("mousemove", handleMouseMove);
      document.addEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "row-resize";
      document.body.style.userSelect = "none";
    }

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [isResizing]);

  const handleResizeStart = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
    startYRef.current = e.clientY;
    startHeightRef.current = consoleHeight;
  };

  // 선택된 포지션과 관련된 거래 기록 필터링
  const relatedTradeRecords = useMemo(() => {
    if (selectedPositionId === null) return [];

    const selectedPosition = positionRecords.find(
      (p) => p.id === selectedPositionId
    );
    if (!selectedPosition) return [];

    // 같은 bot_name, symbol, carry 조합의 모든 포지션을 시간순으로 정렬
    const sameGroupPositions = positionRecords
      .filter(
        (p) =>
          p.bot_name === selectedPosition.bot_name &&
          p.symbol === selectedPosition.symbol &&
          p.carry === selectedPosition.carry
      )
      .sort((a, b) => {
        return (
          new Date(a.executed_at).getTime() - new Date(b.executed_at).getTime()
        );
      });

    // 선택된 포지션의 인덱스 찾기
    const currentIndex = sameGroupPositions.findIndex(
      (p) => p.id === selectedPositionId
    );

    if (currentIndex === -1) return [];

    // 선택된 포지션의 실행 시간 (이 시간 이전의 거래 기록)
    const endTime = new Date(selectedPosition.executed_at).getTime();

    // 이전 포지션의 시작 시간 찾기
    let startTime: number;

    if (currentIndex > 0) {
      // 같은 조합의 이전 포지션이 있으면 그 포지션의 시작 시간 (포함)
      startTime = new Date(
        sameGroupPositions[currentIndex - 1].executed_at
      ).getTime();
    } else {
      // 이전 포지션이 없으면 0부터 (모든 거래 기록)
      startTime = 0;
    }

    // 디버깅: 현재 로직 확인
    console.log("=== 포지션 필터링 디버깅 ===");
    console.log("선택된 포지션:", {
      id: selectedPosition.id,
      executed_at: selectedPosition.executed_at,
      bot_name: selectedPosition.bot_name,
      symbol: selectedPosition.symbol,
      carry: selectedPosition.carry,
    });
    console.log(
      "같은 그룹 포지션들:",
      sameGroupPositions.map((p) => ({
        id: p.id,
        executed_at: p.executed_at,
      }))
    );
    console.log("현재 인덱스:", currentIndex);
    console.log(
      "시작 시간:",
      startTime > 0 ? new Date(startTime).toISOString() : "0 (시작)"
    );
    console.log("종료 시간:", new Date(endTime).toISOString());
    console.log(
      "이전 포지션:",
      currentIndex > 0
        ? {
            id: sameGroupPositions[currentIndex - 1].id,
            executed_at: sameGroupPositions[currentIndex - 1].executed_at,
          }
        : "없음"
    );

    // 이전 포지션의 실행 시간 이후부터 선택된 포지션의 실행 시간 이전까지의 거래 기록 필터링
    const filtered = tradeRecords.filter((trade) => {
      const tradeTime = new Date(trade.executed_at).getTime();
      // startTime 이상이고 endTime 미만 (포지션 실행 시간 이전의 거래 기록)
      return tradeTime >= startTime && tradeTime < endTime;
    });

    console.log("필터링된 거래 기록 수:", filtered.length);
    console.log(
      "필터링된 거래 기록 시간 범위:",
      filtered.length > 0
        ? {
            첫번째: new Date(filtered[0].executed_at).toISOString(),
            마지막: new Date(
              filtered[filtered.length - 1].executed_at
            ).toISOString(),
          }
        : "없음"
    );
    console.log("===========================");

    return filtered;
  }, [selectedPositionId, positionRecords, tradeRecords]);

  if (isLoading) {
    return (
      <Container
        size="1600px"
        py="xl"
        style={{
          height: "100vh",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <Stack align="center" gap="md">
          <Loader size="lg" />
          <Text size="lg" c="dimmed">
            데이터를 불러오는 중...
          </Text>
        </Stack>
      </Container>
    );
  }

  if (error) {
    return (
      <Container size="1600px" py="xl">
        <Alert
          icon={<IconAlertCircle size={16} />}
          title="오류 발생"
          color="red"
        >
          {error}
        </Alert>
      </Container>
    );
  }

  return (
    <Container
      ref={containerRef}
      size="1600px"
      py="xl"
      style={{
        height: "100vh",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Stack gap="md" style={{ flexShrink: 0 }}>
        <Group justify="space-between">
          <Title order={1}>트레이딩 봇 대시보드</Title>
          <Button
            leftSection={<IconTrash size={16} />}
            color="red"
            variant="filled"
            loading={isLiquidationLoading}
            onClick={handleForceLiquidation}
          >
            모든 자산 강제 청산
          </Button>
        </Group>
      </Stack>

      <div
        style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          minHeight: 0,
          overflow: "hidden",
        }}
      >
        <Paper
          shadow="sm"
          p="md"
          withBorder
          mt="md"
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            minHeight: 0,
            overflow: "hidden",
          }}
        >
          <Tabs
            defaultValue="trades"
            style={{ height: "100%", display: "flex", flexDirection: "column" }}
          >
            <Tabs.List>
              <Tabs.Tab value="trades">거래 기록</Tabs.Tab>
              <Tabs.Tab value="positions">포지션 기록</Tabs.Tab>
            </Tabs.List>

            <Tabs.Panel
              value="trades"
              pt="md"
              style={{
                flex: 1,
                minHeight: 0,
                display: "flex",
                flexDirection: "column",
              }}
            >
              <Paper
                shadow="xs"
                p="md"
                withBorder
                style={{
                  flex: 1,
                  minHeight: 0,
                  display: "flex",
                  flexDirection: "column",
                }}
              >
                <Text fw={600} size="lg" mb="md" style={{ flexShrink: 0 }}>
                  거래 기록
                </Text>
                <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
                  <TradeTable
                    records={tradeRecords}
                    onRecordDoubleClick={setSelectedTradeRecord}
                  />
                </div>
              </Paper>
            </Tabs.Panel>

            <Tabs.Panel
              value="positions"
              pt="md"
              style={{
                flex: 1,
                minHeight: 0,
                display: "flex",
                flexDirection: "column",
              }}
            >
              <Paper
                shadow="xs"
                p="md"
                withBorder
                style={{
                  flex: 1,
                  minHeight: 0,
                  display: "flex",
                  flexDirection: "column",
                }}
              >
                <Text fw={600} size="lg" mb="md" style={{ flexShrink: 0 }}>
                  포지션 기록
                </Text>
                <div
                  style={{
                    flex: selectedPositionId ? 0.5 : 1,
                    minHeight: 0,
                    overflow: "hidden",
                    display: "flex",
                    flexDirection: "column",
                  }}
                >
                  <PositionTable
                    records={positionRecords}
                    selectedPositionId={selectedPositionId}
                    onPositionClick={setSelectedPositionId}
                  />
                </div>
                {selectedPositionId && relatedTradeRecords.length > 0 && (
                  <div
                    style={{
                      flex: 0.5,
                      minHeight: 0,
                      overflow: "hidden",
                      marginTop: "16px",
                      display: "flex",
                      flexDirection: "column",
                    }}
                  >
                    <Text fw={600} size="md" mb="md" style={{ flexShrink: 0 }}>
                      관련 거래 기록 ({relatedTradeRecords.length}건)
                    </Text>
                    <div
                      style={{
                        flex: 1,
                        minHeight: 0,
                        overflow: "hidden",
                      }}
                    >
                      <TradeTable
                        records={relatedTradeRecords}
                        onRecordDoubleClick={setSelectedTradeRecord}
                      />
                    </div>
                  </div>
                )}
                {selectedPositionId && relatedTradeRecords.length === 0 && (
                  <div
                    style={{
                      marginTop: "16px",
                      padding: "16px",
                      textAlign: "center",
                    }}
                  >
                    <Text c="dimmed" size="sm">
                      선택한 포지션과 관련된 거래 기록이 없습니다.
                    </Text>
                  </div>
                )}
              </Paper>
            </Tabs.Panel>
          </Tabs>
        </Paper>

        <div
          ref={resizeBarRef}
          style={{
            height: "4px",
            backgroundColor: "var(--mantine-color-gray-6)",
            cursor: "row-resize",
            position: "relative",
            marginTop: "8px",
            flexShrink: 0,
          }}
          onMouseDown={handleResizeStart}
        >
          <div
            style={{
              position: "absolute",
              top: "-2px",
              left: 0,
              right: 0,
              height: "8px",
              zIndex: 1,
            }}
          />
        </div>
        <Paper
          shadow="sm"
          p="md"
          withBorder
          style={{
            flexShrink: 0,
            height: `${consoleHeight}px`,
            minHeight: "100px",
          }}
        >
          <ConsolePanel logs={consoleLogs} />
        </Paper>
      </div>

      <Modal
        opened={selectedTradeRecord !== null}
        onClose={() => setSelectedTradeRecord(null)}
        title={
          <Text fw={700} size="xl">
            거래 기록 상세 정보
          </Text>
        }
        size="70%"
        styles={{
          content: {
            maxHeight: "90vh",
          },
          body: {
            padding: "24px",
          },
        }}
      >
        {selectedTradeRecord && (
          <ScrollArea h="calc(90vh - 120px)">
            <Grid gutter="lg">
              {/* 왼쪽: 기본 정보 및 거래 정보 */}
              <Grid.Col span={6}>
                <Stack gap="md">
                  {/* 기본 정보 카드 */}
                  <Paper p="lg" withBorder shadow="sm">
                    <Text fw={700} size="md" mb="lg">
                      기본 정보
                    </Text>
                    <Stack gap="md">
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          ID
                        </Text>
                        <Text size="sm" fw={500}>
                          #{selectedTradeRecord.id}
                        </Text>
                      </Group>
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          실행 시간
                        </Text>
                        <Text size="sm" fw={500}>
                          {new Date(
                            selectedTradeRecord.executed_at
                          ).toLocaleString("ko-KR")}
                        </Text>
                      </Group>
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          거래소
                        </Text>
                        <Badge variant="light" color="blue" size="md">
                          {selectedTradeRecord.exchange}
                        </Badge>
                      </Group>
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          심볼
                        </Text>
                        <Text size="md" fw={600}>
                          {selectedTradeRecord.symbol}
                        </Text>
                      </Group>
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          마켓 타입
                        </Text>
                        <Badge
                          variant="light"
                          color={
                            selectedTradeRecord.market_type === "SPOT"
                              ? "green"
                              : "orange"
                          }
                          size="md"
                        >
                          {selectedTradeRecord.market_type}
                        </Badge>
                      </Group>
                    </Stack>
                  </Paper>

                  {/* 거래 정보 카드 */}
                  <Paper p="lg" withBorder shadow="sm">
                    <Text fw={700} size="md" mb="lg">
                      거래 정보
                    </Text>
                    <Stack gap="md">
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          방향
                        </Text>
                        <Badge
                          variant="light"
                          color={
                            selectedTradeRecord.side === "BUY" ? "green" : "red"
                          }
                          size="md"
                        >
                          {selectedTradeRecord.side}
                        </Badge>
                      </Group>
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          거래 유형
                        </Text>
                        <Badge variant="light" color="gray" size="md">
                          {selectedTradeRecord.trade_type}
                        </Badge>
                      </Group>
                      <Group justify="space-between">
                        <Text size="sm" c="dimmed">
                          청산 여부
                        </Text>
                        <Badge
                          variant={
                            selectedTradeRecord.is_liquidation
                              ? "filled"
                              : "light"
                          }
                          color={
                            selectedTradeRecord.is_liquidation ? "red" : "gray"
                          }
                          size="md"
                        >
                          {selectedTradeRecord.is_liquidation ? "청산" : "일반"}
                        </Badge>
                      </Group>
                      <Divider />
                      <Stack gap={4}>
                        <Text size="sm" c="dimmed">
                          실행 가격
                        </Text>
                        <Text size="xl" fw={700} c="blue">
                          {selectedTradeRecord.executed_price
                            ? selectedTradeRecord.executed_price.toLocaleString(
                                "ko-KR",
                                {
                                  minimumFractionDigits: 2,
                                  maximumFractionDigits: 8,
                                }
                              )
                            : "N/A"}
                        </Text>
                      </Stack>
                      <Stack gap={4}>
                        <Text size="sm" c="dimmed">
                          수량
                        </Text>
                        <Text size="xl" fw={700} c="green">
                          {selectedTradeRecord.quantity.toLocaleString(
                            "ko-KR",
                            {
                              minimumFractionDigits: 2,
                              maximumFractionDigits: 8,
                            }
                          )}
                        </Text>
                      </Stack>
                    </Stack>
                  </Paper>
                </Stack>
              </Grid.Col>

              {/* 오른쪽: 요청/응답 정보 */}
              <Grid.Col span={6}>
                {(selectedTradeRecord.request_query_string ||
                  selectedTradeRecord.api_response ||
                  selectedTradeRecord.metadata) && (
                  <Stack gap="md">
                    {selectedTradeRecord.request_query_string && (
                      <Paper p="lg" withBorder shadow="sm">
                        <Text fw={700} size="md" mb="md">
                          요청 쿼리 스트링
                        </Text>
                        <Paper
                          p="md"
                          withBorder
                          style={{
                            backgroundColor: "var(--mantine-color-dark-8)",
                          }}
                        >
                          <Code
                            block
                            style={{
                              whiteSpace: "pre-wrap",
                              fontSize: "12px",
                              fontFamily: "monospace",
                            }}
                          >
                            {selectedTradeRecord.request_query_string
                              .split("&")
                              .join("\n")}
                          </Code>
                        </Paper>
                      </Paper>
                    )}

                    {selectedTradeRecord.api_response && (
                      <Paper p="lg" withBorder shadow="sm">
                        <Text fw={700} size="md" mb="md">
                          API 응답
                        </Text>
                        <Paper
                          p="md"
                          withBorder
                          style={{
                            backgroundColor: "var(--mantine-color-dark-8)",
                          }}
                        >
                          <Code
                            block
                            style={{
                              whiteSpace: "pre-wrap",
                              fontSize: "12px",
                              fontFamily: "monospace",
                            }}
                          >
                            {(() => {
                              try {
                                const parsed = JSON.parse(
                                  selectedTradeRecord.api_response
                                );
                                return JSON.stringify(parsed, null, 2);
                              } catch {
                                return selectedTradeRecord.api_response;
                              }
                            })()}
                          </Code>
                        </Paper>
                      </Paper>
                    )}

                    {selectedTradeRecord.metadata && (
                      <Paper p="lg" withBorder shadow="sm">
                        <Text fw={700} size="md" mb="md">
                          메타데이터
                        </Text>
                        <Paper
                          p="md"
                          withBorder
                          style={{
                            backgroundColor: "var(--mantine-color-dark-8)",
                          }}
                        >
                          <Code
                            block
                            style={{
                              whiteSpace: "pre-wrap",
                              fontSize: "12px",
                              fontFamily: "monospace",
                            }}
                          >
                            {(() => {
                              try {
                                const parsed = JSON.parse(
                                  selectedTradeRecord.metadata
                                );
                                return JSON.stringify(parsed, null, 2);
                              } catch {
                                return selectedTradeRecord.metadata;
                              }
                            })()}
                          </Code>
                        </Paper>
                      </Paper>
                    )}
                  </Stack>
                )}
                {!selectedTradeRecord.request_query_string &&
                  !selectedTradeRecord.api_response &&
                  !selectedTradeRecord.metadata && (
                    <Paper p="xl" withBorder>
                      <Text c="dimmed" ta="center">
                        추가 정보가 없습니다
                      </Text>
                    </Paper>
                  )}
              </Grid.Col>
            </Grid>
          </ScrollArea>
        )}
      </Modal>
    </Container>
  );
}

export default App;
