/**
 * Reports & Accounting page — the CFO's home page.
 * Aggregates spending, task costs, service breakdown, and export controls.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { truncate, parseTimestamp, formatChartDate } from '../lib/util.js';
import { fetchBsvUsdRate, formatUsd, satsToUsd, formatDualCurrency, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import { friendlyName, SERVICE_COLORS } from '../lib/constants.js';
import type { TaskSummary, AgentInfo, BudgetDetailResponse, ServiceBreakdown } from '../lib/shared-types.js';
import { pageHost, centerState, pageTitle, stateFeedback, renderLoading, renderEmpty, renderError } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';
import { defaultRange, type DateRange, type DatePreset } from '../components/date-range.js';
import '../components/bar-chart.js';
import '../pages/budget/spending-export.js';
import type { BarData } from '../components/bar-chart.js';

interface ReportsData {
  tasks: TaskSummary[];
  agent: AgentInfo;
  budgetDetail: BudgetDetailResponse | null;
  usdRate: number;
}

/** Compute the previous period of equal duration before the current range. */
function computePreviousPeriod(range: DateRange): DateRange {
  const duration = range.to.getTime() - range.from.getTime();
  return {
    from: new Date(range.from.getTime() - duration),
    to: new Date(range.from.getTime()),
    preset: 'custom' as DatePreset,
  };
}

/** Compute percentage change: positive = increase, negative = decrease. */
function pctChange(current: number, previous: number): number | null {
  if (previous === 0) return current > 0 ? 100 : null;
  return ((current - previous) / previous) * 100;
}

