/**
 * <dm-telemetry> - Prometheus metrics telemetry dashboard.
 *
 * Fetches GET /metrics (Prometheus text format), parses the response,
 * and renders live counters, gauges, and distributions. Auto-refreshes
 * every 10 seconds.
 */
import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

/** Parsed metric sample: name, labels, value. */
interface MetricSample {
  name: string;
  labels: Record<string, string>;
  value: number;
}

/** Parse Prometheus text-format exposition into MetricSample[]. */
function parsePrometheusText(text: string): MetricSample[] {
  const samples: MetricSample[] = [];
  for (const line of text.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;

    // Match: metric_name{label="value",...} value
    // or:    metric_name value
    const match = trimmed.match(/^([a-zA-Z_:][a-zA-Z0-9_:]*)\{?([^}]*)?\}?\s+(.+)$/);
    if (!match) continue;

    const name = match[1];
    const labelStr = match[2] || '';
    const value = parseFloat(match[3]);
    if (isNaN(value)) continue;

    const labels: Record<string, string> = {};
    if (labelStr) {
      // Parse label="value" pairs
      const labelRegex = /(\w+)="([^"]*)"/g;
      let labelMatch;
      while ((labelMatch = labelRegex.exec(labelStr)) !== null) {
        labels[labelMatch[1]] = labelMatch[2];
      }
    }

    samples.push({ name, labels, value });
  }
  return samples;
}

