/**
 * localStorage helpers for persisting user preferences and session state.
 */

const MODEL_KEY = 'dm-model';
const SESSION_KEY = 'dm-session-id';
const CURRENCY_KEY = 'dm-currency';
const DEFAULT_MODEL = 'gpt-5-mini';

export type CurrencyDisplay = 'usd-first' | 'sats-first';

/** Read the persisted currency display preference. Default: usd-first. */
export function getCurrencyDisplay(): CurrencyDisplay {
  return (localStorage.getItem(CURRENCY_KEY) as CurrencyDisplay) ?? 'usd-first';
}

/** Persist the currency display preference. */
export function setCurrencyDisplay(mode: CurrencyDisplay): void {
  localStorage.setItem(CURRENCY_KEY, mode);
}

/** Read the persisted model, falling back to the default. */
export function getModel(): string {
  return localStorage.getItem(MODEL_KEY) || DEFAULT_MODEL;
}

/** Persist the selected model. */
export function setModel(model: string): void {
  localStorage.setItem(MODEL_KEY, model);
}

/** Read the persisted session ID, or null if none. */
export function getSessionId(): string | null {
  return localStorage.getItem(SESSION_KEY);
}

/** Persist the active session ID. */
export function setSessionId(id: string): void {
  localStorage.setItem(SESSION_KEY, id);
}

/** Remove the persisted session ID. */
export function clearSessionId(): void {
  localStorage.removeItem(SESSION_KEY);
}

// ---- Onboarding ----
const ONBOARDING_KEY = 'dm-onboarding-seen';

/** Check if onboarding has been shown. */
export function hasSeenOnboarding(): boolean {
  return localStorage.getItem(ONBOARDING_KEY) === 'true';
}

/** Mark onboarding as seen. */
export function markOnboardingSeen(): void {
  localStorage.setItem(ONBOARDING_KEY, 'true');
}

// ---- Date Range ----
const DATE_RANGE_KEY = 'dm-date-range';

export interface StoredDateRange {
  preset: string;
  from?: string;
  to?: string;
}

/** Read the persisted date range preference. Default: 30d. */
export function getDateRange(): StoredDateRange {
  const raw = localStorage.getItem(DATE_RANGE_KEY);
  if (raw) { try { return JSON.parse(raw); } catch { /* ignore */ } }
  return { preset: '30d' };
}

/** Persist the date range preference. */
export function setDateRange(range: StoredDateRange): void {
  localStorage.setItem(DATE_RANGE_KEY, JSON.stringify(range));
}
