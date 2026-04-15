/**
 * Budget visualization panel — gauge bars, per-service spending, alert banners.
 * Supports compact mode for dashboard embedding.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { pageHost, centerState, pageTitle, sectionTitle, gaugeBar, alertBanner, stateFeedback, renderLoading, renderError, renderEmpty } from '../../lib/shared-styles.js';
import type { BudgetGaugeData, BudgetReport, TaskSummary } from '../../lib/shared-types.js';
import { formatSats, truncate, parseTimestamp } from '../../lib/util.js';
import { formatInlineCurrency, formatInlineCurrencyAlt, fetchBsvUsdRate } from '../../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../../lib/storage.js';
import { SERVICE_COLORS, friendlyName } from '../../lib/constants.js';
import { FetchController } from '../../controllers/fetch-controller.js';
import { BUDGET_HELP } from '../../lib/labels.js';
import './spending-export.js';

// BudgetReport type is now imported from shared-types.ts
// (previously had a local BudgetReport that duplicated the backend shape)

function gaugeColor(pct: number): string {
  if (pct >= 95) return 'var(--error, #fd5454)';
  if (pct >= 80) return '#f97316';
  if (pct >= 60) return 'var(--warning, #fcbe2d)';
  return 'var(--accent, #14A8C4)';
}

function satsToUsd(sats: number, rate: number): string {
  if (!rate || rate <= 0) return '';
  const bsv = sats / 100_000_000;
  const usd = bsv * rate;
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

@customElement('dm-budget-panel')
export class WormBudgetPanel extends LitElement {
  @property({ type: Object }) fetchFn: typeof fetch = fetch;
  @property({ type: Boolean }) compact = false;
  @property({ type: Number }) usdRate = 0;
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  @state() private activeTask: TaskSummary | null = null;

  private ctrl = new FetchController<BudgetReport>(this);
  private statusTimer: number | null = null;

  static styles = [
    pageHost, centerState, pageTitle, sectionTitle, gaugeBar, alertBanner, stateFeedback,
    css`
      .budget-section { margin-bottom: 24px; }

      .service-bar { margin-bottom: 10px; }
      .service-header {
        display: flex;
        justify-content: space-between;
        align-items: baseline;
        margin-bottom: 3px;
      }
      .service-label {
        font-size: 12px;
        color: var(--text, #e0e0e8);
      }
      .service-value {
        font-family: var(--mono);
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }
      .service-track {
        height: 6px;
        background: var(--bg, #0B1929);
        border-radius: 3px;
        overflow: hidden;
        border: 1px solid var(--border, #1A3550);
      }
      .service-fill {
        height: 100%;
        border-radius: 3px;
        transition: width 0.6s ease;
      }

      .no-services {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-style: italic;
      }

      .gauge-fill.pulse {
        /* no animation — solid fill at limit */
      }

      .task-context {
        display: flex;
        align-items: center;
        gap: 6px;
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-bottom: 4px;
        padding-left: 2px;
        line-height: 1.3;
      }
      .task-context a {
        color: var(--text-dim, rgba(255,255,255,0.5));
        text-decoration: none;
      }
      .task-context a:hover {
        color: var(--accent, #14A8C4);
      }
      .task-id {
        font-family: var(--mono);
        font-size: 10px;
        opacity: 0.7;
      }
      .task-dot {
        display: inline-block;
        width: 6px;
        height: 6px;
        border-radius: 50%;
        flex-shrink: 0;
      }
      .task-dot.running {
        background: var(--success, #00b69b);
        animation: pulse-gauge 1.5s ease-in-out infinite;
      }
      .task-dot.completed {
        background: var(--text-dim, rgba(255,255,255,0.5));
        opacity: 0.6;
      }
      .task-status-text {
        font-size: 10px;
        opacity: 0.7;
      }
    `
  ];

  connectedCallback() {
    super.connectedCallback();
    this.ctrl.fetch(async () => {
      const resp = await this.fetchFn('/budget');
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      return resp.json();
    });
    // No automatic polling — only poll when a task is running (started by fetchActiveTask)
    if (!this.usdRate) {
      fetchBsvUsdRate().then(r => { this.usdRate = r; });
    }
    this.fetchActiveTask();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this._clearStatusTimer();
  }

  private _clearStatusTimer() {
    if (this.statusTimer !== null) {
      clearInterval(this.statusTimer);
      this.statusTimer = null;
    }
  }

  private async fetchActiveTask() {
    try {
      const resp = await this.fetchFn('/status');
      if (!resp.ok) return;
      const data = await resp.json();
      const tasks: TaskSummary[] = data.tasks ?? [];

      let pick: TaskSummary | null = null;
      if (tasks.length > 0) {
        // Prefer a running task, then most recent task with actual spending,
        // then most recent task overall (so the label is never blank).
        const running = tasks.find(t => t.status === 'running');
        if (running) {
          pick = running;
        } else {
          const sorted = [...tasks].sort((a, b) => parseTimestamp(b.started_at) - parseTimestamp(a.started_at));
          pick = sorted.find(t => (t.sats_spent ?? 0) > 0) ?? sorted[0] ?? null;
        }
      }

      // Only trigger re-render if task actually changed
      if (pick?.id !== this.activeTask?.id || pick?.sats_spent !== this.activeTask?.sats_spent) {
        this.activeTask = pick;
      }

      // Poll every 5s while a task is running; stop when idle
      const hasRunning = pick?.status === 'running';
      if (hasRunning && !this.statusTimer) {
        this.statusTimer = window.setInterval(() => this.fetchActiveTask(), 5000);
        this.ctrl.startPolling(5000);
      } else if (!hasRunning && this.statusTimer) {
        this._clearStatusTimer();
        this.ctrl.stopPolling();
      }
    } catch {
      // Non-critical — leave activeTask as-is
    }
  }

  private getGauges(): BudgetGaugeData[] {
    if (!this.ctrl.data) return [];
    const b = this.ctrl.data;
    // Use active task's sats_spent when available (covers both running and
    // most-recent completed tasks).  b.task_sats from the global tracker is
    // 0 after server restart, so the /status data is more reliable.
    const taskSpent = (this.activeTask && this.activeTask.sats_spent > 0)
      ? this.activeTask.sats_spent
      : b.task_sats;
    const taskRemaining = b.limits.max_per_task - taskSpent;
    const gauges: BudgetGaugeData[] = [
      { label: 'This Task', spent: taskSpent, limit: b.limits.max_per_task, remaining: Math.max(0, taskRemaining) },
      { label: 'Last Hour', spent: b.hourly_sats, limit: b.limits.max_per_hour, remaining: b.limits.hourly_remaining },
      { label: 'Last 24 Hours', spent: b.daily_sats, limit: b.limits.max_per_day, remaining: b.limits.daily_remaining },
    ];
    if (b.limits.max_per_week > 0) {
      gauges.push({ label: 'Last 7 Days', spent: b.weekly_sats ?? 0, limit: b.limits.max_per_week, remaining: b.limits.weekly_remaining ?? 0 });
    }
    if (b.limits.max_per_month > 0) {
      gauges.push({ label: 'Last 30 Days', spent: b.monthly_sats ?? 0, limit: b.limits.max_per_month, remaining: b.limits.monthly_remaining ?? 0 });
    }
    if (b.limits.max_lifetime > 0) {
      gauges.push({ label: 'Lifetime', spent: b.lifetime_sats ?? 0, limit: b.limits.max_lifetime, remaining: b.limits.lifetime_remaining ?? 0 });
    }
    return gauges;
  }

  private getAlertBanner() {
    const gauges = this.getGauges();
    let worstPct = 0;
    let worstLabel = '';
    for (const g of gauges) {
      const pct = g.limit > 0 ? (g.spent / g.limit) * 100 : 0;
      if (pct > worstPct) {
        worstPct = pct;
        worstLabel = g.label;
      }
    }
    if (worstPct >= 100) {
      return { severity: 'exceeded', message: `Budget limit reached (${worstLabel}). Task paused.` };
    }
    if (worstPct >= 95) {
      return { severity: 'critical', message: `Budget nearly exhausted: ${worstLabel} at ${Math.round(worstPct)}%` };
    }
    if (worstPct >= 80) {
      return { severity: 'warning', message: `Budget approaching limit: ${worstLabel} at ${Math.round(worstPct)}%` };
    }
    return null;
  }

  private renderTaskContext() {
    if (!this.activeTask) return nothing;
    const t = this.activeTask;
    const isRunning = t.status === 'running';
    const dotClass = isRunning ? 'running' : 'completed';
    const shortId = t.id.slice(0, 8);

    if (this.compact) {
      return html`
        <div class="task-context">
          <a href="#task/${t.id}">${truncate(t.task, 36)}</a>
          <span class="task-dot ${dotClass}"></span>
        </div>
      `;
    }

    return html`
      <div class="task-context">
        <a href="#task/${t.id}">"${truncate(t.task, 40)}"</a>
        <span class="task-id">(${shortId})</span>
        <span class="task-dot ${dotClass}"></span>
        <span class="task-status-text">${t.status}</span>
      </div>
    `;
  }

  private renderGauge(g: BudgetGaugeData) {
    const pct = g.limit > 0 ? Math.min((g.spent / g.limit) * 100, 100) : 0;
    const color = gaugeColor(pct);
    const pulseClass = pct >= 100 ? 'pulse' : '';
    const spentStr = formatInlineCurrency(g.spent, this.usdRate, this.currencyMode);
    const limitStr = formatInlineCurrency(g.limit, this.usdRate, this.currencyMode);
    const remainStr = formatInlineCurrency(g.remaining, this.usdRate, this.currencyMode);
    const altSpent = formatInlineCurrencyAlt(g.spent, this.usdRate, this.currencyMode);
    const altLimit = formatInlineCurrencyAlt(g.limit, this.usdRate, this.currencyMode);
    const altRemain = formatInlineCurrencyAlt(g.remaining, this.usdRate, this.currencyMode);
    const showTaskContext = g.label === 'This Task';

    return html`
      <div class="gauge">
        <div class="gauge-header">
          <span class="gauge-label">${g.label}${BUDGET_HELP[g.label] ? html`<dm-help text=${BUDGET_HELP[g.label]}></dm-help>` : nothing}</span>
          <span class="gauge-pct">${Math.round(pct)}%</span>
        </div>
        ${showTaskContext ? this.renderTaskContext() : nothing}
        <div class="gauge-track">
          <div class="gauge-fill ${pulseClass}" style="width: ${pct}%; background: ${color}"></div>
        </div>
        <div class="gauge-detail">
          <span title="${altSpent} spent">${spentStr} spent</span>
          <span title="${altRemain} remaining of ${altLimit} limit">${remainStr} left</span>
        </div>
      </div>
    `;
  }

  private renderServiceBars() {
    if (!this.ctrl.data) return nothing;
    const entries = Object.entries(this.ctrl.data.services)
      .sort(([, a], [, b]) => b.total_sats - a.total_sats);

    if (entries.length === 0) {
      return html`<p class="no-services">No spending recorded yet</p>`;
    }

    const maxSats = entries[0]?.[1].total_sats ?? 1;

    return entries.map(([name, data], i) => {
      const pct = maxSats > 0 ? (data.total_sats / maxSats) * 100 : 0;
      const color = SERVICE_COLORS[i % SERVICE_COLORS.length];
      return html`
        <div class="service-bar">
          <div class="service-header">
            <span class="service-label">${friendlyName(name)}</span>
            <span class="service-value" title="${formatInlineCurrencyAlt(data.total_sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(data.total_sats, this.usdRate, this.currencyMode)} (${data.count} ops)</span>
          </div>
          <div class="service-track">
            <div class="service-fill" style="width: ${pct}%; background: ${color}"></div>
          </div>
        </div>
      `;
    });
  }

  render() {
    if (this.ctrl.loading) {
      return renderLoading('Loading budget...');
    }
    if (this.ctrl.hasError) {
      return renderError(this.ctrl.error ?? 'Failed to load budget', 'Check that the agent server is running.');
    }
    if (!this.ctrl.data) {
      return renderEmpty('No budget data', 'Submit a task to start tracking budget usage.');
    }

    const alert = this.getAlertBanner();
    const gauges = this.getGauges();

    return html`
      ${!this.compact ? html`<h1 class="page-title">Budget</h1>` : nothing}

      ${alert ? html`
        <div class="alert-banner ${alert.severity}">${alert.message}</div>
      ` : nothing}

      <div class="budget-section">
        ${!this.compact ? html`<h2 class="section-title">Limits</h2>` : nothing}
        ${gauges.map(g => this.renderGauge(g))}
      </div>

      ${!this.compact ? html`
        <div class="budget-section">
          <h2 class="section-title">Spending by Service</h2>
          ${this.renderServiceBars()}
        </div>
      ` : nothing}

      ${!this.compact ? html`
        <div class="budget-section">
          <h2 class="section-title">Export</h2>
          <dm-spending-export .fetchFn=${this.fetchFn} .usdRate=${this.usdRate}></dm-spending-export>
        </div>
      ` : nothing}
    `;
  }
}
