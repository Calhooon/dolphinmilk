/**
 * Cross-task unified audit timeline.
 * Aggregates events from all recent tasks into a filterable feed.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { pageHost, centerState, pageTitle, sectionTitle, stateFeedback, renderLoading, renderError } from '../../lib/shared-styles.js';
import { formatSats, parseTimestamp, truncate } from '../../lib/util.js';
import { fetchBsvUsdRate, formatUsd, formatInlineCurrency, formatInlineCurrencyAlt, formatDualCurrency } from '../../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../../lib/storage.js';
import { FetchController } from '../../controllers/fetch-controller.js';
import type { TaskSummary, GlobalAuditEvent, AuditEvent } from '../../lib/shared-types.js';
import { EVENT_LABELS as BUSINESS_LABELS } from '../../lib/labels.js';
import { defaultRange, type DateRange } from '../../components/date-range.js';

type FilterId = 'all' | 'paid' | 'proofs' | 'tools' | 'decisions' | 'system';

interface AuditGlobalData {
  events: GlobalAuditEvent[];
  usdRate: number;
}

const FILTER_TYPES: Record<FilterId, string[]> = {
  all: [],
  paid: ['think_response', 'proof_created', 'receipt_stored'],
  proofs: ['proof_created', 'checkpoint_created'],
  tools: ['tool_call', 'tool_result'],
  decisions: ['think_request', 'think_response'],
  system: ['session_start', 'session_end', 'budget_check', 'error', 'memory_stored'],
};

const FILTER_COLORS: Record<FilterId, string> = {
  all: 'var(--accent, #14A8C4)',
  paid: '#fcbe2d',
  proofs: '#00b69b',
  tools: '#e0e0e8',
  decisions: '#14A8C4',
  system: 'rgba(255,255,255,0.5)',
};

const EVENT_ICONS: Record<string, string> = {
  think_request: '\u25CF', think_response: '\u25CF',
  tool_call: '\u25A0', tool_result: '\u25A0',
  proof_created: '\u25C6', checkpoint_created: '\u25C6',
  receipt_stored: '\u25CB', budget_check: '\u25B2',
  error: '\u2715', session_start: '\u25B7', session_end: '\u25C1',
  memory_stored: '\u25CF',
};

const EVENT_LABELS: Record<string, string> = Object.fromEntries(
  Object.entries(BUSINESS_LABELS).map(([k, v]) => [k, v])
);

@customElement('dm-audit-global')
export class WormAuditGlobal extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  private ctrl = new FetchController<AuditGlobalData>(this);
  @state() private activeFilter: FilterId = 'all';
  @state() private periodFilter = '7d';
  @state() private dateRange: DateRange = defaultRange('7d');
  @state() private visibleCount = 100;
  @state() private expandedIds = new Set<string>();

  static styles = [
    pageHost, centerState, pageTitle, sectionTitle, stateFeedback,
    css`
      .filter-row {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
        margin-bottom: 12px;
        align-items: center;
      }
      .chip {
        padding: 6px 12px;
        border-radius: 16px;
        font-size: 12px;
        font-weight: 500;
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        color: var(--text-dim, rgba(255,255,255,0.5));
        cursor: pointer;
        transition: all 0.15s ease;
        display: inline-flex;
        align-items: center;
        gap: 6px;
      }
      .chip:hover { color: var(--text, #e0e0e8); border-color: var(--text-dim, rgba(255,255,255,0.5)); }
      .chip.active {
        background: var(--chip-color, var(--accent));
        color: #fff;
        border-color: var(--chip-color, var(--accent));
        box-shadow: 0 0 0 1px var(--chip-color, var(--accent)), 0 2px 8px rgba(0,0,0,0.25);
        font-weight: 600;
      }
      .chip-count {
        font-family: var(--mono);
        font-size: 10px;
        background: rgba(0,0,0,0.25);
        padding: 1px 6px;
        border-radius: 8px;
        min-width: 18px;
        text-align: center;
      }
      .chip.active .chip-count {
        background: rgba(255,255,255,0.2);
      }
      .empty-filter {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 8px;
        padding: 48px 24px;
        text-align: center;
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-size: 13px;
      }
      .empty-filter-title {
        font-size: 15px;
        font-weight: 600;
        color: var(--text, rgba(255,255,255,0.8));
      }
      .filter-select {
        padding: 6px 10px;
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: 6px;
        color: var(--text, #e0e0e8);
        font-size: 12px;
      }
      .summary-bar {
        display: flex;
        gap: 16px;
        flex-wrap: wrap;
        padding: 10px 14px;
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: 14px;
        margin-bottom: 16px;
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.5));
      }
      .summary-bar strong {
        color: var(--text-bright, #fff);
        font-family: var(--mono);
      }
      .summary-bar .usd {
        color: var(--text-dim);
        opacity: 0.7;
        margin-left: 4px;
      }
      .event-card {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: 14px;
        padding: 10px 14px;
        margin-bottom: 6px;
        cursor: pointer;
        transition: background 0.15s ease;
      }
      .event-card:hover {
        background: rgba(255,255,255,0.03);
      }
      .event-header {
        display: flex;
        align-items: center;
        gap: 8px;
        flex-wrap: wrap;
        font-size: 12px;
      }
      .event-icon { font-size: 10px; color: var(--text-dim); }
      .event-time {
        font-family: var(--mono);
        color: var(--text-dim);
        font-size: 11px;
      }
      .event-type-badge {
        font-family: var(--mono);
        font-size: 10px;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        padding: 2px 6px;
        border-radius: 4px;
        background: rgba(72,128,255,0.12);
        color: var(--accent, #14A8C4);
      }
      .event-type-badge.proof_created,
      .event-type-badge.checkpoint_created {
        background: rgba(0,182,155,0.12);
        color: var(--success, #00b69b);
      }
      .event-type-badge.error {
        background: rgba(253,84,84,0.12);
        color: var(--error, #fd5454);
      }
      .event-task {
        font-family: var(--mono);
        font-size: 11px;
        color: var(--accent, #14A8C4);
        text-decoration: none;
      }
      .event-task:hover { text-decoration: underline; }
      .event-detail {
        color: var(--text-dim);
        font-size: 11px;
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .event-cost {
        font-family: var(--mono);
        font-size: 11px;
        color: var(--warning, #fcbe2d);
        white-space: nowrap;
      }
      .event-expand {
        padding: 8px 0 0;
        font-size: 11px;
        color: var(--text-dim);
        border-top: 1px solid var(--border, #1A3550);
        margin-top: 8px;
      }
      .event-expand pre {
        font-family: var(--mono);
        font-size: 11px;
        white-space: pre-wrap;
        word-break: break-all;
        max-height: 200px;
        overflow-y: auto;
        background: var(--bg, #0B1929);
        padding: 8px;
        border-radius: 4px;
        margin: 4px 0;
      }
      .proof-inline {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-top: 4px;
      }
      .proof-hash {
        font-family: var(--mono);
        font-size: 11px;
        color: var(--text-dim);
      }
      .mini-btn {
        font-size: 10px;
        padding: 2px 8px;
        border-radius: 4px;
        background: var(--bg);
        border: 1px solid var(--border);
        color: var(--accent);
        cursor: pointer;
        text-decoration: none;
      }
      .mini-btn:hover { background: var(--accent); color: #fff; border-color: var(--accent); }
      .load-more {
        display: block;
        width: 100%;
        padding: 10px;
        background: var(--bg-elevated);
        border: 1px solid var(--border);
        border-radius: 8px;
        color: var(--text-dim);
        font-size: 13px;
        cursor: pointer;
        text-align: center;
        margin-top: 8px;
      }
      .load-more:hover { color: var(--accent); border-color: var(--accent); }
      .load-more:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
      .chip:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }

      @media (max-width: 639px) {
        .event-header { gap: 4px; font-size: 11px; }
        .event-card { padding: 8px 10px; }
        .summary-bar { gap: 10px; padding: 8px 10px; font-size: 11px; }
        .event-expand pre { font-size: 10px; max-height: 150px; }
      }
    `
  ];

  connectedCallback() {
    super.connectedCallback();
    this.loadAudit();
  }

  private async loadAudit() {
    await this.ctrl.fetch(async () => {
      const [tasksRes, rate] = await Promise.all([
        this.fetchFn('/tasks'),
        fetchBsvUsdRate(),
      ]);
      if (!tasksRes.ok) throw new Error(`Tasks: ${tasksRes.status}`);
      const { tasks } = await tasksRes.json() as { tasks: TaskSummary[] };

      const recentTasks = tasks
        .sort((a, b) => parseTimestamp(b.started_at) - parseTimestamp(a.started_at))
        .slice(0, 30);

      const auditResults = await Promise.all(
        recentTasks.map(t =>
          this.fetchFn(`/task/${t.id}/audit`)
            .then(r => r.ok ? r.json() : { events: [] })
            .catch(() => ({ events: [] }))
        )
      );

      const events: GlobalAuditEvent[] = auditResults.flatMap((audit: { events: AuditEvent[] }, i: number) =>
        (audit.events ?? []).map((e: AuditEvent, idx: number) => ({
          timestamp: e.timestamp ?? 0,
          event_type: e.event_type ?? '',
          id: `${i}-${idx}`,
          data: e.data ?? {},
          task_id: recentTasks[i].id,
          task_text: recentTasks[i].task,
          task_status: recentTasks[i].status,
        }))
      );

      events.sort((a, b) => b.timestamp - a.timestamp);

      return { events, usdRate: rate };
    });
  }

  private get filteredEvents(): GlobalAuditEvent[] {
    let events = this.ctrl.data?.events ?? [];

    // Period filter via date range
    const fromTs = this.dateRange.from.getTime() / 1000;
    const toTs = this.dateRange.to.getTime() / 1000;
    events = events.filter(e => e.timestamp >= fromTs && e.timestamp <= toTs);

    // Type filter
    if (this.activeFilter !== 'all') {
      const types = FILTER_TYPES[this.activeFilter];
      events = events.filter(e => types.includes(e.event_type));
    }

    return events;
  }

  private extractSats(e: GlobalAuditEvent): number {
    const d = e.data;
    return (d.sats_paid as number) || (d.sats as number) || (d.sats_cost as number) || 0;
  }

  private extractDetail(e: GlobalAuditEvent): string {
    const d = e.data;
    if (e.event_type === 'think_response') return (d.model as string) ?? '';
    if (e.event_type === 'tool_call') return (d.name as string) ?? '';
    if (e.event_type === 'tool_result') return (d.name as string) ?? '';
    if (e.event_type === 'proof_created') return (d.proof_type as string) ?? '';
    if (e.event_type === 'error') return truncate((d.message as string) ?? '', 60);
    return '';
  }

  private toggleFilter(id: FilterId) {
    this.activeFilter = id;
    this.visibleCount = 100;
  }

  private toggleExpand(e: GlobalAuditEvent) {
    const key = `${e.task_id}-${e.id}-${e.timestamp}`;
    const next = new Set(this.expandedIds);
    if (next.has(key)) next.delete(key); else next.add(key);
    this.expandedIds = next;
  }

  private isExpanded(e: GlobalAuditEvent): boolean {
    return this.expandedIds.has(`${e.task_id}-${e.id}-${e.timestamp}`);
  }

  private countForFilter(id: FilterId): number {
    const events = this.ctrl.data?.events ?? [];
    if (id === 'all') return events.length;
    const types = FILTER_TYPES[id];
    return events.filter(e => types.includes(e.event_type)).length;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) return renderLoading('Loading audit trail...');
    if (this.ctrl.hasError && !this.ctrl.hasData) return renderError(this.ctrl.error ?? 'Failed to load audit trail', 'Check that the agent server is running.');

    const usdRate = this.ctrl.data?.usdRate ?? 0;
    const filtered = this.filteredEvents;
    const displayed = filtered.slice(0, this.visibleCount);
    const totalSats = filtered.reduce((s, e) => s + this.extractSats(e), 0);
    const paidCount = filtered.filter(e => this.extractSats(e) > 0).length;
    const taskIds = new Set(filtered.map(e => e.task_id));

    return html`
      <h1 class="page-title">Audit Trail</h1>

      <div class="filter-row">
        ${(Object.keys(FILTER_TYPES) as FilterId[]).map(id => html`
          <button class="chip ${this.activeFilter === id ? 'active' : ''}"
                  style="--chip-color: ${FILTER_COLORS[id]}"
                  @click=${() => this.toggleFilter(id)}>
            ${id === 'all' ? 'All' : id.charAt(0).toUpperCase() + id.slice(1)}
            <span class="chip-count">${this.countForFilter(id)}</span>
          </button>
        `)}

        <dm-date-range
          .value=${this.dateRange}
          @range-change=${(e: CustomEvent) => { this.dateRange = e.detail; this.visibleCount = 100; }}
        ></dm-date-range>
      </div>

      <div class="summary-bar">
        <span>Events: <strong>${filtered.length.toLocaleString()}</strong></span>
        <span>Paid: <strong>${paidCount.toLocaleString()}</strong></span>
        <span>Total: <strong title="${formatInlineCurrencyAlt(totalSats, usdRate, this.currencyMode)}">${formatInlineCurrency(totalSats, usdRate, this.currencyMode)}</strong></span>
        <span>Tasks: <strong>${taskIds.size}</strong></span>
      </div>

      ${filtered.length === 0 && this.activeFilter !== 'all' ? html`
        <div class="empty-filter">
          <div class="empty-filter-title">No events match this filter</div>
          <div>No "${this.activeFilter}" events found in the selected time period. Try a different filter or expand the time range.</div>
        </div>
      ` : filtered.length === 0 ? html`
        <div class="empty-filter">
          <div class="empty-filter-title">No events found</div>
          <div>No audit events in the selected time period. Try expanding the time range.</div>
        </div>
      ` : html`
        ${displayed.map(e => this.renderEvent(e, usdRate))}
      `}

      ${filtered.length > this.visibleCount ? html`
        <button class="load-more" @click=${() => { this.visibleCount += 100; }}>
          Load More (${(filtered.length - this.visibleCount).toLocaleString()} remaining)
        </button>
      ` : nothing}
    `;
  }

  private renderEvent(e: GlobalAuditEvent, usdRate: number) {
    const icon = EVENT_ICONS[e.event_type] ?? '\u25CF';
    const time = e.timestamp > 0 ? new Date(e.timestamp * 1000).toLocaleTimeString() : '';
    const typeLabel = EVENT_LABELS[e.event_type] ?? e.event_type;
    const taskShort = e.task_id.slice(0, 8);
    const sats = this.extractSats(e);
    const detail = this.extractDetail(e);
    const expanded = this.isExpanded(e);

    return html`
      <div class="event-card" @click=${() => this.toggleExpand(e)}>
        <div class="event-header">
          <span class="event-icon">${icon}</span>
          <span class="event-time">${time}</span>
          <span class="event-type-badge ${e.event_type}">${typeLabel}</span>
          <a class="event-task" href="#task/${e.task_id}" @click=${(ev: Event) => ev.stopPropagation()} title=${e.task_id}>${taskShort}</a>
          <span class="event-detail">${detail}</span>
          ${sats > 0 ? html`
            <span class="event-cost" title="${formatInlineCurrencyAlt(sats, usdRate, this.currencyMode)}">${formatInlineCurrency(sats, usdRate, this.currencyMode)}</span>
          ` : nothing}
        </div>
        ${expanded ? this.renderEventDetail(e) : nothing}
      </div>
    `;
  }

  private renderEventDetail(e: GlobalAuditEvent) {
    const d = e.data;
    return html`
      <div class="event-expand">
        <div style="margin-bottom:4px;color:var(--text)">Task: ${truncate(e.task_text, 80)}</div>
        ${e.event_type === 'proof_created' || e.event_type === 'checkpoint_created' ? html`
          <div class="proof-inline">
            <span class="proof-hash" title=${String(d.hash ?? '')}>${truncate(String(d.hash ?? ''), 24)}</span>
            ${d.txid ? html`
              <a class="mini-btn" href="https://whatsonchain.com/tx/${d.txid}"
                 target="_blank" rel="noopener" @click=${(ev: Event) => ev.stopPropagation()}>WhatsOnChain</a>
            ` : nothing}
          </div>
        ` : nothing}
        <pre>${JSON.stringify(d, null, 2)}</pre>
      </div>
    `;
  }
}
