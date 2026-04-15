import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, parseTimestamp } from '../lib/util.js';
import type { Schedule, ScheduleRun } from '../lib/shared-types.js';
import { pageHost, centerState, statsBar, pageTitle, stateFeedback, renderSmartEmpty, renderLoading, renderError } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';

@customElement('dm-automations')
export class WormAutomations extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);

  @state() private expandedId: string | null = null;

  private ctrl = new FetchController<Schedule[]>(this);

  static styles = [pageHost, centerState, statsBar, pageTitle, stateFeedback, css`
    .page-header {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 16px;
    }

    .page-title {
      flex: 1;
    }

    .stat-value.success {
      color: var(--success, #00b69b);
    }

    /* Schedule cards */
    .schedule-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .schedule-card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px;
      padding: 16px;
      transition: border-color 0.15s ease;
      cursor: pointer;
    }

    .schedule-card:hover {
      border-color: rgba(255, 255, 255, 0.08);
    }

    .schedule-card.expanded {
      border-color: var(--accent, #4880ff);
    }

    .schedule-header {
      display: flex;
      align-items: center;
      gap: 10px;
      margin-bottom: 10px;
      flex-wrap: wrap;
    }

    .schedule-desc {
      flex: 1;
      min-width: 0;
      font-size: 14px;
      color: var(--text, #e0e0e8);
      font-family: var(--sans, sans-serif);
    }

    .type-badge {
      display: inline-flex;
      align-items: center;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
      font-weight: 600;
      font-family: var(--mono, monospace);
      letter-spacing: 0.3px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: rgba(255, 255, 255, 0.06);
    }

    .status-badge {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
      font-weight: 600;
      font-family: var(--mono, monospace);
      text-transform: uppercase;
      letter-spacing: 0.3px;
    }

    .status-badge.enabled {
      color: var(--success, #00b69b);
      background: rgba(0, 182, 155, 0.1);
    }

    .status-badge.disabled {
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: rgba(255, 255, 255, 0.06);
    }

    .schedule-details {
      display: flex;
      gap: 16px;
      flex-wrap: wrap;
    }

    .detail-item {
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .detail-label {
      font-size: 10px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .detail-value {
      font-size: 12px;
      font-family: var(--mono, monospace);
      color: var(--text, #e0e0e8);
    }

    .detail-value.accent {
      color: var(--accent, #14A8C4);
    }

    .detail-value.success {
      color: var(--success, #00b69b);
    }

    .detail-value.dim {
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .conv-link {
      font-size: 12px;
      font-family: var(--mono, monospace);
      color: var(--accent, #14A8C4);
      text-decoration: none;
    }

    .conv-link:hover {
      text-decoration: underline;
    }

    /* Run history */
    .run-history {
      margin-top: 12px;
      padding-top: 12px;
      border-top: 1px solid var(--border, #1A3550);
    }

    .run-history-header {
      font-size: 12px;
      font-weight: 600;
      color: var(--text-dim);
      text-transform: uppercase;
      letter-spacing: 0.5px;
      margin-bottom: 8px;
    }

    .run-entry {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 6px 0;
      font-size: 13px;
      border-bottom: 1px solid rgba(255,255,255,0.04);
    }

    .run-time {
      color: var(--text-dim);
      font-family: var(--mono, monospace);
      font-size: 12px;
      min-width: 140px;
    }

    .run-status {
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
    }

    .run-status.status-triggered {
      background: rgba(72, 128, 255, 0.12);
      color: var(--accent);
    }

    .run-status.status-completed {
      background: rgba(0, 182, 155, 0.12);
      color: var(--success);
    }

    .run-status.status-failed {
      background: rgba(253, 84, 84, 0.12);
      color: var(--error);
    }

    .run-cost {
      font-family: var(--mono, monospace);
      font-size: 12px;
      color: var(--text-dim);
    }

    .run-task-link {
      color: var(--accent);
      text-decoration: none;
      font-size: 12px;
    }

    .run-task-link:hover {
      text-decoration: underline;
    }

    .run-history-empty {
      font-size: 13px;
      color: var(--text-dim);
      padding: 8px 0;
    }

    /* Responsive */
    @media (max-width: 639px) {
      .schedule-card {
        padding: 12px;
      }

      .schedule-header {
        flex-direction: column;
        align-items: flex-start;
        gap: 6px;
      }

      .schedule-details {
        gap: 12px;
      }

      .run-entry {
        flex-wrap: wrap;
        gap: 6px;
      }

      .run-time {
        min-width: auto;
      }
    }
  `];

  firstUpdated() {
    this.loadSchedules();
    this.ctrl.startPolling(30000);
  }

  private async loadSchedules() {
    await this.ctrl.fetch(async () => {
      const res = await this.fetchFn('/schedules');
      if (res.status === 404) {
        // Endpoint not implemented yet -- show empty state
        return [];
      }
      if (!res.ok) {
        throw new Error(`Failed to load automations: ${res.status}`);
      }
      const data = await res.json();
      return Array.isArray(data) ? data : (data.schedules ?? []);
    });
  }

  private toggleExpand(id: string) {
    this.expandedId = this.expandedId === id ? null : id;
  }

  private formatInterval(secs: number): string {
    if (secs < 60) return `every ${secs}s`;
    if (secs < 3600) {
      const m = Math.floor(secs / 60);
      return `every ${m}m`;
    }
    if (secs < 86400) {
      const h = Math.floor(secs / 3600);
      const m = Math.floor((secs % 3600) / 60);
      return m > 0 ? `every ${h}h ${m}m` : `every ${h}h`;
    }
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    return h > 0 ? `every ${d}d ${h}h` : `every ${d}d`;
  }

  private formatFutureTime(ts: string): string {
    const now = Date.now() / 1000;
    const target = new Date(ts).getTime() / 1000;
    const diff = target - now;
    if (diff <= 0) return 'now';
    if (diff < 60) return `in ${Math.round(diff)}s`;
    if (diff < 3600) return `in ${Math.floor(diff / 60)}m`;
    if (diff < 86400) {
      const h = Math.floor(diff / 3600);
      const m = Math.floor((diff % 3600) / 60);
      return m > 0 ? `in ${h}h ${m}m` : `in ${h}h`;
    }
    const d = Math.floor(diff / 86400);
    return `in ${d}d`;
  }

  private formatRunTime(ts: string): string {
    try {
      const date = new Date(ts);
      return date.toLocaleString(undefined, {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      });
    } catch {
      return ts;
    }
  }

  private getScheduleTypeLabel(s: Schedule): string {
    if (s.schedule_type) {
      switch (s.schedule_type) {
        case 'cron': return 'Cron';
        case 'once': return 'One-shot';
        case 'interval': return 'Interval';
        default: return s.schedule_type;
      }
    }
    if (s.one_shot) return 'One-shot';
    if (s.cron_expression) return 'Cron';
    return 'Interval';
  }

  private renderRunHistory(runs: ScheduleRun[]) {
    if (!runs || runs.length === 0) {
      return html`<div class="run-history-empty">No runs yet</div>`;
    }
    return html`
      <div class="run-history">
        <div class="run-history-header">Recent Runs</div>
        ${runs.slice().reverse().map(run => html`
          <div class="run-entry">
            <span class="run-time">${this.formatRunTime(run.started_at)}</span>
            <span class="run-status status-${run.status}">${run.status}</span>
            ${run.sats_spent ? html`<span class="run-cost">${run.sats_spent.toLocaleString()} sats</span>` : ''}
            ${run.task_id ? html`<a class="run-task-link" href="#task/${run.task_id}">View task</a>` : ''}
          </div>
        `)}
      </div>
    `;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) {
      return renderLoading('Loading automations...');
    }

    if (this.ctrl.hasError && !this.ctrl.hasData) {
      return renderError(this.ctrl.error ?? 'Failed to load automations', 'Check that the agent server is running.');
    }

    const schedules = this.ctrl.data ?? [];

    if (schedules.length === 0) {
      return renderSmartEmpty({
        icon: '\u23F0',
        title: 'No automations set up yet',
        description: 'Automations run on a schedule \u2014 your agent works even when you\'re away. Tell your agent what to do and when.',
        prompts: [
          { text: '\u{1F4AC} "Search Twitter for BSV news every morning"', href: '#' },
          { text: '\u{1F4AC} "Check my wallet balance every hour"', href: '#' },
        ],
        action: { label: 'Go to Chat \u2192', href: '#' },
      });
    }

    const enabledCount = schedules.filter((s) => s.enabled).length;
    const totalRuns = schedules.reduce((sum, s) => sum + s.run_count, 0);

    return html`
      <div class="page-header">
        <div class="page-title">Automations</div>
      </div>
      <div class="stats-bar">
        <div class="stat">
          <span class="stat-value">${schedules.length.toLocaleString()}</span>
          <span class="stat-label">automations</span>
        </div>
        <div class="stat">
          <span class="stat-value success">${enabledCount.toLocaleString()}</span>
          <span class="stat-label">enabled</span>
        </div>
        <div class="stat">
          <span class="stat-value">${totalRuns.toLocaleString()}</span>
          <span class="stat-label">total runs</span>
        </div>
      </div>
      <div class="schedule-list">
        ${schedules.map(
          (s) => html`
            <div class="schedule-card ${this.expandedId === s.id ? 'expanded' : ''}"
                 @click=${() => this.toggleExpand(s.id)}>
              <div class="schedule-header">
                <span class="schedule-desc">${s.description}</span>
                <span class="type-badge">${this.getScheduleTypeLabel(s)}</span>
                <span class="status-badge ${s.enabled ? 'enabled' : 'disabled'}">
                  ${s.enabled ? 'enabled' : 'disabled'}
                </span>
              </div>
              <div class="schedule-details">
                ${s.cron_expression
                  ? html`
                      <div class="detail-item">
                        <span class="detail-label">Cron</span>
                        <span class="detail-value">${s.cron_expression}</span>
                      </div>
                    `
                  : html`
                      <div class="detail-item">
                        <span class="detail-label">Interval</span>
                        <span class="detail-value">${this.formatInterval(s.interval_secs)}</span>
                      </div>
                    `}
                ${s.next_run
                  ? html`
                      <div class="detail-item">
                        <span class="detail-label">Next Run</span>
                        <span class="detail-value accent">${this.formatFutureTime(s.next_run)}</span>
                      </div>
                    `
                  : nothing}
                ${s.last_run
                  ? html`
                      <div class="detail-item">
                        <span class="detail-label">Last Run</span>
                        <span class="detail-value dim">
                          ${formatRelativeTime(parseTimestamp(s.last_run))}
                        </span>
                      </div>
                    `
                  : nothing}
                <div class="detail-item">
                  <span class="detail-label">Runs</span>
                  <span class="detail-value">${s.run_count.toLocaleString()}</span>
                </div>
                ${s.conversation_id
                  ? html`
                      <div class="detail-item">
                        <span class="detail-label">Conversation</span>
                        <a class="conv-link" href="#conversation/${s.conversation_id}"
                           @click=${(e: Event) => e.stopPropagation()}>
                          ${s.conversation_id.substring(0, 8)}...
                        </a>
                      </div>
                    `
                  : nothing}
              </div>
              ${this.expandedId === s.id
                ? this.renderRunHistory(s.run_history ?? [])
                : nothing}
            </div>
          `
        )}
      </div>
    `;
  }
}
