/**
 * BSV/USD rate fetching and conversion utilities.
 * Uses the server-side /rates/bsv-usd proxy to avoid CORS.
 */

import type { CurrencyDisplay } from './storage.js';
import { formatSats } from './util.js';

let cachedRate = 0;
let lastFetch = 0;
const CACHE_TTL = 300_000; // 5 minutes
const FALLBACK_RATE = 20.0;

/**
 * Fetch current BSV/USD rate. Caches for 5 minutes.
 */
export async function fetchBsvUsdRate(): Promise<number> {
  const now = Date.now();
  if (cachedRate > 0 && now - lastFetch < CACHE_TTL) {
    return cachedRate;
  }
  try {
    const resp = await fetch('/rates/bsv-usd');
    if (resp.ok) {
      const data = await resp.json();
      cachedRate = data.rate;
      lastFetch = now;
      return cachedRate;
    }
  } catch {
    // Fall through to fallback
  }
  return cachedRate > 0 ? cachedRate : FALLBACK_RATE;
}

/**
 * Convert satoshis to USD. 1 BSV = 100,000,000 satoshis.
 */
export function satsToUsd(sats: number, rate: number): number {
  return (sats / 100_000_000) * rate;
}

/**
 * Convert USD to satoshis. Rounds to nearest integer sat.
 */
export function usdToSats(usd: number, rate: number): number {
  if (!rate || rate <= 0) return 0;
  return Math.round((usd / rate) * 100_000_000);
}

/**
 * Format USD value for display.
 */
export function formatUsd(sats: number, rate: number): string {
  if (!rate || rate <= 0) return '';
  const usd = satsToUsd(sats, rate);
  if (usd === 0) return '$0.00';
  if (usd < 0.01) return '< $0.01';
  if (usd < 100) return `$${usd.toFixed(2)}`;
  return `$${usd.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

/**
 * Dual-currency formatting: primary and secondary strings based on display mode.
 * Falls back to sats-only when rate is unavailable.
 */
export function formatDualCurrency(
  sats: number,
  rate: number,
  mode: CurrencyDisplay = 'usd-first'
): { primary: string; secondary: string } {
  const satsStr = `${formatSats(sats)} sats`;
  if (!rate || rate <= 0) return { primary: satsStr, secondary: '' };
  const usd = formatUsd(sats, rate);
  return mode === 'usd-first'
    ? { primary: usd, secondary: satsStr }
    : { primary: satsStr, secondary: usd };
}

/**
 * Inline currency for compact contexts (events, table cells).
 * Returns the preferred format string; caller should set title attr to the alternate.
 */
export function formatInlineCurrency(
  sats: number,
  rate: number,
  mode: CurrencyDisplay = 'usd-first'
): string {
  if (mode === 'usd-first' && rate > 0) return formatUsd(sats, rate);
  return `${formatSats(sats)} sats`;
}

/**
 * The alternate (tooltip) text for inline currency display.
 */
export function formatInlineCurrencyAlt(
  sats: number,
  rate: number,
  mode: CurrencyDisplay = 'usd-first'
): string {
  if (mode === 'usd-first' || !rate || rate <= 0) return `${formatSats(sats)} sats`;
  return formatUsd(sats, rate);
}