@customElement('dm-telemetry')
export class WormTelemetry extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);

  @state() private samples: MetricSample[] = [];
  @state() private loading = true;
  @state() private error: string | null = null;
  @state() private lastRefresh: Date | null = null;

  private refreshTimer: number | null = null;

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      padding: 24px;
      gap: 20px;
      overflow-y: auto;
      color: var(--text, rgba(255,255,255,0.8));
      font-family: var(--sans, system-ui, sans-serif);
    }

    .page-title {
      font-size: 24px;
      font-weight: 700;
      color: var(--text-bright, #fff);
      margin: 0 0 4px 0;
    }

    .subtitle {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-family: var(--mono, monospace);
    }

    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
      gap: 16px;
    }

    .card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      padding: 16px;
    }

    .card-title {
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-bottom: 8px;
    }

    .card-value {
      font-size: 28px;
      font-weight: 700;
      font-family: var(--mono, monospace);
      color: var(--text-bright, #fff);
    }

    .card-value.accent {
      color: var(--accent, #4880ff);
    }

    .card-value.warning {
      color: var(--warning, #fcbe2d);
    }

    .card-value.success {
      color: var(--success, #00b69b);
    }

    .card-value.error {
      color: var(--error, #fd5454);
    }

    .section-title {
      font-size: 16px;
      font-weight: 600;
      color: var(--text-bright, #fff);
      margin: 8px 0;
    }

    .table {
      width: 100%;
      border-collapse: collapse;
      font-size: 13px;
    }

    .table th {
      text-align: left;
      padding: 8px 12px;
      border-bottom: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-weight: 500;
      text-transform: uppercase;
      font-size: 11px;
      letter-spacing: 0.3px;
    }

    .table td {
      padding: 8px 12px;
      border-bottom: 1px solid var(--border, #1A3550);
      font-family: var(--mono, monospace);
    }

    .table tr:last-child td {
      border-bottom: none;
    }

    .bar-container {
      width: 100%;
      height: 8px;
      background: var(--bg-input, #1A2D42);
      border-radius: 4px;
      overflow: hidden;
      margin-top: 4px;
    }

    .bar-fill {
      height: 100%;
      border-radius: 4px;
      transition: width 0.3s ease;
    }

    .latency-row {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 8px;
    }

    .latency-label {
      font-size: 13px;
      min-width: 80px;
    }

    .latency-value {
      font-family: var(--mono, monospace);
      font-size: 13px;
      color: var(--text-bright, #fff);
      min-width: 60px;
      text-align: right;
    }

    .error-state {
      text-align: center;
      padding: 40px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .loading-state {
      text-align: center;
      padding: 40px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    @media (max-width: 639px) {
      :host { padding: 16px; }
      .grid { grid-template-columns: 1fr 1fr; }
      .card-value { font-size: 22px; }
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    this.loadMetrics();
    this.refreshTimer = window.setInterval(() => this.loadMetrics(), 10_000);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this.refreshTimer !== null) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }
  }

  private async loadMetrics() {
    try {
      const res = await this.fetchFn('/metrics');
      if (!res.ok) {
        this.error = `HTTP ${res.status}`;
        this.loading = false;
        return;
      }
      const text = await res.text();
      this.samples = parsePrometheusText(text);
      this.error = null;
      this.lastRefresh = new Date();
    } catch (e) {
      this.error = String(e);
    }
    this.loading = false;
  }

  /** Sum all samples with the given metric name. */
  private sumMetric(name: string): number {
    return this.samples
      .filter(s => s.name === name)
      .reduce((acc, s) => acc + s.value, 0);
  }

  /** Get metric value for a specific label. */
  private getMetric(name: string, labels: Record<string, string>): number {
    const match = this.samples.find(s => {
      if (s.name !== name) return false;
      for (const [k, v] of Object.entries(labels)) {
        if (s.labels[k] !== v) return false;
      }
      return true;
    });
    return match?.value ?? 0;
  }

  /** Get all label variants for a metric. */
  private getLabeled(name: string): MetricSample[] {
    return this.samples.filter(s => s.name === name);
  }

  /** Get budget info for display. */
  private getBudgetInfo() {
    const spent = this.sumMetric('dm_budget_spent_sats');
    // We use GET /budget for the remaining, but for now show total from metrics
    return { spent };
  }

  private formatNumber(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return n.toLocaleString();
  }

  render() {
    if (this.loading) {
      return html`<div class="loading-state">Loading metrics...</div>`;
    }

    if (this.error) {
      return html`
        <div class="page-title">Telemetry</div>
        <div class="error-state">
          Failed to load metrics: ${this.error}
        </div>
      `;
    }

    const totalTokens = this.sumMetric('dm_tokens_total');
    const activeTasks = this.getMetric('dm_active_tasks', {});
    const totalTasks = this.sumMetric('dm_tasks_total');
    const totalErrors = this.sumMetric('dm_errors_total');
    const budget = this.getBudgetInfo();

    // Latency buckets
    const latencyProviders = ['openai', 'claude'];
    const latencyData = latencyProviders.map(p => {
      const count = this.getMetric('dm_request_latency_seconds_count', { provider: p });
      const sum = this.getMetric('dm_request_latency_seconds_sum', { provider: p });
      const avgMs = count > 0 ? (sum / count) * 1000 : 0;
      return { provider: p, count, avgMs };
    }).filter(d => d.count > 0);

    // Tool calls breakdown
    const toolSamples = this.getLabeled('dm_tool_calls_total');
    const toolCalls = toolSamples
      .map(s => ({ name: s.labels['tool_name'] || 'unknown', count: s.value }))
      .sort((a, b) => b.count - a.count);
    const maxToolCount = toolCalls.length > 0 ? toolCalls[0].count : 1;

    // Task status breakdown
    const taskStatuses = this.getLabeled('dm_tasks_total');

    // Error breakdown
    const errorSamples = this.getLabeled('dm_errors_total');

    // Token breakdown by model
    const tokenSamples = this.getLabeled('dm_tokens_total');
    const tokenByModel = tokenSamples
      .map(s => ({ model: s.labels['model'] || 'unknown', tokens: s.value }))
      .sort((a, b) => b.tokens - a.tokens);

    return html`
      <div class="page-title">Telemetry</div>
      <div class="subtitle">
        Auto-refreshes every 10s
        ${this.lastRefresh ? html` | Last: ${this.lastRefresh.toLocaleTimeString()}` : ''}
      </div>

      <!-- Hero stats grid -->
      <div class="grid">
        <div class="card">
          <div class="card-title">Active Tasks</div>
          <div class="card-value accent">${activeTasks}</div>
        </div>
        <div class="card">
          <div class="card-title">Total Tasks</div>
          <div class="card-value">${this.formatNumber(totalTasks)}</div>
        </div>
        <div class="card">
          <div class="card-title">Total Tokens</div>
          <div class="card-value">${this.formatNumber(totalTokens)}</div>
        </div>
        <div class="card">
          <div class="card-title">Budget Spent (sats)</div>
          <div class="card-value warning">${this.formatNumber(budget.spent)}</div>
        </div>
        <div class="card">
          <div class="card-title">Total Errors</div>
          <div class="card-value ${totalErrors > 0 ? 'error' : 'success'}">${totalErrors}</div>
        </div>
      </div>

      <!-- Request Latency -->
      ${latencyData.length > 0 ? html`
        <div class="section-title">Request Latency</div>
        <div class="card">
          ${latencyData.map(d => html`
            <div class="latency-row">
              <span class="latency-label">${d.provider}</span>
              <div style="flex:1">
                <div class="bar-container">
                  <div class="bar-fill" style="width: ${Math.min(100, d.avgMs / 100)}%; background: var(--accent, #4880ff);"></div>
                </div>
              </div>
              <span class="latency-value">${d.avgMs.toFixed(0)}ms</span>
              <span class="latency-label" style="color: var(--text-dim)">${d.count} req</span>
            </div>
          `)}
        </div>
      ` : ''}

      <!-- Token Usage by Model -->
      ${tokenByModel.length > 0 ? html`
        <div class="section-title">Token Usage by Model</div>
        <div class="card">
          <table class="table">
            <thead>
              <tr>
                <th>Model</th>
                <th style="text-align: right">Tokens</th>
              </tr>
            </thead>
            <tbody>
              ${tokenByModel.map(t => html`
                <tr>
                  <td>${t.model}</td>
                  <td style="text-align: right">${this.formatNumber(t.tokens)}</td>
                </tr>
              `)}
            </tbody>
          </table>
        </div>
      ` : ''}

      <!-- Tool Calls -->
      ${toolCalls.length > 0 ? html`
        <div class="section-title">Tool Calls</div>
        <div class="card">
          <table class="table">
            <thead>
              <tr>
                <th>Tool</th>
                <th style="text-align: right">Calls</th>
                <th style="width: 40%"></th>
              </tr>
            </thead>
            <tbody>
              ${toolCalls.map(t => html`
                <tr>
                  <td>${t.name}</td>
                  <td style="text-align: right">${t.count}</td>
                  <td>
                    <div class="bar-container">
                      <div class="bar-fill" style="width: ${(t.count / maxToolCount) * 100}%; background: var(--success, #00b69b);"></div>
                    </div>
                  </td>
                </tr>
              `)}
            </tbody>
          </table>
        </div>
      ` : ''}

      <!-- Task Status Breakdown -->
      ${taskStatuses.length > 0 ? html`
        <div class="section-title">Task Status</div>
        <div class="grid">
          ${taskStatuses.map(s => {
            const status = s.labels['status'] || 'unknown';
            const colorClass = status === 'complete' ? 'success'
              : status === 'error' ? 'error'
              : status === 'cancelled' ? 'warning'
              : '';
            return html`
              <div class="card">
                <div class="card-title">${status}</div>
                <div class="card-value ${colorClass}">${s.value}</div>
              </div>
            `;
          })}
        </div>
      ` : ''}

      <!-- Error Breakdown -->
      ${errorSamples.length > 0 ? html`
        <div class="section-title">Errors</div>
        <div class="card">
          <table class="table">
            <thead>
              <tr>
                <th>Error Type</th>
                <th style="text-align: right">Count</th>
              </tr>
            </thead>
            <tbody>
              ${errorSamples.map(e => html`
                <tr>
                  <td>${e.labels['error_type'] || 'unknown'}</td>
                  <td style="text-align: right; color: var(--error, #fd5454)">${e.value}</td>
                </tr>
              `)}
            </tbody>
          </table>
        </div>
      ` : ''}
    `;
  }
}
