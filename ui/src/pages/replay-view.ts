import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, formatSats, truncate } from '../lib/util.js';
import { fetchBsvUsdRate, formatInlineCurrency } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import { centerState, pageHost, pageTitle, cardBase, badges, stateFeedback, statsBar, buttonStyles, renderEmpty, renderLoading, renderError, renderBreadcrumb } from '../lib/shared-styles.js';

/** A single event in the replay timeline. */
interface ReplayEvent {
  index: number;
  timestamp: number;
  event_type: string;
  id: string;
  iteration: number;
  event_sats: number;
  cumulative_sats: number;
  tool_name?: string;
  model?: string;
  elapsed_secs: number;
  data: Record<string, unknown>;
}

/** Cost timeline point. */
interface CostPoint {
  elapsed_secs: number;
  cumulative_sats: number;
  event_type: string;
  index: number;
}

/** Tool usage entry. */
interface ToolUsage {
  name: string;
  iteration: number;
  call_index: number;
  result_index?: number;
  success?: boolean;
  sats_paid: number;
}

/** Full replay response from GET /task/{id}/replay. */
interface ReplayData {
  task_id: string;
  total_events: number;
  total_iterations: number;
  total_sats: number;
  duration_secs: number;
  events: ReplayEvent[];
  cost_timeline: CostPoint[];
  tool_usage: ToolUsage[];
}

