/**
 * Simplified Dashboard — 30-second health check.
 * Answers: "Is everything OK?" — Balance, today's stats, recent conversations,
 * budget health gauge, 7-day spending trend.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, truncate } from '../lib/util.js';
import { fetchBsvUsdRate, formatDualCurrency, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import {
  type TaskSummary,
  type AgentInfo,
  type BudgetReport,
  type BudgetDetailResponse,
  type ConversationSummary,
} from '../lib/shared-types.js';
import { pageHost, centerState, pageTitle, stateFeedback, skeleton, renderEmpty, renderLoading, renderError } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';
import '../components/bar-chart.js';
import '../components/sparkline.js';
import type { BarData } from '../components/bar-chart.js';

interface DashboardData {
  tasks: TaskSummary[];
  agent: AgentInfo;
  budget: BudgetReport | null;
  budgetDetail: BudgetDetailResponse | null;
  conversations: ConversationSummary[];
  usdRate: number;
}

@customElement('dm-dashboard')
export class WormDashboard extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  private ctrl = new FetchController<DashboardData>(this);

  static styles = [
    pageHost, centerState, pageTitle, stateFeedback, skeleton,
    css`
      .page-title { margin-bottom: 20px; }

      /* Balance hero */
      .balance-hero {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 28px 32px;
        text-align: center;
        margin-bottom: 20px;
        cursor: pointer;
        transition: border-color 0.2s ease, box-shadow 0.2s ease;
      }

      .balance-hero:hover {
        border-color: var(--accent, #14A8C4);
        box-shadow: 0 4px 20px rgba(0, 0, 0, 0.2);
      }

      .balance-amount {
        font-family: var(--sans, sans-serif);
        font-size: 40px;
        font-weight: 800;
        color: var(--text-bright, #fff);
        line-height: 1.1;
      }

      .balance-secondary {
        font-family: var(--mono, monospace);
        font-size: 14px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-top: 4px;
      }

      .balance-label {
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-top: 8px;
      }

      /* Today stats row */
      .today-row {
        display: grid;
        grid-template-columns: repeat(3, 1fr);
        gap: 12px;
        margin-bottom: 20px;
      }

      @media (max-width: 639px) {
        .today-row { grid-template-columns: 1fr; gap: 8px; }
        .balance-amount { font-size: 32px; }
        .balance-hero { padding: 20px; }
      }

      .today-card {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px 20px;
        display: flex;
        flex-direction: column;
        gap: 2px;
      }

      .today-value {
        font-family: var(--sans, sans-serif);
        font-size: 22px;
        font-weight: 700;
        color: var(--text-bright, #fff);
      }

      .today-value.cost { color: var(--warning, #fcbe2d); }

      .today-label {
        font-size: 11px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .today-sub {
        font-family: var(--mono, monospace);
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      /* Budget gauge */
      .budget-section {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px 20px;
        margin-bottom: 20px;
        cursor: pointer;
        transition: border-color 0.2s ease;
      }

      .budget-section:hover { border-color: var(--accent, #14A8C4); }

      .budget-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 10px;
      }

      .budget-title {
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .budget-pct {
        font-family: var(--mono, monospace);
        font-size: 13px;
        font-weight: 600;
      }

      .budget-bar {
        height: 8px;
        border-radius: 4px;
        background: var(--bg, #0B1929);
        overflow: hidden;
      }

      .budget-fill {
        height: 100%;
        border-radius: 4px;
        transition: width 0.4s ease;
      }

      .budget-detail {
        display: flex;
        justify-content: space-between;
        margin-top: 6px;
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-family: var(--mono, monospace);
      }

      /* Recent conversations */
      .section-header {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-bottom: 10px;
      }

      .section-title {
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .section-link {
        font-size: 12px;
        color: var(--accent, #14A8C4);
        text-decoration: none;
      }

      .section-link:hover { text-decoration: underline; }

      .conv-list {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        overflow: hidden;
        margin-bottom: 20px;
      }

      .conv-item {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px 16px;
        border-bottom: 1px solid var(--border, #1A3550);
        text-decoration: none;
        color: inherit;
        transition: background 0.12s ease;
      }

      .conv-item:last-child { border-bottom: none; }
      .conv-item:hover { background: rgba(255, 255, 255, 0.03); }

      .conv-title {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        color: var(--text, #e0e0e8);
        font-size: 13px;
      }

      .conv-meta {
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        white-space: nowrap;
      }

      .conv-cost {
        font-family: var(--mono, monospace);
        font-size: 12px;
        font-weight: 600;
        color: var(--warning, #fcbe2d);
        white-space: nowrap;
      }

      .conv-msgs {
        font-family: var(--mono, monospace);
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        white-space: nowrap;
        min-width: 40px;
        text-align: right;
      }

      /* Spending trend */
      .trend-section {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px 20px;
        margin-bottom: 20px;
      }

      .trend-header {
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-bottom: 12px;
      }

      .empty-hint {
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-size: 13px;
        text-align: center;
        padding: 24px 16px;
      }

      .empty-hint a {
        color: var(--accent, #14A8C4);
        text-decoration: none;
      }

      .empty-hint a:hover { text-decoration: underline; }

      .active-badge {
        display: inline-block;
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: var(--success, #00b69b);
        animation: pulse-active 2s ease-in-out infinite;
        margin-left: 4px;
        vertical-align: middle;
      }

      @keyframes pulse-active {
        0%, 100% { opacity: 1; }
        50% { opacity: 0.4; }
      }
    `,
  ];

  firstUpdated() {
    this.loadDashboard();
  }

  private async loadDashboard() {
    await this.ctrl.fetch(async () => {
      const [tasksRes, agentRes, budgetRes, detailRes, convsRes, usdRate] = await Promise.all([
        this.fetchFn('/tasks'),
        this.fetchFn('/agent'),
        this.fetchFn('/budget').catch(() => null),
        this.fetchFn('/budget/detail').catch(() => null),
        this.fetchFn('/conversations').catch(() => null),
        fetchBsvUsdRate(),
      ]);

      if (!tasksRes.ok) throw new Error(`Tasks: ${tasksRes.status}`);
      if (!agentRes.ok) throw new Error(`Agent: ${agentRes.status}`);

      const tasksData = await tasksRes.json();
      const tasks: TaskSummary[] = tasksData.tasks ?? [];
      const agent: AgentInfo = await agentRes.json();

      let budget: BudgetReport | null = null;
      if (budgetRes && budgetRes.ok) budget = await budgetRes.json();

      let budgetDetail: BudgetDetailResponse | null = null;
      if (detailRes && detailRes.ok) budgetDetail = await detailRes.json();

      let conversations: ConversationSummary[] = [];
      if (convsRes && convsRes.ok) {
        const data = await convsRes.json();
        conversations = Array.isArray(data) ? data : [];
      }

      return { tasks, agent, budget, budgetDetail, conversations, usdRate };
    });
  }

  /** Compute the highest-utilization budget tier for the single gauge. */
  private budgetGauge(): { label: string; pct: number; spent: number; limit: number } {
    const b = this.ctrl.data?.budget;
    if (!b) return { label: 'Daily', pct: 0, spent: 0, limit: 0 };

    const tiers: { label: string; spent: number; limit: number }[] = [
      { label: 'Hourly', spent: b.hourly_sats, limit: b.limits.max_per_hour },
      { label: 'Daily', spent: b.daily_sats, limit: b.limits.max_per_day },
      { label: 'Weekly', spent: b.weekly_sats, limit: b.limits.max_per_week },
      { label: 'Monthly', spent: b.monthly_sats, limit: b.limits.max_per_month },
    ].filter(t => t.limit > 0);

    if (tiers.length === 0) return { label: 'Daily', pct: 0, spent: 0, limit: 0 };

    let highest = tiers[0];
    let highestPct = 0;
    for (const t of tiers) {
      const pct = Math.min(100, Math.round((t.spent / t.limit) * 100));
      if (pct > highestPct) { highest = t; highestPct = pct; }
    }
    return { ...highest, pct: highestPct };
  }

  private gaugeColor(pct: number): string {
    if (pct < 60) return 'var(--success, #00b69b)';
    if (pct < 80) return 'var(--warning, #fcbe2d)';
    if (pct < 95) return '#e87a3a';
    return 'var(--error, #fd5454)';
  }

  /** Today's stats computed from tasks. */
  private todayStats(): { convs: number; cost: number; active: number } {
    const tasks = this.ctrl.data?.tasks ?? [];
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);
    const todayTs = todayStart.getTime() / 1000;

    let cost = 0;
    let active = 0;
    const todayTaskIds = new Set<string>();

    for (const t of tasks) {
      const started = new Date(t.started_at).getTime() / 1000;
      if (started >= todayTs) {
        cost += t.sats_spent ?? 0;
        todayTaskIds.add(t.id);
      }
      if (t.status === 'running') active++;
    }

    // Count conversations that have tasks started today
    const convs = (this.ctrl.data?.conversations ?? []).filter(c =>
      c.task_ids?.some(tid => todayTaskIds.has(tid))
    ).length;

    return { convs, cost, active };
  }

  /** 7-day spending for trend line chart. */
  private computeWeeklyTrend(): BarData[] {
    const budgetDetail = this.ctrl.data?.budgetDetail;
    if (!budgetDetail?.entries?.length) return [];
    const now = Date.now();
    const days = 7;
    const buckets = new Array(days).fill(0);
    const labels: string[] = [];

    for (let i = 0; i < days; i++) {
      const d = new Date(now - (days - 1 - i) * 86_400_000);
      labels.push(d.toLocaleDateString(undefined, { weekday: 'short' }));
    }

    for (const e of budgetDetail.entries) {
      const t = new Date(e.timestamp).getTime();
      const daysAgo = Math.floor((now - t) / 86_400_000);
      if (daysAgo >= 0 && daysAgo < days) {
        buckets[days - 1 - daysAgo] += e.sats;
      }
    }

    return labels.map((label, i) => ({ label, value: buckets[i] }));
  }

  /** Sparkline data for balance hero (7-day running balance — not ideal, use spending). */
  private computeSparklineData(): number[] {
    const budgetDetail = this.ctrl.data?.budgetDetail;
    if (!budgetDetail?.entries?.length) return [];
    const now = Date.now();
    const days = 7;
    const buckets = new Array(days).fill(0);
    for (const e of budgetDetail.entries) {
      const t = new Date(e.timestamp).getTime();
      const daysAgo = Math.floor((now - t) / 86_400_000);
      if (daysAgo >= 0 && daysAgo < days) {
        buckets[days - 1 - daysAgo] += e.sats;
      }
    }
    return buckets;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) {
      return html`
        <div class="page-title">Dashboard</div>
        <div class="skeleton skeleton-card" style="height:120px;margin-bottom:20px"></div>
        <div class="skeleton-hero-grid" style="margin-bottom:20px">
          ${[0,1,2].map(() => html`<div class="skeleton skeleton-card" style="height:80px"></div>`)}
        </div>
      `;
    }
    if (this.ctrl.hasError && !this.ctrl.hasData) {
      return renderError(this.ctrl.error ?? 'Failed to load dashboard', 'Check that the agent server is running.');
    }
    if (!this.ctrl.data) {
      return renderEmpty('No dashboard data available', 'Submit a task to start seeing activity here.');
    }

    return html`
      <div class="page-title">Dashboard</div>
      ${this.renderBalanceHero()}
      ${this.renderTodayStats()}
      ${this.renderBudgetGauge()}
      ${this.renderRecentConversations()}
      ${this.renderSpendingTrend()}
    `;
  }

  private renderBalanceHero() {
    const d = this.ctrl.data!;
    const balance = d.agent?.balance ?? 0;
    const { primary, secondary } = formatDualCurrency(balance, d.usdRate, this.currencyMode);

    return html`
      <a class="balance-hero" href="#budget" style="text-decoration:none;display:block">
        <div class="balance-amount" title="${formatInlineCurrencyAlt(balance, d.usdRate, this.currencyMode)}">${primary}</div>
        ${secondary ? html`<div class="balance-secondary">${secondary}</div>` : nothing}
        <div class="balance-label">Balance</div>
      </a>
    `;
  }

  private renderTodayStats() {
    const d = this.ctrl.data!;
    const { convs, cost, active } = this.todayStats();

    return html`
      <div class="today-row">
        <div class="today-card">
          <span class="today-value">${convs}${active > 0 ? html`<span class="active-badge"></span>` : nothing}</span>
          <span class="today-label">Conversations today</span>
          ${active > 0 ? html`<span class="today-sub">${active} active now</span>` : nothing}
        </div>
        <div class="today-card">
          <span class="today-value cost" title="${formatInlineCurrencyAlt(cost, d.usdRate, this.currencyMode)}">${formatInlineCurrency(cost, d.usdRate, this.currencyMode)}</span>
          <span class="today-label">Spent today</span>
        </div>
        <div class="today-card">
          <span class="today-value">${(d.tasks ?? []).length.toLocaleString()}</span>
          <span class="today-label">Total tasks</span>
          <span class="today-sub">${(d.agent?.total_sats_spent ? formatInlineCurrency(d.agent.total_sats_spent, d.usdRate, this.currencyMode) : '0 sats')} all time</span>
        </div>
      </div>
    `;
  }

  private renderBudgetGauge() {
    const gauge = this.budgetGauge();
    if (gauge.limit === 0) return nothing;
    const color = this.gaugeColor(gauge.pct);
    const d = this.ctrl.data!;

    return html`
      <a class="budget-section" href="#budget" style="text-decoration:none;display:block">
        <div class="budget-header">
          <span class="budget-title">Budget Health (${gauge.label})</span>
          <span class="budget-pct" style="color:${color}">${gauge.pct}%</span>
        </div>
        <div class="budget-bar">
          <div class="budget-fill" style="width:${Math.min(gauge.pct, 100)}%;background:${color}"></div>
        </div>
        <div class="budget-detail">
          <span>${formatInlineCurrency(gauge.spent, d.usdRate, this.currencyMode)} spent</span>
          <span>${formatInlineCurrency(gauge.limit - gauge.spent, d.usdRate, this.currencyMode)} left</span>
        </div>
      </a>
    `;
  }

  private renderRecentConversations() {
    const d = this.ctrl.data!;
    const convs = (d.conversations ?? []).slice(0, 5);

    return html`
      <div class="section-header">
        <span class="section-title">Recent Conversations</span>
        <a class="section-link" href="#conversations">All Chats \u2192</a>
      </div>
      ${convs.length > 0 ? html`
        <div class="conv-list">
          ${convs.map(c => html`
            <a class="conv-item" href="#conversation/${c.id}">
              <span class="conv-title">${c.title || 'Untitled'}</span>
              <span class="conv-meta">${formatRelativeTime(new Date(c.updated_at).getTime() / 1000)}</span>
              <span class="conv-cost" title="${formatInlineCurrencyAlt(c.total_sats, d.usdRate, this.currencyMode)}">${formatInlineCurrency(c.total_sats, d.usdRate, this.currencyMode)}</span>
              <span class="conv-msgs">${c.message_count} msg${c.message_count !== 1 ? 's' : ''}</span>
            </a>
          `)}
        </div>
      ` : html`
        <div class="conv-list">
          <div class="empty-hint">No conversations yet. <a href="#">Start chatting</a> to see activity here.</div>
        </div>
      `}
    `;
  }

  private renderSpendingTrend() {
    const trendData = this.computeWeeklyTrend();
    const hasData = trendData.some(d => d.value > 0);

    if (!hasData) return nothing;

    return html`
      <div class="trend-section">
        <div class="trend-header">7-Day Spending</div>
        <dm-bar-chart
          chartType="line"
          .data=${trendData}
          .usdRate=${this.ctrl.data?.usdRate ?? 0}
          .currencyMode=${this.currencyMode}
          height="160px"
        ></dm-bar-chart>
      </div>
    `;
  }
}
