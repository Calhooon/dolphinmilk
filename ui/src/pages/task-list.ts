import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, formatTokens, parseTimestamp, truncate } from '../lib/util.js';
import { fetchBsvUsdRate, formatUsd, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import { type TaskSummary, getStatusClass } from '../lib/shared-types.js';
import { centerState, statsBar, cardBase, badges, pageHost, pageTitle, stateFeedback, renderEmpty, renderSmartEmpty, renderLoading, renderError } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';

@customElement('dm-task-list')
export class WormTaskList extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @state() private page = 0;
  @state() private usdRate = 0;
  @state() private originFilter: string = 'all';
  private pageSize = 25;

  private ctrl = new FetchController<TaskSummary[]>(this);

  static styles = [pageHost, centerState, pageTitle, statsBar, cardBase, badges, stateFeedback, css`
    /* Task cards */
    .task-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .task-card {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 16px 20px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      cursor: pointer;
      transition: background 0.15s ease, border-color 0.15s ease;
      text-decoration: none;
      color: inherit;
      min-height: 44px;
    }

    .task-card:hover {
      background: rgba(255, 255, 255, 0.03);
      border-color: var(--accent-dim, #0E8FA8);
    }

    .task-card:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    .task-text {
      flex: 1;
      min-width: 0;
      font-size: 13px;
      color: var(--text, #e0e0e8);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .task-meta {
      display: flex;
      align-items: center;
      gap: 12px;
      flex-shrink: 0;
    }

    .meta-item {
      font-size: 12px;
    }

    .proof-count {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--success, #00b69b);
      white-space: nowrap;
    }

    /* Responsive */
    @media (max-width: 639px) {
      .task-card {
        flex-direction: column;
        align-items: flex-start;
        gap: 8px;
        padding: 12px;
      }

      .task-text {
        white-space: normal;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
      }

      .task-meta {
        width: 100%;
        flex-wrap: wrap;
        gap: 8px;
      }
    }

    @media (min-width: 640px) and (max-width: 1023px) {
      .task-card {
        flex-wrap: wrap;
      }
    }

    /* Filter bar */
    .filter-bar {
      display: flex;
      gap: 8px;
      margin-bottom: 16px;
      flex-wrap: wrap;
    }

    .filter-chip {
      padding: 6px 14px;
      border-radius: 999px;
      border: 1px solid var(--border, #1A3550);
      background: transparent;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 13px;
      cursor: pointer;
      transition: all 0.15s ease;
    }

    .filter-chip:hover {
      color: var(--text, rgba(255,255,255,0.8));
      border-color: var(--text-dim);
    }

    .filter-chip.active {
      background: var(--accent, #4880ff);
      color: var(--text-bright, #fff);
      border-color: var(--accent, #4880ff);
    }

    /* Origin badge */
    .origin-badge {
      display: inline-flex;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 11px;
      background: rgba(255,255,255,0.06);
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    /* Pagination */
    .pagination {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 12px;
      padding: 16px 0;
      font-size: 13px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .pagination button {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text, #e0e0e8);
      padding: 6px 14px;
      border-radius: 6px;
      cursor: pointer;
      font-size: 12px;
      font-family: var(--mono, monospace);
      transition: background 0.15s, border-color 0.15s;
    }

    .pagination button:hover:not(:disabled) {
      background: rgba(20, 168, 196, 0.08);
      border-color: var(--accent, #14A8C4);
    }

    .pagination button:disabled {
      opacity: 0.3;
      cursor: not-allowed;
    }

    .page-info {
      font-family: var(--mono, monospace);
      font-size: 12px;
    }
  `];

  firstUpdated() {
    this.ctrl.fetch(async () => {
      const res = await this.fetchFn('/tasks');
      if (!res.ok) throw new Error(`Failed to load tasks: ${res.status}`);
      const data = await res.json();
      this.page = 0;
      this.usdRate = await fetchBsvUsdRate();
      return data.tasks ?? [];
    });
  }

  private getStatusBadge(status: string) {
    const cls = getStatusClass(status);
    const label = cls === 'pending' ? status : cls;
    return html`<span class="badge badge-${cls}">${label}</span>`;
  }

  private get filteredTasks(): TaskSummary[] {
    const tasks = this.ctrl.data ?? [];
    if (this.originFilter === 'all') return tasks;
    if (this.originFilter === 'system') {
      return tasks.filter(t => ['continuation', 'reflection', 'checklist'].includes(t.origin || ''));
    }
    return tasks.filter(t => (t.origin || 'chat') === this.originFilter);
  }

  private renderOriginBadge(origin?: string) {
    const labels: Record<string, string> = {
      chat: 'Chat',
      task: 'API',
      message: 'External',
      fork: 'Fork',
      schedule: 'Scheduled',
      continuation: 'Continuation',
      reflection: 'Reflection',
      checklist: 'Checklist',
    };
    const label = labels[origin || 'chat'] || origin || 'Chat';
    return html`<span class="origin-badge">${label}</span>`;
  }

  render() {
    if (this.ctrl.loading) {
      return renderLoading('Loading activity...');
    }

    if (this.ctrl.hasError) {
      return renderError(this.ctrl.error ?? 'Failed to load activity', 'Check that the agent server is running.');
    }

    const allTasks = this.ctrl.data ?? [];

    if (allTasks.length === 0) {
      return renderSmartEmpty({
        icon: '\u{1F4CB}',
        title: 'Your agent hasn\'t done anything yet',
        description: 'Activity shows everything your agent works on \u2014 conversations, automations, and background tasks. Start a conversation to see your first task appear here.',
        action: { label: 'Start a Conversation \u2192', href: '#' },
      });
    }

    const tasks = this.filteredTasks;
    const totalSats = tasks.reduce((sum, t) => sum + (t.sats_spent ?? 0), 0);
    const totalTokens = tasks.reduce((sum, t) => sum + (t.tokens ?? 0), 0);
    const activeCount = tasks.filter(
      (t) => t.status === 'running' || t.status === 'in_progress'
    ).length;

    // Sort by started_at descending (most recent first)
    const sorted = [...tasks].sort((a, b) => {
      const ta = parseTimestamp(a.started_at);
      const tb = parseTimestamp(b.started_at);
      return tb - ta;
    });

    // Pagination
    const totalPages = Math.max(1, Math.ceil(sorted.length / this.pageSize));
    const safePage = Math.min(this.page, totalPages - 1);
    const paginatedTasks = sorted.slice(
      safePage * this.pageSize,
      (safePage + 1) * this.pageSize
    );

    const isFiltered = this.originFilter !== 'all';

    return html`
      <div class="page-title">Activity</div>
      <div class="stats-bar">
        <div class="stat">
          <span class="stat-value">${tasks.length.toLocaleString()}</span>
          <span class="stat-label">${isFiltered ? `of ${allTasks.length.toLocaleString()}` : 'total'}</span>
        </div>
        <div class="stat">
          <span class="stat-value sats">${formatInlineCurrency(totalSats, this.usdRate, this.currencyMode)}</span>
          <span class="stat-label">spent${formatInlineCurrencyAlt(totalSats, this.usdRate, this.currencyMode) !== formatInlineCurrency(totalSats, this.usdRate, this.currencyMode) ? ` (${formatInlineCurrencyAlt(totalSats, this.usdRate, this.currencyMode)})` : ''}</span>
        </div>
        ${totalTokens > 0
          ? html`
              <div class="stat">
                <span class="stat-value">${formatTokens(totalTokens)}</span>
                <span class="stat-label">tokens</span>
              </div>
            `
          : nothing}
        ${activeCount > 0
          ? html`
              <div class="stat">
                <span class="stat-value">${activeCount}</span>
                <span class="stat-label">active</span>
              </div>
            `
          : nothing}
      </div>
      <div class="filter-bar">
        <button class="filter-chip ${this.originFilter === 'all' ? 'active' : ''}"
                @click=${() => { this.originFilter = 'all'; this.page = 0; }}>All</button>
        <button class="filter-chip ${this.originFilter === 'chat' ? 'active' : ''}"
                @click=${() => { this.originFilter = 'chat'; this.page = 0; }}>Conversations</button>
        <button class="filter-chip ${this.originFilter === 'schedule' ? 'active' : ''}"
                @click=${() => { this.originFilter = 'schedule'; this.page = 0; }}>Automations</button>
        <button class="filter-chip ${this.originFilter === 'message' ? 'active' : ''}"
                @click=${() => { this.originFilter = 'message'; this.page = 0; }}>External</button>
        <button class="filter-chip ${this.originFilter === 'system' ? 'active' : ''}"
                @click=${() => { this.originFilter = 'system'; this.page = 0; }}>System</button>
      </div>
      ${tasks.length === 0 && isFiltered
        ? renderEmpty('No matching activity', 'Try a different filter.')
        : html`
          <div class="task-list">
            ${paginatedTasks.map(
              (t) => html`
                <a class="task-card" href="#task/${t.id}">
                  ${this.getStatusBadge(t.status)}
                  ${this.renderOriginBadge(t.origin)}
                  <span class="task-text" title=${t.task}>
                    ${truncate(t.task, 80)}
                  </span>
                  <div class="task-meta">
                    <span class="meta-item">${t.iterations ?? 0} iter</span>
                    ${(t.tokens ?? 0) > 0
                      ? html`<span class="meta-item">${formatTokens(t.tokens!)} tok</span>`
                      : nothing}
                    <span class="meta-item sats" title="${formatInlineCurrencyAlt(t.sats_spent ?? 0, this.usdRate, this.currencyMode)}">${formatInlineCurrency(t.sats_spent ?? 0, this.usdRate, this.currencyMode)}</span>
                    ${(t.proof_txids?.length ?? 0) > 0
                      ? html`<span class="proof-count">${t.proof_txids!.length} proofs</span>`
                      : nothing}
                    <span class="time" title=${t.started_at}>
                      ${formatRelativeTime(parseTimestamp(t.started_at))}
                    </span>
                  </div>
                </a>
              `
            )}
          </div>
          ${totalPages > 1
            ? html`
                <div class="pagination">
                  <button
                    ?disabled=${safePage === 0}
                    @click=${() => { this.page = safePage - 1; }}
                  >&lt; Prev</button>
                  <span class="page-info">Page ${safePage + 1} of ${totalPages}</span>
                  <button
                    ?disabled=${safePage >= totalPages - 1}
                    @click=${() => { this.page = safePage + 1; }}
                  >Next &gt;</button>
                </div>
              `
            : nothing}
        `}
    `;
  }
}