@customElement('dm-reports')
export class WormReports extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  private ctrl = new FetchController<ReportsData>(this);
  @state() private dateRange: DateRange = defaultRange('mtd');

  static styles = [
    pageHost, centerState, pageTitle, stateFeedback,
    css`
      .reports-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
        margin-bottom: 24px;
      }

      /* Hero cards */
      .hero-grid {
        display: grid;
        grid-template-columns: repeat(4, 1fr);
        gap: 16px;
        margin-bottom: 24px;
      }
      @media (max-width: 639px) {
        .hero-grid { grid-template-columns: repeat(2, 1fr); }
      }
      .hero-card {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 20px;
      }
      .hero-label {
        font-size: 13px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-bottom: 4px;
      }
      .hero-value {
        font-family: var(--sans, sans-serif);
        font-size: 28px;
        font-weight: 800;
        color: var(--text-bright, #fff);
      }
      .hero-value.sats { color: var(--warning, #fcbe2d); }
      .hero-subtitle {
        font-family: var(--mono, monospace);
        font-size: 13px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-top: 2px;
      }
      .hero-badge {
        display: inline-block;
        font-size: 11px;
        font-weight: 700;
        font-family: var(--mono, monospace);
        padding: 2px 6px;
        border-radius: 4px;
        margin-top: 4px;
      }
      .hero-badge.up { background: rgba(0, 182, 155, 0.15); color: var(--success, #00b69b); }
      .hero-badge.down { background: rgba(253, 84, 84, 0.15); color: var(--error, #fd5454); }

      /* Sections */
      .report-section {
        margin-bottom: 24px;
      }
      .section-heading {
        font-size: 13px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin: 0 0 12px 0;
      }

      /* Full-width chart */
      .chart-full {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px;
      }

      /* Side-by-side */
      .chart-pair {
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 16px;
      }
      @media (max-width: 639px) {
        .chart-pair { grid-template-columns: 1fr; }
      }
      .chart-container {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px;
      }
      .chart-label {
        font-size: 12px;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin: 0 0 12px 0;
      }

      /* Service table */
      .service-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 13px;
      }
      .service-table th {
        text-align: left;
        font-size: 11px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        padding: 8px 12px;
        border-bottom: 1px solid var(--border, #1A3550);
      }
      .service-table td {
        padding: 8px 12px;
        border-bottom: 1px solid var(--border, #1A3550);
        color: var(--text, #e0e0e8);
      }
      .service-table tr:last-child td { border-bottom: none; }
      .service-dot {
        display: inline-block;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        margin-right: 8px;
        vertical-align: middle;
      }
      .service-ops {
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }
      .service-pct {
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }
      .service-amount {
        font-family: var(--mono, monospace);
        font-weight: 600;
        color: var(--warning, #fcbe2d);
        white-space: nowrap;
      }

      /* Top tasks table */
      .task-table {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: 8px;
        overflow: hidden;
      }
      .task-row {
        display: flex;
        align-items: center;
        gap: 12px;
        padding: 12px 20px;
        border-bottom: 1px solid var(--border, #1A3550);
        text-decoration: none;
        color: inherit;
        transition: background 0.15s ease;
        font-size: 13px;
      }
      .task-row:last-child { border-bottom: none; }
      .task-row:hover { background: rgba(255, 255, 255, 0.03); }
      .task-rank {
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        min-width: 24px;
      }
      .task-desc {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        color: var(--text, #e0e0e8);
      }
      .task-date {
        font-family: var(--mono, monospace);
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        white-space: nowrap;
      }
      .task-iters {
        font-family: var(--mono, monospace);
        font-size: 11px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        white-space: nowrap;
      }
      .task-cost {
        font-family: var(--mono, monospace);
        font-weight: 600;
        color: var(--warning, #fcbe2d);
        font-size: 12px;
        white-space: nowrap;
      }

      /* Cost distribution */
      .dist-bar-row {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-bottom: 8px;
        font-size: 13px;
      }
      .dist-label {
        min-width: 90px;
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        text-align: right;
      }
      .dist-track {
        flex: 1;
        height: 20px;
        background: var(--bg, #0B1929);
        border-radius: 4px;
        overflow: hidden;
      }
      .dist-fill {
        height: 100%;
        background: var(--accent, #14A8C4);
        border-radius: 4px;
        transition: width 0.3s ease;
      }
      .dist-count {
        min-width: 60px;
        font-family: var(--mono, monospace);
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      /* Export section */
      .export-section {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 24px;
        text-align: center;
      }
      .export-title {
        font-size: 14px;
        font-weight: 600;
        color: var(--text, #e0e0e8);
        margin-bottom: 12px;
      }
      .export-buttons {
        display: flex;
        gap: 12px;
        justify-content: center;
        margin-bottom: 12px;
      }
      .export-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 10px 20px;
        background: var(--bg, #0B1929);
        border: 1px solid var(--border, #1A3550);
        border-radius: 8px;
        color: var(--text, #e0e0e8);
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        transition: all 0.15s ease;
      }
      .export-btn:hover {
        background: var(--accent, #14A8C4);
        color: #fff;
        border-color: var(--accent, #14A8C4);
      }
      .export-btn:disabled {
        opacity: 0.5;
        cursor: not-allowed;
      }
      .export-summary {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-family: var(--mono, monospace);
      }

      .empty-hint {
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-size: 12px;
        padding: 16px;
        text-align: center;
      }

      /* Print styles */
      .print-footer { display: none; }
      @media print {
        :host {
          background: white !important;
          color: #1a1a1a !important;
          padding: 24px !important;
        }
        .no-print { display: none !important; }
        .hero-card {
          background: #f8f9fa !important;
          border-color: #e0e0e0 !important;
          color: #1a1a1a !important;
        }
        .hero-value { color: #1a1a1a !important; }
        .hero-value.sats { color: #b8860b !important; }
        .hero-label { color: #666 !important; }
        .hero-subtitle { color: #666 !important; }
        .chart-full, .chart-container, .task-table, .export-section {
          background: #f8f9fa !important;
          border-color: #e0e0e0 !important;
        }
        .section-heading { color: #333 !important; }
        .service-table th { color: #666 !important; }
        .service-table td { color: #1a1a1a !important; }
        .task-desc { color: #1a1a1a !important; }
        .dist-track { background: #e0e0e0 !important; }
        .print-footer {
          display: block !important;
          text-align: center;
          font-size: 10px;
          color: #999;
          margin-top: 24px;
          border-top: 1px solid #e0e0e0;
          padding-top: 12px;
        }
      }
    `,
  ];

  firstUpdated() {
    this.loadData();
  }

  private async loadData() {
    await this.ctrl.fetch(async () => {
      const [tasksRes, agentRes, detailRes, usdRate] = await Promise.all([
        this.fetchFn('/tasks'),
        this.fetchFn('/agent'),
        this.fetchFn('/budget/detail').catch(() => null),
        fetchBsvUsdRate(),
      ]);

      if (!tasksRes.ok) throw new Error(`Tasks: ${tasksRes.status}`);
      if (!agentRes.ok) throw new Error(`Agent: ${agentRes.status}`);

      const tasksData = await tasksRes.json();
      const tasks: TaskSummary[] = tasksData.tasks ?? [];
      const agent: AgentInfo = await agentRes.json();

      let budgetDetail: BudgetDetailResponse | null = null;
      if (detailRes && detailRes.ok) {
        budgetDetail = await detailRes.json();
      }

      return { tasks, agent, budgetDetail, usdRate };
    });
  }

  /** Filter tasks to those started within the date range. */
  private filteredTasks(): TaskSummary[] {
    const tasks = this.ctrl.data?.tasks ?? [];
    const fromTs = this.dateRange.from.getTime() / 1000;
    const toTs = this.dateRange.to.getTime() / 1000;
    return tasks.filter(t => {
      const ts = parseTimestamp(t.started_at);
      return ts >= fromTs && ts <= toTs;
    });
  }

  /** Filter spending entries to the date range. */
  private filteredEntries() {
    const entries = this.ctrl.data?.budgetDetail?.entries ?? [];
    const fromMs = this.dateRange.from.getTime();
    const toMs = this.dateRange.to.getTime();
    return entries.filter(e => {
      const t = new Date(e.timestamp).getTime();
      return t >= fromMs && t <= toMs;
    });
  }

  /** Compute total sats from filtered entries. */
  private periodTotalSats(): number {
    return this.filteredEntries().reduce((s, e) => s + e.sats, 0);
  }

  /** Compute previous-period sats for comparison. */
  private previousPeriodSats(): number {
    const prev = computePreviousPeriod(this.dateRange);
    const entries = this.ctrl.data?.budgetDetail?.entries ?? [];
    const fromMs = prev.from.getTime();
    const toMs = prev.to.getTime();
    return entries.filter(e => {
      const t = new Date(e.timestamp).getTime();
      return t >= fromMs && t <= toMs;
    }).reduce((s, e) => s + e.sats, 0);
  }

  /** Previous period task count for comparison. */
  private previousPeriodTaskCount(): number {
    const prev = computePreviousPeriod(this.dateRange);
    const tasks = this.ctrl.data?.tasks ?? [];
    const fromTs = prev.from.getTime() / 1000;
    const toTs = prev.to.getTime() / 1000;
    return tasks.filter(t => {
      const ts = parseTimestamp(t.started_at);
      return ts >= fromTs && ts <= toTs;
    }).length;
  }

  /** Service breakdown computed from filtered entries. */
  private computeServiceData(): Array<{ service: string; sats: number; ops: number }> {
    const entries = this.filteredEntries();
    const map = new Map<string, { sats: number; ops: number }>();
    for (const e of entries) {
      const s = map.get(e.service) ?? { sats: 0, ops: 0 };
      s.sats += e.sats;
      s.ops += 1;
      map.set(e.service, s);
    }
    return [...map.entries()]
      .map(([service, v]) => ({ service, ...v }))
      .sort((a, b) => b.sats - a.sats);
  }

  /** Trend data for line chart. */
  private computeTrendData(): BarData[] {
    const entries = this.filteredEntries();
    if (entries.length === 0) return [];
    const groups = new Map<string, number>();
    for (const e of entries) {
      const d = new Date(e.timestamp);
      const key = `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;
      groups.set(key, (groups.get(key) || 0) + e.sats);
    }
    const sorted = [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0])).slice(-30);
    // Show year if range spans multiple years
    const years = new Set(sorted.map(([k]) => k.slice(0, 4)));
    const showYear = years.size > 1;
    return sorted.map(([key, value]) => ({
      label: formatChartDate(new Date(key + 'T00:00:00'), showYear),
      value,
    }));
  }

  /** Cost distribution buckets. */
  private computeCostDistribution(): Array<{ label: string; count: number; pct: number }> {
    const tasks = this.filteredTasks();
    const total = tasks.length || 1;
    const buckets = [
      { label: '< $0.01', min: 0, max: 0.01, count: 0 },
      { label: '$0.01 - $0.10', min: 0.01, max: 0.10, count: 0 },
      { label: '$0.10 - $1.00', min: 0.10, max: 1.00, count: 0 },
      { label: '$1.00+', min: 1.00, max: Infinity, count: 0 },
    ];
    const rate = this.ctrl.data?.usdRate ?? 0;
    for (const t of tasks) {
      const usd = rate > 0 ? satsToUsd(t.sats_spent, rate) : t.sats_spent / 100_000_000;
      for (const b of buckets) {
        if (usd >= b.min && usd < b.max) { b.count++; break; }
      }
    }
    return buckets.map(b => ({ label: b.label, count: b.count, pct: (b.count / total) * 100 }));
  }

  private renderComparisonBadge(current: number, previous: number) {
    const pct = pctChange(current, previous);
    if (pct === null) return nothing;
    const isUp = pct >= 0;
    const label = `${isUp ? '+' : ''}${pct.toFixed(0)}%`;
    return html`<span class="hero-badge ${isUp ? 'up' : 'down'}">${isUp ? '\u2191' : '\u2193'} ${label}</span>`;
  }

  private renderHero() {
    const d = this.ctrl.data!;
    const tasks = this.filteredTasks();
    const totalSats = this.periodTotalSats();
    const taskCount = tasks.length;
    const avgSats = taskCount > 0 ? Math.round(totalSats / taskCount) : 0;
    const balance = d.agent?.balance ?? 0;

    const prevSats = this.previousPeriodSats();
    const prevTasks = this.previousPeriodTaskCount();
    const prevAvg = prevTasks > 0 ? Math.round(prevSats / prevTasks) : 0;

    return html`
      <div class="hero-grid">
        <div class="hero-card">
          <div class="hero-label">Total Spend</div>
          <div class="hero-value sats">${formatDualCurrency(totalSats, d.usdRate, this.currencyMode).primary}</div>
          <div class="hero-subtitle">${formatDualCurrency(totalSats, d.usdRate, this.currencyMode).secondary}</div>
          ${this.renderComparisonBadge(totalSats, prevSats)}
        </div>
        <div class="hero-card">
          <div class="hero-label">Tasks Run</div>
          <div class="hero-value">${taskCount.toLocaleString()}</div>
          <div class="hero-subtitle">this period</div>
          ${this.renderComparisonBadge(taskCount, prevTasks)}
        </div>
        <div class="hero-card">
          <div class="hero-label">Avg / Task</div>
          <div class="hero-value sats">${formatDualCurrency(avgSats, d.usdRate, this.currencyMode).primary}</div>
          <div class="hero-subtitle">${formatDualCurrency(avgSats, d.usdRate, this.currencyMode).secondary}</div>
          ${this.renderComparisonBadge(avgSats, prevAvg)}
        </div>
        <div class="hero-card">
          <div class="hero-label">Balance</div>
          <div class="hero-value sats">${formatDualCurrency(balance, d.usdRate, this.currencyMode).primary}</div>
          <div class="hero-subtitle">${formatDualCurrency(balance, d.usdRate, this.currencyMode).secondary}</div>
        </div>
      </div>
    `;
  }

  private renderTrend() {
    const trendData = this.computeTrendData();
    if (trendData.length === 0) return nothing;

    return html`
      <section class="report-section">
        <h3 class="section-heading">Spending Trend</h3>
        <div class="chart-full">
          <dm-bar-chart chartType="line" .data=${trendData} .maxBars=${30} .usdRate=${this.ctrl.data?.usdRate ?? 0} .currencyMode=${this.currencyMode} height="280px"></dm-bar-chart>
        </div>
      </section>
    `;
  }

  private renderServiceBreakdown() {
    const services = this.computeServiceData();
    const d = this.ctrl.data!;
    if (services.length === 0) return nothing;

    const totalSats = this.periodTotalSats();
    const donutData: BarData[] = services.slice(0, 8).map(s => ({
      label: friendlyName(s.service),
      value: s.sats,
    }));

    return html`
      <section class="report-section">
        <h3 class="section-heading">Service Breakdown</h3>
        <div class="chart-pair">
          <div class="chart-container">
            <h4 class="chart-label">By Service</h4>
            <dm-bar-chart chartType="doughnut" .data=${donutData} .usdRate=${this.ctrl.data?.usdRate ?? 0} .currencyMode=${this.currencyMode} height="240px"></dm-bar-chart>
          </div>
          <div class="chart-container">
            <h4 class="chart-label">Details</h4>
            <table class="service-table">
              <thead>
                <tr><th>Service</th><th>Ops</th><th>Amount</th><th>%</th></tr>
              </thead>
              <tbody>
                ${services.map((s, i) => {
                  const pct = totalSats > 0 ? ((s.sats / totalSats) * 100).toFixed(1) : '0';
                  const color = SERVICE_COLORS[i % SERVICE_COLORS.length];
                  return html`
                    <tr>
                      <td><span class="service-dot" style="background:${color}"></span>${friendlyName(s.service)}</td>
                      <td class="service-ops">${s.ops}</td>
                      <td class="service-amount" title="${formatInlineCurrencyAlt(s.sats, d.usdRate, this.currencyMode)}">${formatInlineCurrency(s.sats, d.usdRate, this.currencyMode)}</td>
                      <td class="service-pct">${pct}%</td>
                    </tr>
                  `;
                })}
              </tbody>
            </table>
          </div>
        </div>
      </section>
    `;
  }

  private renderTopTasks() {
    const d = this.ctrl.data!;
    const tasks = [...this.filteredTasks()]
      .sort((a, b) => b.sats_spent - a.sats_spent)
      .slice(0, 10);

    if (tasks.length === 0) return nothing;

    return html`
      <section class="report-section">
        <h3 class="section-heading">Top Tasks by Cost</h3>
        <div class="task-table">
          ${tasks.map((t, i) => {
            const date = new Date(parseTimestamp(t.started_at) * 1000);
            const dateStr = formatChartDate(date);
            return html`
              <a class="task-row" href="#task/${t.id}">
                <span class="task-rank">${i + 1}.</span>
                <span class="task-desc">${truncate(t.task, 55)}</span>
                <span class="task-date">${dateStr}</span>
                <span class="task-iters">${t.iterations} iter</span>
                <span class="task-cost" title="${formatInlineCurrencyAlt(t.sats_spent, d.usdRate, this.currencyMode)}">${formatInlineCurrency(t.sats_spent, d.usdRate, this.currencyMode)}</span>
              </a>
            `;
          })}
        </div>
      </section>
    `;
  }

  private renderCostDistribution() {
    const dist = this.computeCostDistribution();
    const maxCount = Math.max(...dist.map(d => d.count), 1);

    return html`
      <section class="report-section">
        <h3 class="section-heading">Cost Distribution</h3>
        <div class="chart-full" style="padding: 20px 24px">
          ${dist.map(d => html`
            <div class="dist-bar-row">
              <span class="dist-label">${d.label}</span>
              <div class="dist-track">
                <div class="dist-fill" style="width: ${(d.count / maxCount) * 100}%"></div>
              </div>
              <span class="dist-count">${d.count} task${d.count !== 1 ? 's' : ''}</span>
            </div>
          `)}
        </div>
      </section>
    `;
  }

  @state() private exportingJson = false;

  private async exportSignedJson() {
    this.exportingJson = true;
    try {
      const entries = this.filteredEntries();
      const tasks = this.filteredTasks();
      const d = this.ctrl.data!;
      const report = {
        generated_at: new Date().toISOString(),
        period: {
          from: this.dateRange.from.toISOString(),
          to: this.dateRange.to.toISOString(),
          preset: this.dateRange.preset,
        },
        summary: {
          total_sats: this.periodTotalSats(),
          total_usd: d.usdRate > 0 ? formatUsd(this.periodTotalSats(), d.usdRate) : null,
          task_count: tasks.length,
          proof_count: tasks.reduce((c, t) => c + (t.proof_txids?.length ?? 0), 0),
        },
        services: this.computeServiceData(),
        tasks: tasks.map(t => ({
          id: t.id,
          task: t.task,
          status: t.status,
          iterations: t.iterations,
          sats_spent: t.sats_spent,
          started_at: t.started_at,
          proof_txids: t.proof_txids,
        })),
        entries: entries.map(e => ({
          timestamp: e.timestamp,
          service: e.service,
          sats: e.sats,
          operation: e.operation,
        })),
      };

      const json = JSON.stringify(report, null, 2);
      const blob = new Blob([json], { type: 'application/json;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `dolphin-milk-report-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      console.error('JSON export failed:', e);
    } finally {
      this.exportingJson = false;
    }
  }

  private renderExportSection() {
    const d = this.ctrl.data!;
    const tasks = this.filteredTasks();
    const totalSats = this.periodTotalSats();
    const proofCount = tasks.reduce((c, t) => c + (t.proof_txids?.length ?? 0), 0);
    const from = this.dateRange.from.toLocaleDateString();
    const to = this.dateRange.to.toLocaleDateString();

    return html`
      <section class="report-section">
        <div class="export-section">
          <div class="export-title">Export Reports</div>
          <div class="export-buttons no-print">
            <dm-spending-export .fetchFn=${this.fetchFn} .usdRate=${d.usdRate}></dm-spending-export>
            <button class="export-btn" @click=${this.exportSignedJson} ?disabled=${this.exportingJson}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="width:14px;height:14px">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                <polyline points="14 2 14 8 20 8"/>
              </svg>
              ${this.exportingJson ? 'Exporting...' : 'JSON Report'}
            </button>
          </div>
          <div class="export-summary">
            ${from} \u2013 ${to} &middot; ${tasks.length} tasks &middot;
            ${formatInlineCurrency(totalSats, d.usdRate, this.currencyMode)} total &middot;
            ${proofCount} proofs
          </div>
        </div>
      </section>
    `;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) {
      return renderLoading('Loading reports...');
    }
    if (this.ctrl.hasError && !this.ctrl.hasData) {
      return renderError(this.ctrl.error ?? 'Failed to load reports', 'Check that the agent server is running.');
    }
    if (!this.ctrl.data) {
      return renderEmpty('No data available', 'Submit a task to start generating reports.');
    }

    return html`
      <div class="reports-header">
        <div class="page-title" style="margin:0">Reports</div>
        <dm-date-range
          class="no-print"
          .value=${this.dateRange}
          @range-change=${(e: CustomEvent) => { this.dateRange = e.detail; }}
        ></dm-date-range>
      </div>
      ${this.renderHero()}
      ${this.renderTrend()}
      ${this.renderServiceBreakdown()}
      ${this.renderTopTasks()}
      ${this.renderCostDistribution()}
      ${this.renderExportSection()}
      <div class="print-footer">
        Generated by Dolphin Milk on ${new Date().toLocaleDateString()}
      </div>
    `;
  }
}
