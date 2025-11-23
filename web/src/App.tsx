import { useState, useEffect, useMemo } from "react";
import {
  Container,
  Title,
  Paper,
  Loader,
  Center,
  Alert,
  Text,
  Badge,
  Group,
  Chip,
  Stack,
} from "@mantine/core";
import { IconAlertCircle, IconRefresh } from "@tabler/icons-react";
import SnapshotTable from "./components/SnapshotTable";
import type { UnifiedSnapshot, ExchangeId } from "./types";

const EXCHANGES: ExchangeId[] = [
  "Binance",
  "Bybit",
  "Okx",
  "Bitget",
  "Bithumb",
];

function App() {
  const [snapshots, setSnapshots] = useState<UnifiedSnapshot[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdate, setLastUpdate] = useState<Date | null>(null);
  const [selectedExchanges, setSelectedExchanges] =
    useState<ExchangeId[]>(EXCHANGES);

  const fetchSnapshots = async () => {
    try {
      // const response = await fetch("https://koromo.cc/perp/unified-snapshots");
      const response = await fetch("http://localhost:12090/unified-snapshots");
      if (!response.ok) {
        throw new Error("데이터를 가져오는데 실패했습니다");
      }
      const data: UnifiedSnapshot[] = await response.json();
      setSnapshots(data);
      setError(null);
      setLastUpdate(new Date());
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "알 수 없는 오류가 발생했습니다"
      );
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchSnapshots();

    // 10초마다 자동 새로고침
    const interval = setInterval(fetchSnapshots, 10000);

    return () => clearInterval(interval);
  }, []);

  const filteredSnapshots = useMemo(() => {
    return snapshots.filter(
      (snapshot) =>
        selectedExchanges.includes(snapshot.exchange) &&
        (snapshot.perp !== null || snapshot.spot !== null) // perp 또는 spot이 있는 경우만 표시
    );
  }, [snapshots, selectedExchanges]);

  // 첫 번째 snapshot에서 환율 정보 가져오기
  const exchangeRates =
    snapshots.length > 0 ? snapshots[0].exchange_rates : null;

  const handleExchangeToggle = (exchange: ExchangeId) => {
    setSelectedExchanges((prev) =>
      prev.includes(exchange)
        ? prev.filter((e) => e !== exchange)
        : [...prev, exchange]
    );
  };

  if (loading) {
    return (
      <Center h="100vh">
        <Loader size="lg" />
      </Center>
    );
  }

  if (error) {
    return (
      <Container size="xl" py="xl">
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
      size="1600px"
      py="xl"
      className="fixed-width-container"
      style={{
        height: "100vh",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <Stack gap="md" style={{ flexShrink: 0 }}>
        <Group justify="space-between">
          <Title order={1}>Perp Scanner</Title>
          <Group gap="xs">
            {lastUpdate && (
              <Text size="sm" c="dimmed">
                마지막 업데이트: {lastUpdate.toLocaleTimeString("ko-KR")}
              </Text>
            )}
            <Badge
              color="blue"
              variant="light"
              leftSection={<IconRefresh size={12} />}
            >
              10초마다 자동 새로고침
            </Badge>
          </Group>
        </Group>
        {exchangeRates && (
          <Group gap="md" wrap="wrap">
            <Badge variant="light" color="gray">
              <Text size="xs" fw={500}>
                USD/KRW: ₩
                {exchangeRates.usd_krw.toLocaleString("ko-KR", {
                  minimumFractionDigits: 0,
                  maximumFractionDigits: 0,
                })}{" "}
                (exrate-api)
              </Text>
            </Badge>
            <Badge variant="light" color="blue">
              <Text size="xs" fw={500}>
                USDT/KRW: ₩
                {exchangeRates.usdt_krw.toLocaleString("ko-KR", {
                  minimumFractionDigits: 0,
                  maximumFractionDigits: 0,
                })}{" "}
                (
                {((exchangeRates.usdt_krw - exchangeRates.usd_krw) /
                  exchangeRates.usd_krw) *
                  100 >
                0
                  ? "+"
                  : ""}
                {(
                  ((exchangeRates.usdt_krw - exchangeRates.usd_krw) /
                    exchangeRates.usd_krw) *
                  100
                ).toFixed(2)}
                %, Bithumb)
              </Text>
            </Badge>
            <Badge variant="light" color="yellow">
              <Text size="xs" fw={500}>
                USDT/USD: {exchangeRates.usdt_usd.toFixed(4)} (Binance)
              </Text>
            </Badge>
          </Group>
        )}
        <Group gap="xs">
          <Text size="sm" fw={500}>
            거래소 필터:
          </Text>
          {EXCHANGES.map((exchange) => (
            <Chip
              key={exchange}
              checked={selectedExchanges.includes(exchange)}
              onChange={() => handleExchangeToggle(exchange)}
              variant="light"
            >
              {exchange}
            </Chip>
          ))}
        </Group>
      </Stack>
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
        }}
      >
        <SnapshotTable
          snapshots={filteredSnapshots}
          exchangeRates={exchangeRates}
        />
      </Paper>
    </Container>
  );
}

export default App;
