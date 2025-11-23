export type ExchangeId = 'Binance' | 'Bybit' | 'Okx' | 'Bitget' | 'Bithumb';
export type Currency = 'USD' | 'KRW' | 'USDT';

export interface PerpData {
  currency: Currency;
  mark_price: number;
  oi_usd: number;
  vol_24h_usd: number;
  funding_rate: number;
  next_funding_time: string | null;
}

export interface SpotData {
  currency: Currency;
  price: number;
  vol_24h_usd: number;
}

export interface ExchangeRates {
  usd_krw: number; // 1 USD = ? KRW (예: 1300.0)
  usdt_usd: number; // 1 USDT = ? USD (보통 1.0)
  usdt_krw: number; // 1 USDT = ? KRW (예: 1300.0)
  updated_at: string;
}

export interface UnifiedSnapshot {
  exchange: ExchangeId;
  symbol: string;
  currency: Currency;
  perp: PerpData | null;
  spot: SpotData | null;
  exchange_rates: ExchangeRates;
  updated_at: string;
}

export type SortField = 'exchange' | 'symbol' | 'mark_price' | 'spot_price' | 'kimchi_gap' | 'perp_spot_gap' | 'oi_usd' | 'vol_24h_usd' | 'funding_rate' | 'next_funding_time';
export type SortDirection = 'asc' | 'desc';

