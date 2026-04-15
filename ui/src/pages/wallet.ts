/**
 * Wallet page — "fuel gauge" for the AI agent.
 * Shows spendable balance, UTXO tier distribution, funding flow,
 * and individual UTXO breakdown. Every thought costs satoshis.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import {
  pageHost,
  summaryGrid,
  buttonStyles,
  stateFeedback,
  skeleton,
  renderError,
} from '../lib/shared-styles.js';
import { formatSats } from '../lib/util.js';
import {
  fetchBsvUsdRate,
  formatDualCurrency,
  formatInlineCurrencyAlt,
  formatUsd,
} from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';

interface TierInfo {
  count: number;
  total_sats: number;
}

interface UtxoEntry {
  outpoint: string;
  satoshis: number;
  spendable: boolean;
}

interface WalletUtxosResponse {
  balance: number;
  utxo_count: number;
  utxos: UtxoEntry[];
  tiers: {
    dust: TierInfo;
    small: TierInfo;
    medium: TierInfo;
    large: TierInfo;
  };
}

interface WalletData {
  utxos: WalletUtxosResponse;
  address: string;
  usdRate: number;
}

const TIER_CONFIG = {
  dust: { label: 'Dust', color: 'var(--error, #fd5454)', threshold: '< 1K sats' },
  small: { label: 'Small', color: 'var(--warning, #fcbe2d)', threshold: '1K - 100K' },
  medium: { label: 'Medium', color: 'var(--accent, #14A8C4)', threshold: '100K - 1M' },
  large: { label: 'Large', color: 'var(--success, #00b69b)', threshold: '> 1M sats' },
} as const;

type TierKey = keyof typeof TIER_CONFIG;

const TIER_ORDER: TierKey[] = ['dust', 'small', 'medium', 'large'];

/** Average sats per LLM call for estimates. */
const AVG_SATS_PER_CALL = 300;

/** Number of UTXOs shown before "Show all" expansion. */
const UTXO_PAGE_SIZE = 20;

function tierForSats(sats: number): TierKey {
  if (sats < 1_000) return 'dust';
  if (sats < 100_000) return 'small';
  if (sats < 1_000_000) return 'medium';
  return 'large';
}

function truncateOutpoint(outpoint: string): string {
  if (outpoint.length <= 16) return outpoint;
  return `${outpoint.slice(0, 8)}...${outpoint.slice(-4)}`;
}

function formatCompactSats(sats: number): string {
  if (sats >= 1_000_000) {
    const m = sats / 1_000_000;
    return m >= 10 ? `${Math.round(m)}M` : `${m.toFixed(1).replace(/\.0$/, '')}M`;
  }
  if (sats >= 1_000) {
    const k = sats / 1_000;
    return k >= 10 ? `${Math.round(k)}K` : `${k.toFixed(1).replace(/\.0$/, '')}K`;
  }
  return sats.toLocaleString();
}

