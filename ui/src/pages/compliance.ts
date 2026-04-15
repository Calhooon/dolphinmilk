/**
 * Compliance page -- shows WORM mode, compliance mode, retention policy,
 * regulations, summary stats, and a filterable task table.
 * Fetches data from GET /compliance/report with optional ?from=&to= date params.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { pageHost, centerState, pageTitle, stateFeedback, renderLoading, renderError, renderEmpty } from '../lib/shared-styles.js';
import { fetchBsvUsdRate, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import { FetchController } from '../controllers/fetch-controller.js';

interface ComplianceReport {
  compliance_mode: boolean;
  worm_mode: boolean;
  regulations: string[];
  retention_days: number;
  summary: {
    total_tasks: number;
    total_proofs: number;
    total_sats_spent: number;
  };
  tasks: Array<{
    task_id: string;
    iterations: number;
    sats_spent: number;
    proof_count: number;
    started_at?: string;
  }>;
}

@customElement('dm-compliance')
export class WormCompliance extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @state() private usdRate = 0;

  private ctrl = new FetchController<ComplianceReport>(this);

  @state() private filterFrom = '';
  @state() private filterTo = '';

  static styles = [
    pageHost, centerState, pageTitle, stateFeedback,
    css`
      /* Status cards grid */
      .status-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
        gap: 12px;
        margin-bottom: 24px;
      }

      .status-card {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px 20px;
      }

      .status-label {
        font-size: 11px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-bottom: 8px;
      }

      .status-value {
        font-size: 16px;
        font-weight: 700;
        font-family: var(--sans, sans-serif);
        color: var(--text-bright, #fff);
      }

      .badge-on {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 4px 12px;
        border-radius: 999px;
        font-size: 12px;
        font-weight: 700;
        font-family: var(--mono, monospace);
        background: rgba(0, 182, 155, 0.15);
        color: var(--success, #00b69b);
      }

      .badge-off {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 4px 12px;
        border-radius: 999px;
        font-size: 12px;
        font-weight: 700;
        font-family: var(--mono, monospace);
        background: rgba(255, 255, 255, 0.06);
        color: var(--text-dim, rgba(255,255,255,0.4));
      }

      .regulations-list {
        list-style: none;
        margin: 0;
        padding: 0;
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
      }

      .regulation-pill {
        display: inline-block;
        padding: 3px 10px;
        border-radius: 999px;
        font-size: 11px;
        font-weight: 600;
        font-family: var(--mono, monospace);
        background: rgba(20, 168, 196, 0.12);
        color: var(--accent, #14A8C4);
      }

      .none-text {
        font-size: 13px;
        color: var(--text-dim, rgba(255,255,255,0.4));
        font-style: italic;
      }

      /* Summary stats row */
      .summary-row {
        display: flex;
        gap: 16px;
        flex-wrap: wrap;
        margin-bottom: 24px;
      }

      .summary-stat {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        padding: 16px 24px;
        flex: 1;
        min-width: 140px;
        text-align: center;
      }

      .summary-stat-value {
        font-size: 28px;
        font-weight: 700;
        font-family: var(--sans, sans-serif);
        color: var(--text-bright, #fff);
        line-height: 1.2;
      }

      .summary-stat-value.sats {
        color: var(--warning, #fcbe2d);
      }

      .summary-stat-label {
        font-size: 11px;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-top: 4px;
      }

      /* Date filter */
      .filter-bar {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-bottom: 20px;
        flex-wrap: wrap;
      }

      .filter-label {
        font-size: 12px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      .filter-input {
        padding: 6px 10px;
        background: var(--bg-input, #1A2D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: 6px;
        color: var(--text, #c9d1d9);
        font-size: 13px;
        font-family: var(--mono, monospace);
        outline: none;
        transition: border-color 0.15s ease;
      }

      .filter-input:focus {
        border-color: var(--accent, #14A8C4);
      }

      /* Webkit date input calendar icon color fix for dark theme */
      .filter-input::-webkit-calendar-picker-indicator {
        filter: invert(0.7);
      }

      .filter-btn {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        padding: 6px 16px;
        border-radius: 6px;
        font-size: 13px;
        font-weight: 600;
        cursor: pointer;
        border: 1px solid var(--border, #1A3550);
        font-family: inherit;
        background: rgba(20, 168, 196, 0.12);
        color: var(--accent, #14A8C4);
        transition: background 0.15s ease;
      }

      .filter-btn:hover {
        background: rgba(20, 168, 196, 0.2);
      }

      /* Task table */
      .task-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 13px;
      }

      .task-table th {
        text-align: left;
        font-size: 11px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        padding: 10px 12px;
        border-bottom: 1px solid var(--border, #1A3550);
      }

      .task-table td {
        padding: 10px 12px;
        border-bottom: 1px solid var(--border, #1A3550);
        color: var(--text, #e0e0e8);
      }

      .task-table tr:last-child td {
        border-bottom: none;
      }

      .task-table tr:hover td {
        background: rgba(255, 255, 255, 0.02);
      }

      .task-link {
        color: var(--accent, #14A8C4);
        text-decoration: none;
        font-family: var(--mono, monospace);
        font-size: 12px;
        transition: color 0.15s ease;
      }

      .task-link:hover {
        color: var(--text-bright, #fff);
        text-decoration: underline;
      }

      .table-container {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        overflow: hidden;
      }

      .mono {
        font-family: var(--mono, monospace);
      }

      .sats-value {
        color: var(--warning, #fcbe2d);
        font-family: var(--mono, monospace);
        font-weight: 600;
      }

      .section-heading {
        font-size: 13px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin: 0 0 12px 0;
      }

      .empty-row {
        text-align: center;
        color: var(--text-dim, rgba(255,255,255,0.4));
        font-style: italic;
        padding: 24px 12px;
      }

      @media (max-width: 639px) {
        .status-grid {
          grid-template-columns: repeat(2, 1fr);
        }
        .summary-row {
          flex-direction: column;
        }
        .summary-stat-value {
          font-size: 20px;
        }
        .filter-bar {
          flex-direction: column;
          align-items: flex-start;
        }
      }
    `,
  ];

  firstUpdated() {
    fetchBsvUsdRate().then(r => { this.usdRate = r; });
    this.loadReport();
  }

  private async loadReport() {
    await this.ctrl.fetch(async () => {
      let url = '/compliance/report';
      const params: string[] = [];
      if (this.filterFrom) params.push(`from=${this.filterFrom}`);
      if (this.filterTo) params.push(`to=${this.filterTo}`);
      if (params.length > 0) url += `?${params.join('&')}`;

      const res = await this.fetchFn(url);
      if (!res.ok) throw new Error(`Compliance report: ${res.status}`);
      return res.json() as Promise<ComplianceReport>;
    });
  }

  private handleFilter() {
    this.loadReport();
  }

  private formatRetention(days: number): string {
    const years = Math.floor(days / 365);
    const remainder = days % 365;
    if (years > 0 && remainder === 0) {
      return `${days} days (${years} year${years !== 1 ? 's' : ''})`;
    }
    if (years > 0) {
      return `${days} days (~${years} year${years !== 1 ? 's' : ''})`;
    }
    return `${days} days`;
  }

  private renderStatusCards() {
    const d = this.ctrl.data!;

    return html`
      <div class="status-grid">
        <div class="status-card">
          <div class="status-label">WORM Mode</div>
          <div class="status-value">
            ${d.worm_mode
              ? html`<span class="badge-on">ON</span>`
              : html`<span class="badge-off">OFF</span>`}
          </div>
        </div>
        <div class="status-card">
          <div class="status-label">Compliance Mode</div>
          <div class="status-value">
            ${d.compliance_mode
              ? html`<span class="badge-on">ON</span>`
              : html`<span class="badge-off">OFF</span>`}
          </div>
        </div>
        <div class="status-card">
          <div class="status-label">Retention</div>
          <div class="status-value" style="font-size:14px">
            ${this.formatRetention(d.retention_days)}
          </div>
        </div>
        <div class="status-card">
          <div class="status-label">Regulations</div>
          <div class="status-value">
            ${d.regulations.length > 0
              ? html`
                  <ul class="regulations-list">
                    ${d.regulations.map(r => html`<li class="regulation-pill">${r}</li>`)}
                  </ul>
                `
              : html`<span class="none-text">None configured</span>`}
          </div>
        </div>
      </div>
    `;
  }

  private renderSummary() {
    const s = this.ctrl.data!.summary;

    return html`
      <div class="summary-row">
        <div class="summary-stat">
          <div class="summary-stat-value">${s.total_tasks.toLocaleString()}</div>
          <div class="summary-stat-label">Total Tasks</div>
        </div>
        <div class="summary-stat">
          <div class="summary-stat-value">${s.total_proofs.toLocaleString()}</div>
          <div class="summary-stat-label">Total Proofs</div>
        </div>
        <div class="summary-stat">
          <div class="summary-stat-value sats" title="${formatInlineCurrencyAlt(s.total_sats_spent, this.usdRate, this.currencyMode)}">${formatInlineCurrency(s.total_sats_spent, this.usdRate, this.currencyMode)}</div>
          <div class="summary-stat-label">Total Spent</div>
        </div>
      </div>
    `;
  }

  private renderDateFilter() {
    return html`
      <div class="filter-bar">
        <span class="filter-label">Filter by date</span>
        <input
          type="date"
          class="filter-input"
          .value=${this.filterFrom}
          @input=${(e: Event) => { this.filterFrom = (e.target as HTMLInputElement).value; }}
        />
        <span class="filter-label">to</span>
        <input
          type="date"
          class="filter-input"
          .value=${this.filterTo}
          @input=${(e: Event) => { this.filterTo = (e.target as HTMLInputElement).value; }}
        />
        <button class="filter-btn" @click=${this.handleFilter}>Filter</button>
      </div>
    `;
  }

  private renderTaskTable() {
    const tasks = this.ctrl.data!.tasks;

    return html`
      <h3 class="section-heading">Tasks</h3>
      <div class="table-container">
        <table class="task-table">
          <thead>
            <tr>
              <th>Task ID</th>
              <th>Started</th>
              <th>Iterations</th>
              <th>Sats Spent</th>
              <th>Proofs</th>
            </tr>
          </thead>
          <tbody>
            ${tasks.length === 0
              ? html`<tr><td colspan="5" class="empty-row">No tasks found</td></tr>`
              : tasks.map(t => html`
                  <tr>
                    <td>
                      <a class="task-link" href="#task/${t.task_id}">${t.task_id.length > 16 ? t.task_id.slice(0, 16) + '...' : t.task_id}</a>
                    </td>
                    <td class="mono" style="font-size:12px; color:var(--text-dim)">
                      ${t.started_at ? new Date(t.started_at).toLocaleDateString() : '-'}
                    </td>
                    <td class="mono">${t.iterations}</td>
                    <td class="sats-value" title="${formatInlineCurrencyAlt(t.sats_spent, this.usdRate, this.currencyMode)}">${formatInlineCurrency(t.sats_spent, this.usdRate, this.currencyMode)}</td>
                    <td class="mono">${t.proof_count}</td>
                  </tr>
                `)}
          </tbody>
        </table>
      </div>
    `;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) {
      return renderLoading('Loading compliance report...');
    }
    if (this.ctrl.hasError && !this.ctrl.hasData) {
      return renderError(this.ctrl.error ?? 'Failed to load compliance report', 'Check that the agent server is running.');
    }
    if (!this.ctrl.data) {
      return renderEmpty('No compliance data available', 'Submit a task to start generating compliance data.');
    }

    return html`
      <div class="page-title">Compliance</div>
      ${this.renderStatusCards()}
      ${this.renderSummary()}
      ${this.renderDateFilter()}
      ${this.renderTaskTable()}
    `;
  }
}
