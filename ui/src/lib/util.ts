/**
 * Format a Unix timestamp as a relative time string.
 * Examples: "just now", "2m ago", "1h ago", "3d ago"
 */
export function formatRelativeTime(unixTimestamp: number): string {
  const now = Date.now() / 1000;
  const diff = now - unixTimestamp;
  if (diff < 0) return 'just now';
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 604800) return `${Math.floor(diff / 86400)}d ago`;
  return new Date(unixTimestamp * 1000).toLocaleDateString();
}

/**
 * Format a duration in seconds as a human-readable string.
 * Examples: "< 1s", "12s", "2m 30s", "1h 15m"
 */
export function formatDuration(secs: number): string {
  if (secs < 1) return '< 1s';
  if (secs < 60) return `${Math.round(secs)}s`;
  const m = Math.floor(secs / 60);
  const s = Math.round(secs % 60);
  if (m < 60) return s > 0 ? `${m}m ${s}s` : `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  return rm > 0 ? `${h}h ${rm}m` : `${h}h`;
}

/**
 * Format an uptime in seconds as a human-readable string.
 * Examples: "2h 15m", "3d 4h", "12m"
 */
export function formatUptime(secs: number): string {
  if (secs < 60) return `${Math.round(secs)}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  const rm = m % 60;
  if (h < 24) return rm > 0 ? `${h}h ${rm}m` : `${h}h`;
  const d = Math.floor(h / 24);
  const rh = h % 24;
  return rh > 0 ? `${d}d ${rh}h` : `${d}d`;
}

/**
 * Truncate a string to maxLen and append ellipsis if truncated.
 */
export function truncate(s: string, maxLen: number): string {
  if (s.length <= maxLen) return s;
  return s.slice(0, maxLen) + '...';
}

/**
 * Parse an ISO 8601 timestamp string to a Unix timestamp (seconds).
 */
export function parseTimestamp(ts: string): number {
  return new Date(ts).getTime() / 1000;
}

/**
 * Truncate a hex key for display: "02af…3b1e".
 */
export function truncateKey(key: string): string {
  if (key.length <= 20) return key;
  return `${key.slice(0, 10)}...${key.slice(-8)}`;
}

/**
 * Format satoshis with locale grouping: 1234567 → "1,234,567"
 */
export function formatSats(sats: number): string {
  return sats.toLocaleString();
}

/**
 * Format a Date for chart x-axis labels.
 * Returns "Mon D" format (e.g., "Jan 5", "Dec 31").
 * If showYear is true, appends 2-digit year: "Jan 5 '25".
 */
export function formatChartDate(date: Date, showYear = false): string {
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const label = `${months[date.getMonth()]} ${date.getDate()}`;
  if (showYear) {
    const yr = String(date.getFullYear()).slice(-2);
    return `${label} '${yr}`;
  }
  return label;
}

/**
 * Format token counts with compact notation for large numbers.
 * Examples: 0 → "0", 450 → "450", 45230 → "45,230", 1234567 → "1.2M"
 */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return m >= 10 ? `${Math.round(m)}M` : `${m.toFixed(1).replace(/\.0$/, '')}M`;
  }
  return n.toLocaleString();
}