@customElement('dm-wallet')
export class DmWallet extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  @state() private loading = true;
  @state() private error = '';
  @state() private data: WalletData | null = null;
  @state() private copied = false;
  @state() private checking = false;
  @state() private splitting = false;
  @state() private splitMessage = '';
  @state() private showAllUtxos = false;
  @state() private barAnimated = false;

  static styles = [
    pageHost,
    summaryGrid,
    buttonStyles,
    stateFeedback,
    skeleton,
    css`
      /* Page header */
      .page-header {
        margin-bottom: 24px;
      }

      .page-title {
        font-size: 24px;
        font-weight: 700;
        color: var(--text-bright, #fff);
        font-family: var(--sans, sans-serif);
        margin: 0;
      }

      .page-subtitle {
        font-size: 13px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-top: 4px;
      }

      /* Summary cards — override summary-value for balance */
      .summary-value.sats {
        color: var(--warning, #fcbe2d);
      }

      .summary-secondary {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-family: var(--mono, monospace);
        margin-top: 2px;
      }

      /* Tier distribution section */
      .tier-section {
        margin-bottom: 24px;
      }

      .section-label {
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-bottom: 10px;
      }

      .tier-bar-container {
        width: 100%;
        height: 12px;
        border-radius: 6px;
        background: var(--bg-elevated, #142D42);
        overflow: hidden;
        display: flex;
      }

      .tier-bar-segment {
        height: 100%;
        transition: width 0.8s cubic-bezier(0.4, 0, 0.2, 1);
      }

      .tier-bar-segment:first-child {
        border-radius: 6px 0 0 6px;
      }

      .tier-bar-segment:last-child {
        border-radius: 0 6px 6px 0;
      }

      .tier-bar-segment:only-child {
        border-radius: 6px;
      }

      @media (prefers-reduced-motion: reduce) {
        .tier-bar-segment { transition: none; }
      }

      .tier-legend {
        display: flex;
        flex-wrap: wrap;
        gap: 16px;
        margin-top: 10px;
      }

      .tier-legend-item {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 12px;
        color: var(--text, rgba(255,255,255,0.8));
      }

      .tier-dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        flex-shrink: 0;
      }

      .tier-legend-count {
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .tier-legend-sats {
        font-family: var(--mono, monospace);
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      /* Funding card */
      .funding-card {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 20px 24px;
        margin-bottom: 24px;
        transition: border-color 0.3s ease, box-shadow 0.3s ease;
      }

      .funding-card.urgent {
        border-color: rgba(252, 190, 45, 0.4);
        box-shadow: 0 0 20px rgba(252, 190, 45, 0.06), inset 0 0 20px rgba(252, 190, 45, 0.02);
      }

      .funding-title {
        font-size: 16px;
        font-weight: 700;
        color: var(--text-bright, #fff);
        font-family: var(--sans, sans-serif);
        margin-bottom: 16px;
      }

      .funding-steps {
        display: flex;
        flex-direction: column;
        gap: 12px;
        margin-bottom: 16px;
      }

      .funding-step {
        display: flex;
        align-items: flex-start;
        gap: 12px;
        font-size: 13px;
        color: var(--text, rgba(255,255,255,0.8));
        line-height: 1.5;
      }

      .step-num {
        width: 22px;
        height: 22px;
        border-radius: 50%;
        background: var(--accent, #14A8C4);
        color: #fff;
        font-size: 12px;
        font-weight: 700;
        display: flex;
        align-items: center;
        justify-content: center;
        flex-shrink: 0;
        margin-top: 1px;
      }

      .address-card {
        background: var(--bg, #0B1929);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-sm, 8px);
        padding: 14px 16px;
        margin-bottom: 16px;
      }

      .address-label {
        font-size: 11px;
        font-weight: 600;
        color: var(--text-dim, rgba(255,255,255,0.5));
        text-transform: uppercase;
        letter-spacing: 0.5px;
        margin-bottom: 8px;
      }

      .address-row {
        display: flex;
        align-items: center;
        gap: 8px;
      }

      .address-value {
        flex: 1;
        font-family: var(--mono, monospace);
        font-size: 14px;
        color: var(--accent, #14A8C4);
        word-break: break-all;
        line-height: 1.4;
      }

      .copy-btn {
        padding: 6px 14px;
        background: var(--accent, #14A8C4);
        border: none;
        border-radius: 6px;
        color: #fff;
        font-size: 12px;
        font-weight: 600;
        font-family: inherit;
        cursor: pointer;
        flex-shrink: 0;
        transition: opacity 0.15s ease, background 0.15s ease;
      }

      .copy-btn:hover { opacity: 0.85; }

      .copy-btn.copied {
        background: var(--success, #00b69b);
      }

      .funding-actions {
        display: flex;
        gap: 8px;
        margin-bottom: 12px;
      }

      .funding-hint {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        line-height: 1.5;
        padding-top: 8px;
        border-top: 1px solid var(--border, #1A3550);
      }

      /* UTXO table */
      .utxo-section {
        margin-bottom: 24px;
      }

      .utxo-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 12px;
      }

      .utxo-table-wrap {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        overflow: hidden;
      }

      .utxo-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 13px;
      }

      .utxo-table thead th {
        text-align: left;
        font-size: 11px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        padding: 10px 16px;
        border-bottom: 1px solid var(--border, #1A3550);
        background: rgba(0, 0, 0, 0.15);
      }

      .utxo-table tbody tr {
        transition: background 0.12s ease;
      }

      .utxo-table tbody tr:hover {
        background: rgba(255, 255, 255, 0.02);
      }

      .utxo-table td {
        padding: 10px 16px;
        border-bottom: 1px solid rgba(26, 53, 80, 0.5);
        color: var(--text, rgba(255,255,255,0.8));
        vertical-align: middle;
      }

      .utxo-table tbody tr:last-child td {
        border-bottom: none;
      }

      .utxo-tier-cell {
        display: flex;
        align-items: center;
        gap: 8px;
      }

      .utxo-tier-dot {
        width: 8px;
        height: 8px;
        border-radius: 50%;
        flex-shrink: 0;
      }

      .utxo-tier-label {
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .utxo-amount {
        font-family: var(--mono, monospace);
        font-weight: 600;
        color: var(--text-bright, #fff);
      }

      .utxo-usd {
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .utxo-outpoint {
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .utxo-row-border {
        border-left: 3px solid transparent;
      }

      .show-all-row {
        text-align: center;
        padding: 12px;
      }

      .show-all-btn {
        background: none;
        border: 1px solid var(--border, #1A3550);
        color: var(--accent, #14A8C4);
        font-size: 12px;
        font-weight: 600;
        font-family: inherit;
        padding: 6px 20px;
        border-radius: 6px;
        cursor: pointer;
        transition: background 0.15s ease, border-color 0.15s ease;
      }

      .show-all-btn:hover {
        background: rgba(20, 168, 196, 0.08);
        border-color: var(--accent, #14A8C4);
      }

      .empty-utxos {
        text-align: center;
        padding: 32px 16px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-size: 13px;
      }

      .split-status {
        margin-top: 12px;
        padding: 10px 14px;
        background: rgba(20, 168, 196, 0.08);
        border: 1px solid rgba(20, 168, 196, 0.24);
        border-radius: 6px;
        font-size: 12px;
        color: var(--text, #E8F0FB);
        font-family: var(--font-mono, ui-monospace, monospace);
      }

      /* Responsive */
      @media (max-width: 639px) {
        .page-title { font-size: 20px; }
        .tier-legend { gap: 10px; }
        .funding-card { padding: 16px; }
        .funding-actions { flex-direction: column; }
        .address-value { font-size: 12px; }

        .utxo-table-wrap { overflow-x: auto; }
        .utxo-table { min-width: 500px; }

        .tier-legend-item { font-size: 11px; }
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.loadData();
  }

  private async loadData() {
    this.loading = true;
    this.error = '';

    try {
      const [utxoRes, addrRes, usdRate] = await Promise.all([
        this.fetchFn('/wallet/utxos'),
        fetch('/wallet/address').catch(() => null),
        fetchBsvUsdRate(),
      ]);

      if (!utxoRes.ok) throw new Error(`Failed to load UTXOs: ${utxoRes.status}`);

      const utxos: WalletUtxosResponse = await utxoRes.json();

      let address = '';
      if (addrRes && addrRes.ok) {
        const data = await addrRes.json();
        address = data.address ?? '';
      }

      this.data = { utxos, address, usdRate };

      // Trigger bar animation after data loads
      requestAnimationFrame(() => {
        this.barAnimated = true;
      });
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      this.loading = false;
    }
  }

  private async copyAddress() {
    if (!this.data?.address) return;
    try {
      await navigator.clipboard.writeText(this.data.address);
      this.copied = true;
      setTimeout(() => { this.copied = false; }, 2000);
    } catch { /* fallback: do nothing */ }
  }

  private async checkForFunding() {
    this.checking = true;
    try {
      const res = await fetch('/wallet/check-funding', { method: 'POST' });
      if (res.ok) {
        // Re-fetch everything to reflect changes
        await this.loadData();
      }
    } catch { /* best effort */ }
    this.checking = false;
  }

  private async splitBalance() {
    const raw = window.prompt(
      'Split the wallet balance into how many equal UTXOs?\n\n' +
      'Useful for parallel-agent funding — each output becomes an independently\n' +
      'spendable chunk. Must be ≥ 2.',
      '5'
    );
    if (raw === null) return;
    const count = Number.parseInt(raw, 10);
    if (!Number.isFinite(count) || count < 2) {
      this.splitMessage = 'Split count must be an integer ≥ 2.';
      setTimeout(() => { this.splitMessage = ''; }, 4000);
      return;
    }

    this.splitting = true;
    this.splitMessage = '';
    try {
      const res = await fetch('/wallet/split', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ count }),
      });
      if (res.ok) {
        const body = await res.json() as {
          txid: string;
          per_output_sats: number;
          count: number;
          view_url: string;
        };
        this.splitMessage =
          `✓ Split into ${body.count} UTXOs (${body.per_output_sats.toLocaleString()} sats each). ` +
          `TxID: ${body.txid.slice(0, 16)}…`;
        await this.loadData();
      } else {
        const err = await res.text();
        this.splitMessage = `Split failed: ${err || res.statusText}`;
      }
    } catch (e) {
      this.splitMessage = `Split failed: ${e instanceof Error ? e.message : 'network error'}`;
    }
    this.splitting = false;
    // Leave success message visible for a while so the user can copy the txid
    setTimeout(() => { this.splitMessage = ''; }, 15000);
  }

  private async refresh() {
    await this.loadData();
  }

  render() {
    if (this.loading && !this.data) {
      return html`
        <div class="page-header">
          <div class="page-title">Wallet</div>
          <div class="page-subtitle">Agent fuel — every thought costs satoshis</div>
        </div>
        <div class="skeleton-hero-grid" style="grid-template-columns: repeat(3, 1fr)">
          ${[0, 1, 2].map(() => html`<div class="skeleton skeleton-card" style="height:90px"></div>`)}
        </div>
        <div class="skeleton skeleton-card" style="height:12px;margin-bottom:24px;border-radius:6px"></div>
        <div class="skeleton skeleton-card" style="height:200px;margin-bottom:24px"></div>
        <div class="skeleton skeleton-card" style="height:300px"></div>
      `;
    }

    if (this.error && !this.data) {
      return renderError(this.error, 'Check that the agent server is running and the wallet is connected.');
    }

    if (!this.data) return nothing;

    const { utxos, address, usdRate } = this.data;

    return html`
      <div class="page-header">
        <div class="page-title">Wallet</div>
        <div class="page-subtitle">Agent fuel — every thought costs satoshis</div>
      </div>
      ${this.renderSummaryCards(utxos, usdRate)}
      ${this.renderTierBar(utxos, usdRate)}
      ${this.renderFundingCard(utxos.balance, address)}
      ${this.renderUtxoTable(utxos, usdRate)}
    `;
  }

  private renderSummaryCards(utxos: WalletUtxosResponse, usdRate: number) {
    const { primary, secondary } = formatDualCurrency(utxos.balance, usdRate, this.currencyMode);
    const estimatedCalls = Math.floor(utxos.balance / AVG_SATS_PER_CALL);

    return html`
      <div class="summary-grid" style="grid-template-columns: repeat(3, 1fr)">
        <div class="summary-card">
          <div class="summary-value sats"
               title="${formatInlineCurrencyAlt(utxos.balance, usdRate, this.currencyMode)}">
            ${this.currencyMode === 'sats-first'
              ? `${formatSats(utxos.balance)} sats`
              : primary}
          </div>
          ${secondary ? html`<div class="summary-secondary">${this.currencyMode === 'sats-first' ? secondary : `${formatSats(utxos.balance)} sats`}</div>` : nothing}
          <div class="summary-label">Balance</div>
        </div>

        <div class="summary-card">
          <div class="summary-value accent">${utxos.utxo_count.toLocaleString()}</div>
          <div class="summary-label">Spendable UTXOs</div>
        </div>

        <div class="summary-card">
          <div class="summary-value success">~${estimatedCalls.toLocaleString()}</div>
          <div class="summary-label">LLM calls remaining</div>
        </div>
      </div>
    `;
  }

  private renderTierBar(utxos: WalletUtxosResponse, usdRate: number) {
    const { tiers } = utxos;
    const total = utxos.balance || 1; // avoid division by zero

    // Only show tiers that have sats
    const activeTiers = TIER_ORDER.filter(key => tiers[key].total_sats > 0);

    if (activeTiers.length === 0 && utxos.balance === 0) {
      return html`
        <div class="tier-section">
          <div class="section-label">UTXO Distribution</div>
          <div class="tier-bar-container" style="background: var(--bg-elevated, #142D42)"></div>
          <div class="tier-legend">
            ${TIER_ORDER.map(key => html`
              <div class="tier-legend-item">
                <span class="tier-dot" style="background: ${TIER_CONFIG[key].color}"></span>
                <span>${TIER_CONFIG[key].label}</span>
                <span class="tier-legend-count">(0)</span>
              </div>
            `)}
          </div>
        </div>
      `;
    }

    return html`
      <div class="tier-section">
        <div class="section-label">UTXO Distribution</div>
        <div class="tier-bar-container">
          ${TIER_ORDER.map(key => {
            const tier = tiers[key];
            if (tier.total_sats === 0) return nothing;
            const pct = (tier.total_sats / total) * 100;
            const width = this.barAnimated ? `${Math.max(pct, 1)}%` : '0%';
            return html`
              <div class="tier-bar-segment"
                   style="width: ${width}; background: ${TIER_CONFIG[key].color}"
                   title="${TIER_CONFIG[key].label}: ${formatSats(tier.total_sats)} sats (${tier.count} UTXOs)">
              </div>
            `;
          })}
        </div>
        <div class="tier-legend">
          ${TIER_ORDER.map(key => {
            const tier = tiers[key];
            return html`
              <div class="tier-legend-item">
                <span class="tier-dot" style="background: ${TIER_CONFIG[key].color}"></span>
                <span>${TIER_CONFIG[key].label}</span>
                <span class="tier-legend-count">(${tier.count})</span>
                ${tier.total_sats > 0 ? html`
                  <span class="tier-legend-sats"
                        title="${formatInlineCurrencyAlt(tier.total_sats, usdRate, this.currencyMode)}">
                    ${formatCompactSats(tier.total_sats)} sats
                  </span>
                ` : nothing}
              </div>
            `;
          })}
        </div>
      </div>
    `;
  }

  private renderFundingCard(balance: number, address: string) {
    const isEmpty = balance === 0;

    return html`
      <div class="funding-card ${isEmpty ? 'urgent' : ''}">
        <div class="funding-title">Add Funds</div>

        ${address ? html`
          <div class="address-card">
            <div class="address-label">Send BSV to this address</div>
            <div class="address-row">
              <code class="address-value">${address}</code>
              <button class="copy-btn ${this.copied ? 'copied' : ''}"
                      @click=${() => this.copyAddress()}>
                ${this.copied ? 'Copied!' : 'Copy'}
              </button>
            </div>
          </div>
        ` : nothing}

        <div class="funding-steps">
          <div class="funding-step">
            <span class="step-num">1</span>
            <span>Send BSV to the address above from any wallet or exchange</span>
          </div>
          <div class="funding-step">
            <span class="step-num">2</span>
            <span>Click <strong>Check for payment</strong> below (transactions confirm in seconds)</span>
          </div>
        </div>

        <div class="funding-actions">
          <button class="btn btn-primary" @click=${() => this.checkForFunding()} ?disabled=${this.checking || this.splitting}>
            ${this.checking ? 'Checking...' : 'Check for payment'}
          </button>
          <button class="btn" @click=${() => this.splitBalance()} ?disabled=${this.checking || this.splitting}>
            ${this.splitting ? 'Splitting...' : 'Split UTXOs'}
          </button>
          <button class="btn" @click=${() => this.refresh()}>
            Refresh
          </button>
        </div>

        ${this.splitMessage ? html`
          <div class="split-status">${this.splitMessage}</div>
        ` : nothing}

        <div class="funding-hint">
          ~300 sats per LLM call. $1 gets you ~7,000 calls.
          Split UTXOs when you need parallel-agent funding.
        </div>
      </div>
    `;
  }

  private renderUtxoTable(utxos: WalletUtxosResponse, usdRate: number) {
    if (utxos.utxos.length === 0) {
      return html`
        <div class="utxo-section">
          <div class="utxo-header">
            <div class="section-label">UTXOs</div>
          </div>
          <div class="utxo-table-wrap">
            <div class="empty-utxos">No spendable UTXOs. Fund the wallet to get started.</div>
          </div>
        </div>
      `;
    }

    // Sort by satoshis descending
    const sorted = [...utxos.utxos].sort((a, b) => b.satoshis - a.satoshis);
    const visible = this.showAllUtxos ? sorted : sorted.slice(0, UTXO_PAGE_SIZE);
    const hasMore = sorted.length > UTXO_PAGE_SIZE && !this.showAllUtxos;

    return html`
      <div class="utxo-section">
        <div class="utxo-header">
          <div class="section-label">UTXOs (${utxos.utxo_count})</div>
        </div>
        <div class="utxo-table-wrap">
          <table class="utxo-table">
            <thead>
              <tr>
                <th>Tier</th>
                <th>Amount</th>
                <th>USD</th>
                <th>Outpoint</th>
              </tr>
            </thead>
            <tbody>
              ${visible.map(u => {
                const tier = tierForSats(u.satoshis);
                const cfg = TIER_CONFIG[tier];
                return html`
                  <tr class="utxo-row-border" style="border-left-color: ${cfg.color}">
                    <td>
                      <div class="utxo-tier-cell">
                        <span class="utxo-tier-dot" style="background: ${cfg.color}"></span>
                        <span class="utxo-tier-label">${cfg.label}</span>
                      </div>
                    </td>
                    <td>
                      <span class="utxo-amount">${formatSats(u.satoshis)}</span>
                    </td>
                    <td>
                      <span class="utxo-usd">${usdRate > 0 ? formatUsd(u.satoshis, usdRate) : '--'}</span>
                    </td>
                    <td>
                      <span class="utxo-outpoint" title="${u.outpoint}">${truncateOutpoint(u.outpoint)}</span>
                    </td>
                  </tr>
                `;
              })}
            </tbody>
          </table>
          ${hasMore ? html`
            <div class="show-all-row">
              <button class="show-all-btn" @click=${() => { this.showAllUtxos = true; }}>
                Show all ${sorted.length} UTXOs
              </button>
            </div>
          ` : nothing}
        </div>
      </div>
    `;
  }
}
