/**
 * Compact progress + budget strip for embedding in chat view.
 * Shows iteration step, task spending vs limit, hourly/daily spent, and wallet balance.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import type { BudgetReport } from '../lib/shared-types.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import { formatInlineCurrency } from '../lib/usd.js';
import './tooltip.js';

function gaugeColor(pct: number): string {
  if (pct >= 95) return 'var(--error, #fd5454)';
  if (pct >= 80) return '#f97316';
  if (pct >= 60) return 'var(--warning, #fcbe2d)';
  return 'var(--accent, #14A8C4)';
}

function formatCompactSats(sats: number): string {
  if (sats >= 1_000_000) return `${(sats / 1_000_000).toFixed(1)}M`;
  if (sats >= 1_000) return `${(sats / 1_000).toFixed(1)}K`;
  return sats.toLocaleString();
}

@customElement('dm-budget-strip')
export class WormBudgetStrip extends LitElement {
  @property({ type: Object }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ type: Boolean }) active = false;
  @property({ type: Number }) iteration = 0;
  @property({ type: Number }) taskSats = 0;
  @property() currencyMode: CurrencyDisplay = 'usd-first';
  @property({ type: Number }) usdRate = 0;

  @state() private data: BudgetReport | null = null;

  private _timer: ReturnType<typeof setInterval> | null = null;

  static styles = css`
    :host { display: block; }

    .strip {
      display: flex;
      align-items: center;
      height: 30px;
      padding: 0 16px;
      gap: 14px;
      background: var(--bg-surface, #0F2337);
      border-bottom: 1px solid var(--border, #1A3550);
      font-family: var(--mono, monospace);
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      cursor: pointer;
      transition: background 0.15s;
    }
    .strip:hover {
      background: var(--bg-elevated, #142D42);
    }

    .progress-section {
      display: flex;
      align-items: center;
      gap: 6px;
      flex-shrink: 0;
    }
    .step-label {
      white-space: nowrap;
      color: var(--text, rgba(255,255,255,0.8));
    }
    .progress-track {
      width: 60px;
      height: 3px;
      background: var(--bg, #0B1929);
      border-radius: 2px;
      overflow: hidden;
    }
    .progress-fill {
      height: 100%;
      border-radius: 2px;
      background: var(--accent, #14A8C4);
      transition: width 0.4s ease;
    }

    .sep {
      width: 1px;
      height: 14px;
      background: var(--border, #1A3550);
      flex-shrink: 0;
    }

    .metric {
      display: flex;
      align-items: baseline;
      gap: 3px;
      white-space: nowrap;
    }
    .metric-label {
      color: var(--text-dim, rgba(255,255,255,0.35));
      font-size: 9px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }
    .metric-value {
      color: var(--text, rgba(255,255,255,0.8));
    }
    .metric-value.warn {
      color: var(--warning, #fcbe2d);
    }
    .metric-slash {
      color: var(--text-dim, rgba(255,255,255,0.25));
      font-size: 10px;
    }

    .spacer { flex: 1; }

    .balance {
      display: flex;
      align-items: baseline;
      gap: 3px;
      white-space: nowrap;
      font-size: 10px;
    }
    .balance .metric-label {
      color: var(--text-dim, rgba(255,255,255,0.35));
    }
    .balance .metric-value {
      color: var(--success, #00b69b);
    }

    @media (max-width: 640px) {
      .strip { padding: 0 8px; gap: 8px; font-size: 10px; }
      .progress-track { width: 40px; }
      .hide-mobile { display: none; }
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    this._loadData();
    this._syncTimer();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this._clearTimer();
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('active')) {
      this._syncTimer();
    }
  }

  private _syncTimer() {
    this._clearTimer();
    if (this.active) {
      this._timer = setInterval(() => this._loadData(), 5000);
    }
  }

  private _clearTimer() {
    if (this._timer !== null) {
      clearInterval(this._timer);
      this._timer = null;
    }
  }

  private async _loadData() {
    try {
      const resp = await this.fetchFn('/budget');
      if (resp.ok) {
        this.data = await resp.json();
      }
    } catch {
      // keep last data visible
    }
  }

  private _navigate() {
    window.location.hash = '#budget';
  }

  /** Format sats using currency mode — USD when available in usd-first, compact sats otherwise. */
  private _fmt(sats: number): string {
    if (this.usdRate > 0 && this.currencyMode === 'usd-first') {
      return formatInlineCurrency(sats, this.usdRate, this.currencyMode);
    }
    return formatCompactSats(sats);
  }

  render() {
    // Show strip when active (task running) or when we have data from a recent task
    const showProgress = this.active || this.iteration > 0;
    const hasData = this.data !== null;
    if (!showProgress && !hasData) return nothing;

    const b = this.data;
    // Progress bar reflects per-task budget consumption (sats spent / per-task limit)
    const taskLimit = b?.limits?.max_per_task ?? 0;
    const taskPct = taskLimit > 0 ? Math.min((this.taskSats / taskLimit) * 100, 100) : 0;
    const taskColor = gaugeColor(taskPct);

    // Include current task spending in hourly/daily totals (global tracker
    // only includes completed tasks — current task's entries merge on finish)
    const hourlySpent = (b?.hourly_sats ?? 0) + (this.active ? this.taskSats : 0);
    const dailySpent = (b?.daily_sats ?? 0) + (this.active ? this.taskSats : 0);
    const hourlyLimit = b?.limits?.max_per_hour ?? 0;
    const dailyLimit = b?.limits?.max_per_day ?? 0;

    return html`
      <div class="strip" @click=${this._navigate} title="Click for detailed budget view">
        ${showProgress ? html`
          <div class="progress-section">
            <span class="step-label">Step ${this.iteration}</span>
            <div class="progress-track">
              <div class="progress-fill" style="width:${taskPct}%;background:${taskColor}"></div>
            </div>
          </div>
          ${this.taskSats > 0 ? html`
            <div class="sep"></div>
            <div class="metric">
              <span class="metric-label">task</span>
              <span class="metric-value warn">${this._fmt(this.taskSats)}</span>
              ${taskLimit > 0 ? html`<span class="metric-slash">/</span><span class="metric-value">${this._fmt(taskLimit)}</span>` : ''}
            </div>
          ` : ''}
        ` : ''}
        ${b ? html`
          ${showProgress ? html`<div class="sep"></div>` : ''}
          <dm-tooltip text="Total spent in the last rolling hour vs hourly limit" class="hide-mobile">
            <div class="metric">
              <span class="metric-label">hour</span>
              <span class="metric-value">${this._fmt(hourlySpent)}</span>
              ${hourlyLimit > 0 ? html`<span class="metric-slash">/</span><span class="metric-value">${this._fmt(hourlyLimit)}</span>` : ''}
            </div>
          </dm-tooltip>
          <dm-tooltip text="Total spent in the last rolling 24 hours vs daily limit" class="hide-mobile">
            <div class="metric">
              <span class="metric-label">day</span>
              <span class="metric-value">${this._fmt(dailySpent)}</span>
              ${dailyLimit > 0 ? html`<span class="metric-slash">/</span><span class="metric-value">${this._fmt(dailyLimit)}</span>` : ''}
            </div>
          </dm-tooltip>
          <div class="spacer"></div>
          <div class="balance">
            <span class="metric-label">wallet</span>
            <span class="metric-value">${this._fmt(b.balance)}</span>
          </div>
        ` : ''}
      </div>
    `;
  }
}