@customElement('dm-replay-view')
export class WormReplayView extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @property() taskId = '';

  @state() private data: ReplayData | null = null;
  @state() private loading = true;
  @state() private error = '';
  @state() private selectedIndex: number | null = null;
  @state() private usdRate = 0;
  @state() private forkLoading = false;
  @state() private forkError = '';

  static styles = [pageHost, centerState, pageTitle, cardBase, badges, stateFeedback, statsBar, buttonStyles, css`
    .replay-header {
      display: flex;
      align-items: center;
      gap: 16px;
      margin-bottom: 16px;
      flex-wrap: wrap;
    }

    .summary-stats {
      display: flex;
      gap: 24px;
      flex-wrap: wrap;
      padding: 12px 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      margin-bottom: 16px;
    }

    .stat {
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .stat-label {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }

    .stat-value {
      font-size: 16px;
      font-family: var(--mono, monospace);
      color: var(--text-bright, #fff);
    }

    /* Cost graph */
    .cost-graph {
      position: relative;
      height: 120px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      margin-bottom: 16px;
      padding: 8px 12px;
      overflow: hidden;
    }

    .cost-graph-title {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-transform: uppercase;
      letter-spacing: 0.5px;
      margin-bottom: 4px;
    }

    .cost-svg {
      width: 100%;
      height: 90px;
    }

    /* Timeline */
    .timeline {
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .timeline-event {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid transparent;
      border-radius: 6px;
      cursor: pointer;
      transition: background 0.15s, border-color 0.15s;
      font-size: 12px;
    }

    .timeline-event:hover {
      background: rgba(255,255,255,0.03);
      border-color: var(--border, #1A3550);
    }

    .timeline-event.selected {
      border-color: var(--accent, #14A8C4);
      background: rgba(20, 168, 196, 0.08);
    }

    .event-index {
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      min-width: 32px;
      text-align: right;
    }

    .event-type-badge {
      display: inline-block;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 10px;
      font-family: var(--mono, monospace);
      text-transform: uppercase;
      min-width: 80px;
      text-align: center;
    }

    .event-type-badge.think_request,
    .event-type-badge.think_response { background: rgba(72, 128, 255, 0.15); color: var(--accent, #6ea8ff); }
    .event-type-badge.tool_call,
    .event-type-badge.tool_result { background: rgba(0, 182, 155, 0.15); color: var(--success, #00b69b); }
    .event-type-badge.user,
    .event-type-badge.system { background: rgba(255, 255, 255, 0.08); color: var(--text-dim); }
    .event-type-badge.error { background: rgba(253, 84, 84, 0.15); color: var(--error, #fd5454); }
    .event-type-badge.budget_check { background: rgba(252, 190, 45, 0.15); color: var(--warning, #fcbe2d); }
    .event-type-badge.proof_created,
    .event-type-badge.checkpoint_created { background: rgba(0, 182, 155, 0.15); color: var(--success, #00b69b); }

    .event-detail {
      flex: 1;
      color: var(--text, #e0e0e8);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .event-cost {
      font-family: var(--mono, monospace);
      color: var(--warning, #fcbe2d);
      min-width: 60px;
      text-align: right;
    }

    .event-elapsed {
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      min-width: 50px;
      text-align: right;
    }

    /* Fork panel */
    .fork-panel {
      margin-top: 16px;
      padding: 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--accent, #14A8C4);
      border-radius: var(--radius-md, 14px);
    }

    .fork-panel h3 {
      margin: 0 0 8px;
      font-size: 14px;
      color: var(--text-bright, #fff);
    }

    .fork-panel p {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin: 0 0 12px;
    }

    .fork-actions {
      display: flex;
      gap: 8px;
      align-items: center;
    }

    .fork-error {
      color: var(--error, #fd5454);
      font-size: 12px;
      margin-top: 8px;
    }

    /* Section titles */
    .section-title {
      font-size: 13px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-transform: uppercase;
      letter-spacing: 0.5px;
      margin: 16px 0 8px;
    }

    /* Tool usage table */
    .tool-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 12px;
      margin-bottom: 16px;
    }

    .tool-table th {
      text-align: left;
      padding: 6px 8px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-weight: normal;
      font-size: 11px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      border-bottom: 1px solid var(--border, #1A3550);
    }

    .tool-table td {
      padding: 6px 8px;
      color: var(--text, #e0e0e8);
      border-bottom: 1px solid rgba(255,255,255,0.03);
    }

    .tool-table .tool-name {
      font-family: var(--mono, monospace);
      color: var(--accent, #14A8C4);
    }

    .tool-success { color: var(--success, #00b69b); }
    .tool-failed { color: var(--error, #fd5454); }

    @media (max-width: 639px) {
      .summary-stats { flex-direction: column; gap: 8px; }
      .timeline-event { flex-wrap: wrap; }
      .event-detail { min-width: 100%; order: 4; }
    }
  `];

  connectedCallback() {
    super.connectedCallback();
    this._load();
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('taskId') && this.taskId) {
      this._load();
    }
  }

  private async _load() {
    if (!this.taskId) return;
    this.loading = true;
    this.error = '';
    try {
      const [replayRes, rate] = await Promise.all([
        this.fetchFn(`/task/${this.taskId}/replay`),
        fetchBsvUsdRate(),
      ]);
      if (!replayRes.ok) {
        this.error = replayRes.status === 404
          ? 'Task not found or transcript missing.'
          : `Failed to load replay data (${replayRes.status}).`;
        return;
      }
      this.data = await replayRes.json();
      this.usdRate = rate;
    } catch (e) {
      this.error = `Network error: ${e}`;
    } finally {
      this.loading = false;
    }
  }

  private _selectEvent(index: number) {
    this.selectedIndex = this.selectedIndex === index ? null : index;
  }

  private async _fork() {
    if (this.selectedIndex === null || !this.data) return;
    this.forkLoading = true;
    this.forkError = '';
    try {
      const resp = await this.fetchFn(`/task/${this.taskId}/fork`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ event_index: this.selectedIndex }),
      });
      if (!resp.ok) {
        const body = await resp.json().catch(() => ({ error: 'Unknown error' }));
        this.forkError = body.error || `Fork failed (${resp.status})`;
        return;
      }
      const result = await resp.json();
      // Navigate to the new task
      window.location.hash = `#task/${result.task_id}`;
    } catch (e) {
      this.forkError = `Network error: ${e}`;
    } finally {
      this.forkLoading = false;
    }
  }

  private _formatElapsed(secs: number): string {
    if (secs < 1) return '< 1s';
    if (secs < 60) return `${Math.round(secs)}s`;
    const m = Math.floor(secs / 60);
    const s = Math.round(secs % 60);
    return `${m}m ${s}s`;
  }

  private _eventDetail(ev: ReplayEvent): string {
    switch (ev.event_type) {
      case 'think_request': return `Model: ${ev.model || 'unknown'}`;
      case 'think_response': {
        const text = (ev.data?.content as string) || '';
        return truncate(text, 80);
      }
      case 'tool_call': return ev.tool_name || 'unknown tool';
      case 'tool_result': return `${ev.tool_name || 'tool'}: ${(ev.data?.success as boolean) ? 'ok' : 'error'}`;
      case 'user': return truncate((ev.data?.content as string) || '', 80);
      case 'system': return truncate((ev.data?.content as string) || '', 80);
      case 'proof_created': return `Proof: ${(ev.data?.proof_type as string) || 'unknown'}`;
      case 'budget_check': return `Balance: ${formatSats((ev.data?.balance as number) || 0)}`;
      case 'error': return (ev.data?.error as string) || 'Error';
      default: return ev.event_type;
    }
  }

  render() {
    if (this.loading) return renderLoading('Loading replay data...');
    if (this.error) return renderError(this.error);
    if (!this.data) return renderEmpty('No replay data', 'Task may not have a transcript.');

    const d = this.data;
    const breadcrumb = [
      { label: 'Tasks', hash: '#tasks' },
      { label: `Task ${d.task_id.slice(0, 8)}...`, hash: `#task/${d.task_id}` },
      { label: 'Replay' },
    ];

    return html`
      ${renderBreadcrumb(breadcrumb)}
      <h1 class="page-title">Task Replay</h1>

      <div class="summary-stats">
        <div class="stat">
          <span class="stat-label">Events</span>
          <span class="stat-value">${d.total_events}</span>
        </div>
        <div class="stat">
          <span class="stat-label">Iterations</span>
          <span class="stat-value">${d.total_iterations}</span>
        </div>
        <div class="stat">
          <span class="stat-label">Total Cost</span>
          <span class="stat-value">${formatInlineCurrency(d.total_sats, this.usdRate, this.currencyMode)}</span>
        </div>
        <div class="stat">
          <span class="stat-label">Duration</span>
          <span class="stat-value">${this._formatElapsed(d.duration_secs)}</span>
        </div>
      </div>

      ${this._renderCostGraph()}

      ${d.tool_usage.length > 0 ? html`
        <div class="section-title">Tool Usage</div>
        <table class="tool-table">
          <thead>
            <tr>
              <th>Tool</th>
              <th>Iter</th>
              <th>Status</th>
              <th>Cost</th>
            </tr>
          </thead>
          <tbody>
            ${d.tool_usage.map(t => html`
              <tr>
                <td class="tool-name">${t.name}</td>
                <td>${t.iteration}</td>
                <td class="${t.success === true ? 'tool-success' : t.success === false ? 'tool-failed' : ''}">${
                  t.success === true ? 'OK' : t.success === false ? 'Failed' : 'Pending'
                }</td>
                <td>${t.sats_paid > 0 ? formatSats(t.sats_paid) + ' sats' : '-'}</td>
              </tr>
            `)}
          </tbody>
        </table>
      ` : nothing}

      <div class="section-title">Event Timeline (click to fork)</div>
      <div class="timeline">
        ${d.events.map(ev => html`
          <div class="timeline-event ${this.selectedIndex === ev.index ? 'selected' : ''}"
               @click=${() => this._selectEvent(ev.index)}>
            <span class="event-index">${ev.index}</span>
            <span class="event-type-badge ${ev.event_type}">${ev.event_type.replace(/_/g, ' ')}</span>
            <span class="event-detail">${this._eventDetail(ev)}</span>
            ${ev.event_sats > 0 ? html`
              <span class="event-cost">${formatSats(ev.event_sats)} sats</span>
            ` : nothing}
            <span class="event-elapsed">${this._formatElapsed(ev.elapsed_secs)}</span>
          </div>
        `)}
      </div>

      ${this.selectedIndex !== null ? this._renderForkPanel() : nothing}
    `;
  }

  private _renderCostGraph() {
    const d = this.data;
    if (!d || d.cost_timeline.length === 0) return nothing;

    const maxSats = d.total_sats || 1;
    const maxTime = d.duration_secs || 1;
    const width = 600;
    const height = 80;
    const padding = 4;

    const points = d.cost_timeline.map(p => ({
      x: padding + (p.elapsed_secs / maxTime) * (width - 2 * padding),
      y: height - padding - (p.cumulative_sats / maxSats) * (height - 2 * padding),
    }));

    // Build SVG path
    let pathD = '';
    if (points.length > 0) {
      pathD = `M ${padding} ${height - padding}`;
      for (const p of points) {
        pathD += ` L ${p.x} ${p.y}`;
      }
    }

    // Fill area
    const fillD = pathD + ` L ${points[points.length - 1]?.x || padding} ${height - padding} Z`;

    return html`
      <div class="cost-graph">
        <div class="cost-graph-title">Cumulative Spending</div>
        <svg class="cost-svg" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none">
          <defs>
            <linearGradient id="cost-fill" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stop-color="var(--warning, #fcbe2d)" stop-opacity="0.3" />
              <stop offset="100%" stop-color="var(--warning, #fcbe2d)" stop-opacity="0.02" />
            </linearGradient>
          </defs>
          ${pathD ? html`
            <path d="${fillD}" fill="url(#cost-fill)" />
            <path d="${pathD}" fill="none" stroke="var(--warning, #fcbe2d)" stroke-width="2" />
          ` : nothing}
          ${points.map((p, i) => html`
            <circle cx="${p.x}" cy="${p.y}" r="3" fill="var(--warning, #fcbe2d)" opacity="0.8" />
          `)}
        </svg>
      </div>
    `;
  }

  private _renderForkPanel() {
    const ev = this.data?.events[this.selectedIndex!];
    if (!ev) return nothing;

    return html`
      <div class="fork-panel">
        <h3>Fork from event #${ev.index}</h3>
        <p>
          Create a new task starting from this point in the conversation.
          The forked run will have ${ev.index} prior events as context
          (iteration ${ev.iteration}, ${formatSats(ev.cumulative_sats)} sats spent so far).
        </p>
        <div class="fork-actions">
          <button class="btn btn-primary" @click=${this._fork} ?disabled=${this.forkLoading}>
            ${this.forkLoading ? 'Forking...' : 'Fork Execution'}
          </button>
          <button class="btn" @click=${() => { this.selectedIndex = null; }}>Cancel</button>
        </div>
        ${this.forkError ? html`<div class="fork-error">${this.forkError}</div>` : nothing}
      </div>
    `;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'dm-replay-view': WormReplayView;
  }
}
