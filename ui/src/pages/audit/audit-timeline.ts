/**
 * Timeline view for the per-task audit page.
 * Renders events grouped by iteration with collapsible details.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatDuration, truncate, formatSats } from '../../lib/util.js';
import { formatInlineCurrency, formatInlineCurrencyAlt } from '../../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../../lib/storage.js';
import type { ReceiptDetail, BudgetDetailResponse } from '../../lib/shared-types.js';
import { centerState } from '../../lib/shared-styles.js';
import { EVENT_LABELS, EVENT_DESCRIPTIONS } from '../../lib/labels.js';

// ---- Shared types (exported for audit.ts) ----

export interface AuditEvent {
  timestamp: number;
  event_type: string;
  data: Record<string, unknown>;
}

export interface IterationGroup {
  number: number;
  events: AuditEvent[];
  model: string;
  sats: number;
  duration: number;
  toolCount: number;
  proofCount: number;
  receipt?: ReceiptDetail;
}

// ---- Internal types ----

interface EventStyle {
  label: string;
  color: string;
  icon: string;
}

type FilterCategory = 'all' | 'think' | 'tools' | 'proofs' | 'budget' | 'system';

const FILTER_MAP: Record<FilterCategory, string[]> = {
  all: [],
  think: ['think_request', 'think_response'],
  tools: ['tool_call', 'tool_result'],
  proofs: ['proof_created', 'checkpoint_created', 'receipt_stored'],
  budget: ['budget_check'],
  system: ['system', 'user', 'session_start', 'session_end', 'error', 'loop_warning', 'continuation_save', 'continuation_resume', 'memory_stored'],
};

const EVENT_STYLES: Record<string, EventStyle> = {
  think_request: { label: 'THINK', color: 'var(--accent, #14A8C4)', icon: '\u25CF' },
  think_response: { label: 'THINK', color: 'var(--accent, #14A8C4)', icon: '\u25CF' },
  tool_call: { label: 'TOOL', color: 'var(--text-bright, #fff)', icon: '\u25A0' },
  tool_result: { label: 'RESULT', color: 'var(--text-bright, #fff)', icon: '\u25A0' },
  proof_created: { label: 'PROOF', color: 'var(--success, #00b69b)', icon: '\u25C6' },
  checkpoint_created: { label: 'CKPT', color: 'var(--warning, #fcbe2d)', icon: '\u25B2' },
  receipt_stored: { label: 'RECEIPT', color: 'var(--text-dim, rgba(255,255,255,0.5))', icon: '\u25CB' },
  budget_check: { label: '$', color: 'var(--warning, #fcbe2d)', icon: '\u25CF' },
  error: { label: 'ERROR', color: 'var(--error, #fd5454)', icon: '\u2717' },
  user: { label: 'USER', color: 'var(--accent, #14A8C4)', icon: '\u25B6' },
  system: { label: 'SYS', color: 'var(--text-dim, rgba(255,255,255,0.5))', icon: '\u25CB' },
  session_start: { label: 'START', color: 'var(--text-dim, rgba(255,255,255,0.5))', icon: '\u25B6' },
  session_end: { label: 'END', color: 'var(--text-dim, rgba(255,255,255,0.5))', icon: '\u25A0' },
  continuation_save: { label: 'SAVE', color: 'var(--accent, #14A8C4)', icon: '\u25A0' },
  continuation_resume: { label: 'RESUME', color: 'var(--accent, #14A8C4)', icon: '\u25B6' },
  loop_warning: { label: 'WARN', color: 'var(--warning, #fcbe2d)', icon: '\u25B2' },
  memory_stored: { label: 'MEM', color: 'var(--accent, #14A8C4)', icon: '\u25C6' },
};

const DEFAULT_STYLE: EventStyle = {
  label: 'EVENT',
  color: 'var(--text-dim, rgba(255,255,255,0.5))',
  icon: '\u25CB',
};

@customElement('dm-audit-timeline')
export class WormAuditTimeline extends LitElement {
  @property({ attribute: false }) events: AuditEvent[] = [];
  @property({ attribute: false }) receiptsByIteration = new Map<number, ReceiptDetail>();
  @property({ attribute: false }) budgetDetail: BudgetDetailResponse | null = null;
  @property() taskId = '';
  @property({ attribute: false }) usdRate = 0;
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  @state() private expandedEvents = new Set<string>();
  @state() private collapsedIterations = new Set<number>();
  @state() private allCollapsed = true;  // Start collapsed by default
  @state() private filter: FilterCategory = 'all';
  @state() private budgetExpanded = false;

  static styles = [centerState, css`
    :host { display: block; }

    .filter-bar { display: flex; gap: 4px; flex-wrap: wrap; }

    .filter-btn {
      padding: 3px 10px; font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: transparent;
      border: 1px solid var(--border, #1A3550);
      border-radius: 4px; cursor: pointer;
      transition: all 0.15s ease;
    }
    .filter-btn:hover { color: var(--text, #e0e0e8); border-color: var(--text-dim, rgba(255,255,255,0.5)); }
    .filter-btn.active { color: var(--accent, #14A8C4); border-color: var(--accent, #14A8C4); background: rgba(20, 168, 196, 0.08); }

    /* Iteration groups */
    .iter-group { margin-bottom: 8px; }
    .iter-group + .iter-group { padding-top: 8px; border-top: 1px solid var(--border, #1A3550); }

    .iter-header {
      display: flex; align-items: center; gap: 8px;
      padding: 8px 12px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px; cursor: pointer;
      transition: border-color 0.15s ease, background 0.15s ease;
      flex-wrap: wrap;
    }
    .iter-header:hover { border-color: var(--text-dim, rgba(255,255,255,0.5)); background: rgba(255,255,255,0.02); }

    .collapse-controls {
      display: flex; gap: 6px; margin-left: auto;
    }
    .collapse-btn {
      padding: 3px 8px; font-size: 10px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: transparent;
      border: 1px solid var(--border, #1A3550);
      border-radius: 4px; cursor: pointer;
      transition: all 0.15s ease;
    }
    .collapse-btn:hover { color: var(--text, #e0e0e8); border-color: var(--text-dim, rgba(255,255,255,0.5)); }

    .iter-chevron {
      font-size: 10px; color: var(--text-dim, rgba(255,255,255,0.5));
      transition: transform 0.2s ease; flex-shrink: 0;
    }
    .iter-chevron.open { transform: rotate(90deg); }

    .iter-num {
      font-size: 12px; font-weight: 600;
      font-family: var(--mono, monospace);
      color: var(--text-bright, #fff);
    }

    .iter-meta {
      display: flex; gap: 12px;
      font-size: 11px; font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
    }
    .iter-meta .val { color: var(--text, #e0e0e8); }
    .iter-meta .sats { color: var(--warning, #fcbe2d); }
    .iter-meta .refund { color: var(--success, #00b69b); }

    .iter-events {
      padding-left: 16px;
      border-left: 1px solid var(--border, #1A3550);
      margin-left: 12px; margin-top: 4px;
    }

    .ungrouped { margin-bottom: 4px; }
    .timeline { position: relative; }

    /* Timeline events */
    .tl-event {
      position: relative; margin-bottom: 2px;
      padding: 6px 12px; border-radius: 6px;
      cursor: pointer; transition: background 0.15s ease;
    }
    .tl-event:hover { background: rgba(255, 255, 255, 0.03); }
    .tl-event.expanded { background: rgba(255, 255, 255, 0.02); }

    .tl-header { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }

    .tl-time {
      font-size: 11px; font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5)); white-space: nowrap;
    }

    .tl-badge {
      display: inline-flex; align-items: center;
      padding: 1px 6px; border-radius: 3px;
      font-size: 10px; font-weight: 600;
      font-family: var(--mono, monospace); letter-spacing: 0.3px;
    }

    .tl-content { font-size: 13px; color: var(--text, #e0e0e8); margin-top: 2px; }
    .tl-content-dim { color: var(--text-dim, rgba(255,255,255,0.5)); }
    .tl-content code { font-family: var(--mono, monospace); font-size: 12px; background: var(--bg, #0B1929); padding: 1px 4px; border-radius: 3px; }
    .tl-txid { font-family: var(--mono, monospace); font-size: 12px; color: var(--text-dim, rgba(255,255,255,0.5)); }
    .verify-link { font-size: 11px; color: var(--accent, #14A8C4); text-decoration: none; margin-left: 4px; }
    .verify-link:hover { text-decoration: underline; }
    .sats-inline {
      display: inline-flex;
      align-items: center;
      gap: 3px;
      font-family: var(--mono, monospace);
      font-size: 11px;
      color: var(--warning, #fcbe2d);
      background: rgba(252, 190, 45, 0.08);
      padding: 1px 6px;
      border-radius: 3px;
      margin-left: 4px;
    }
    .proof-verified-badge {
      display: inline-flex;
      align-items: center;
      gap: 3px;
      font-size: 10px;
      font-family: var(--mono, monospace);
      padding: 1px 6px;
      border-radius: 3px;
      margin-left: 4px;
    }
    .proof-verified-badge.has-txid {
      color: var(--success, #00b69b);
      background: rgba(0, 182, 155, 0.1);
    }
    .proof-verified-badge.no-txid {
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: rgba(255, 255, 255, 0.05);
    }

    /* Expanded detail panel */
    .detail {
      margin-top: 8px; padding: 10px 12px;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px; font-size: 12px;
      font-family: var(--mono, monospace);
      line-height: 1.5; overflow-x: auto;
    }

    .detail-row { display: flex; gap: 8px; margin-bottom: 4px; }
    .detail-row:last-child { margin-bottom: 0; }
    .detail-label { color: var(--text-dim, rgba(255,255,255,0.5)); white-space: nowrap; min-width: 100px; flex-shrink: 0; }
    .detail-value { color: var(--text, #e0e0e8); word-break: break-all; }
    .detail-value.sats { color: var(--warning, #fcbe2d); }
    .detail-value.success { color: var(--success, #00b69b); }
    .detail-value.error { color: var(--error, #fd5454); }

    .detail-block {
      margin-top: 8px; padding: 8px;
      background: var(--bg-elevated, #142D42);
      border-radius: 4px; white-space: pre-wrap;
      word-break: break-word; max-height: 300px;
      overflow-y: auto; color: var(--text, #e0e0e8);
    }

    /* Budget breakdown */
    .budget-breakdown {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px; padding: 12px 16px;
      margin-bottom: 16px;
    }

    .budget-table {
      width: 100%; border-collapse: collapse;
      font-size: 12px; font-family: var(--mono, monospace);
    }

    .budget-table th {
      text-align: left; padding: 4px 8px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      border-bottom: 1px solid var(--border, #1A3550);
      font-weight: 600; font-size: 10px;
      text-transform: uppercase; letter-spacing: 0.3px;
    }

    .budget-table td { padding: 4px 8px; color: var(--text, #e0e0e8); }
    .budget-table td.sats { color: var(--warning, #fcbe2d); }
    .budget-table tr:hover td { background: rgba(255, 255, 255, 0.02); }

    @media (max-width: 639px) {
      .tl-header { flex-direction: column; align-items: flex-start; gap: 4px; }
      .iter-meta { gap: 8px; flex-wrap: wrap; }
    }
  `];

  render() {
    if (this.events.length === 0) {
      return html`<div class="center-state">No events recorded for this task.</div>`;
    }

    const { preEvents, iterations, postEvents } = this.buildIterations();

    // Default: all collapsed on first render
    if (this.allCollapsed && iterations.length > 0 && this.collapsedIterations.size === 0) {
      const all = new Set(iterations.map(it => it.number));
      this.collapsedIterations = all;
    }

    return html`
      <div class="filter-bar" style="margin-bottom:12px">
        ${(['all', 'think', 'tools', 'proofs', 'budget', 'system'] as FilterCategory[]).map((f) => html`
          <button class="filter-btn ${this.filter === f ? 'active' : ''}" @click=${() => { this.filter = f; }}>${f}</button>
        `)}
        <div class="collapse-controls">
          <button class="collapse-btn" @click=${() => this.expandAll(iterations)} title="Expand all iterations">\u25BC All</button>
          <button class="collapse-btn" @click=${() => this.collapseAll(iterations)} title="Collapse all iterations">\u25B6 All</button>
        </div>
      </div>

      ${this.budgetExpanded ? this.renderBudgetBreakdown() : nothing}

      <div class="timeline">
        ${preEvents.map((e, i) => html`<div class="ungrouped">${this.renderEvent(e, i)}</div>`)}
        ${iterations.map((iter) => {
          const collapsed = this.collapsedIterations.has(iter.number);
          const hasVisibleEvents = iter.events.some((e) => this.passesFilter(e.event_type));
          if (!hasVisibleEvents) return nothing;
          return html`
            <div class="iter-group">
              <div class="iter-header" @click=${() => this.toggleIteration(iter.number)}>
                <span class="iter-chevron ${collapsed ? '' : 'open'}">\u25B6</span>
                <span class="iter-num">Iteration ${iter.number}</span>
                <div class="iter-meta">
                  ${iter.model ? html`<span><span class="val">${iter.model}</span></span>` : nothing}
                  ${iter.sats ? html`<span title="${formatInlineCurrencyAlt(iter.sats, this.usdRate, this.currencyMode)}"><span class="sats">${formatInlineCurrency(iter.sats, this.usdRate, this.currencyMode)}</span></span>` : nothing}
                  ${iter.receipt && iter.receipt.sats_refunded > 0
                    ? html`<span title="${formatInlineCurrencyAlt(iter.receipt.sats_refunded, this.usdRate, this.currencyMode)}"><span class="refund">-${formatInlineCurrency(iter.receipt.sats_refunded, this.usdRate, this.currencyMode)}</span> refund</span>`
                    : nothing}
                  ${iter.receipt
                    ? html`<span><span class="val">${iter.receipt.tokens.toLocaleString()}</span> tok</span>`
                    : nothing}
                  ${iter.duration ? html`<span><span class="val">${(iter.duration / 1000).toFixed(1)}s</span></span>` : nothing}
                  ${iter.toolCount ? html`<span><span class="val">${iter.toolCount}</span> tool${iter.toolCount !== 1 ? 's' : ''}</span>` : nothing}
                  ${iter.proofCount ? html`<span><span class="val">${iter.proofCount}</span> proof${iter.proofCount !== 1 ? 's' : ''}</span>` : nothing}
                  ${collapsed ? html`<span style="color:var(--text-dim,rgba(255,255,255,0.3))">${iter.events.length} events</span>` : nothing}
                </div>
              </div>
              ${!collapsed ? html`
                <div class="iter-events">
                  ${iter.events.map((e, i) => this.renderEvent(e, preEvents.length + iterations.slice(0, iter.number - 1).reduce((n, it) => n + it.events.length, 0) + i))}
                </div>
              ` : nothing}
            </div>
          `;
        })}
        ${postEvents.map((e, i) => html`<div class="ungrouped">${this.renderEvent(e, this.events.length - postEvents.length + i)}</div>`)}
      </div>
    `;
  }

  /** Toggle budget breakdown visibility (called by parent via ref). */
  toggleBudget() { this.budgetExpanded = !this.budgetExpanded; }
  get isBudgetExpanded() { return this.budgetExpanded; }

  // --------------------------------------------------------------------------
  // Iteration builder
  // --------------------------------------------------------------------------

  private buildIterations(): { preEvents: AuditEvent[]; iterations: IterationGroup[]; postEvents: AuditEvent[] } {
    const preEvents: AuditEvent[] = [];
    const iterations: IterationGroup[] = [];
    const postEvents: AuditEvent[] = [];
    let current: IterationGroup | null = null;
    let seenSessionEnd = false;

    for (const event of this.events) {
      if (event.event_type === 'session_end') seenSessionEnd = true;
      if (event.event_type === 'think_request') {
        if (current) iterations.push(current);
        const d = event.data as Record<string, unknown>;
        current = { number: iterations.length + 1, events: [event], model: typeof d.model === 'string' ? d.model : '', sats: 0, duration: 0, toolCount: 0, proofCount: 0 };
      } else if (current) {
        current.events.push(event);
        const d = event.data as Record<string, unknown>;
        if (event.event_type === 'think_response') {
          current.sats += typeof d.sats_effective === 'number' ? d.sats_effective : 0;
          if (typeof d.duration_ms === 'number') current.duration += d.duration_ms;
        }
        if (event.event_type === 'tool_result' && typeof d.sats_paid === 'number') current.sats += d.sats_paid;
        if (event.event_type === 'tool_call') current.toolCount++;
        if (event.event_type === 'proof_created' || event.event_type === 'checkpoint_created') current.proofCount++;
        if (seenSessionEnd && (event.event_type === 'proof_created' || event.event_type === 'checkpoint_created' || event.event_type === 'receipt_stored')) {
          current.events.pop();
          postEvents.push(event);
        }
      } else {
        if (seenSessionEnd) postEvents.push(event);
        else preEvents.push(event);
      }
    }
    if (current) iterations.push(current);

    for (const iter of iterations) {
      const receipt = this.receiptsByIteration.get(iter.number);
      if (receipt) iter.receipt = receipt;
    }
    return { preEvents, iterations, postEvents };
  }

  // --------------------------------------------------------------------------
  // Event rendering
  // --------------------------------------------------------------------------

  private renderEvent(event: AuditEvent, index: number) {
    if (!this.passesFilter(event.event_type)) return nothing;
    const style = EVENT_STYLES[event.event_type] ?? DEFAULT_STYLE;
    const eid = `${event.event_type}-${index}`;
    const expanded = this.expandedEvents.has(eid);

    return html`
      <div class="tl-event ${expanded ? 'expanded' : ''}" @click=${() => this.toggleEvent(eid)}>
        <div class="tl-header">
          <span class="tl-time" title=${new Date(event.timestamp * 1000).toLocaleString()}>${new Date(event.timestamp * 1000).toLocaleTimeString('en-US', { hour12: false })}</span>
          <span class="tl-badge" style="color: ${style.color}; background: ${style.color}18;">${EVENT_LABELS[event.event_type] ?? style.label}</span>
          ${this.renderCollapsed(event)}
        </div>
        ${expanded ? this.renderDetail(event) : nothing}
      </div>
    `;
  }

  private renderCollapsed(event: AuditEvent) {
    const d = event.data as Record<string, unknown>;
    switch (event.event_type) {
      case 'think_request':
        return html`<span class="tl-content-dim">${d.model ? html`<code>${d.model}</code>` : 'Thinking...'}</span>`;
      case 'think_response': {
        const text = typeof d.content === 'string' ? d.content : '';
        const sats = typeof d.sats_effective === 'number' ? d.sats_effective : 0;
        const ptok = typeof d.prompt_tokens === 'number' ? d.prompt_tokens : 0;
        const ctok = typeof d.completion_tokens === 'number' ? d.completion_tokens : 0;
        const totalTok = ptok + ctok;
        return html`<span class="tl-content">
          ${text ? truncate(text, 120) : nothing}
          <span class="tl-content-dim">${totalTok ? html` ${totalTok.toLocaleString()} tok` : nothing}</span>${sats ? html`<span class="sats-inline" title="${formatInlineCurrencyAlt(sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(sats, this.usdRate, this.currencyMode)}</span>` : nothing}
        </span>`;
      }
      case 'tool_call': {
        const name = typeof d.name === 'string' ? d.name : 'unknown';
        const args = typeof d.arguments === 'object' && d.arguments ? JSON.stringify(d.arguments) : (typeof d.arguments === 'string' ? d.arguments : '');
        return html`<span class="tl-content"><code>${name}</code>${args ? html` <span class="tl-content-dim">${truncate(args, 80)}</span>` : nothing}</span>`;
      }
      case 'tool_result': {
        const name = typeof d.name === 'string' ? d.name : '';
        const success = d.success !== false;
        const output = typeof d.content === 'string' ? d.content : '';
        return html`<span class="tl-content">
          ${name ? html`<code>${name}</code>` : nothing}
          <span style="color: ${success ? 'var(--success)' : 'var(--error)'}">${success ? '\u2713' : '\u2717'}</span>
          ${output ? html` <span class="tl-content-dim">${truncate(output, 100)}</span>` : nothing}
        </span>`;
      }
      case 'proof_created': {
        const proofType = typeof d.proof_type === 'string' ? d.proof_type : 'unknown';
        const txid = typeof d.txid === 'string' ? d.txid : '';
        const proofDataPreview = typeof d.proof_data === 'string' ? truncate(d.proof_data, 60) : '';
        const satsCost = typeof d.sats_cost === 'number' ? d.sats_cost : 0;
        return html`<span class="tl-content">
          ${proofType}
          ${txid ? html`<span class="proof-verified-badge has-txid">\u2713 on-chain</span>` : html`<span class="proof-verified-badge no-txid">pending</span>`}
          ${txid ? html`<span class="tl-txid">${truncate(txid, 16)}</span> <a class="verify-link" href="#task/${this.taskId}/proofs" @click=${(e: Event) => e.stopPropagation()}>[verify]</a>` : nothing}
          ${proofDataPreview ? html` <span class="tl-content-dim">${proofDataPreview}</span>` : nothing}
          ${satsCost > 0 ? html` <span class="tl-content-dim" title="${formatInlineCurrencyAlt(satsCost, this.usdRate, this.currencyMode)}">${formatInlineCurrency(satsCost, this.usdRate, this.currencyMode)}</span>` : nothing}
        </span>`;
      }
      case 'checkpoint_created': {
        const basket = typeof d.basket === 'string' ? d.basket : '';
        const txid = typeof d.txid === 'string' ? d.txid : '';
        const hasData = !!d.checkpoint_data;
        return html`<span class="tl-content">${basket ? html`<code>${basket}</code>` : 'Checkpoint'} ${txid ? html`<span class="tl-txid">${truncate(txid, 16)}</span>` : nothing}${hasData ? html` <span class="tl-content-dim">(data recorded)</span>` : nothing}</span>`;
      }
      case 'receipt_stored': {
        const path = typeof d.path === 'string' ? d.path : '';
        const sats = typeof d.sats === 'number' ? d.sats : 0;
        return html`<span class="tl-content-dim">${path ? html`<code>${truncate(path, 40)}</code>` : ''} ${sats ? html`<span title="${formatInlineCurrencyAlt(sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(sats, this.usdRate, this.currencyMode)}</span>` : nothing}</span>`;
      }
      case 'budget_check': {
        const balance = typeof d.balance === 'number' ? d.balance : 0;
        const spentSession = typeof d.spent_session === 'number' ? d.spent_session : 0;
        return html`<span class="tl-content-dim">Balance: <span title="${formatInlineCurrencyAlt(balance, this.usdRate, this.currencyMode)}">${formatInlineCurrency(balance, this.usdRate, this.currencyMode)}</span>, session: <span title="${formatInlineCurrencyAlt(spentSession, this.usdRate, this.currencyMode)}">${formatInlineCurrency(spentSession, this.usdRate, this.currencyMode)}</span></span>`;
      }
      case 'error':
        return html`<span class="tl-content" style="color: var(--error)">${truncate(typeof d.error === 'string' ? d.error : JSON.stringify(d), 200)}</span>`;
      case 'user':
        return html`<span class="tl-content">${truncate(typeof d.content === 'string' ? d.content : '', 120)}</span>`;
      case 'session_start':
      case 'session_end': {
        const content = typeof d.content === 'string' ? d.content : '';
        const task = typeof d.task === 'string' ? d.task : '';
        const result = typeof d.result === 'string' ? d.result : '';
        const status = typeof d.status === 'string' ? d.status : '';
        return html`<span class="tl-content-dim">${content || task || result || status || event.event_type}</span>`;
      }
      case 'continuation_save':
      case 'continuation_resume':
        return html`<span class="tl-content-dim">${typeof d.reason === 'string' ? d.reason : event.event_type}</span>`;
      case 'loop_warning':
        return html`<span class="tl-content" style="color: var(--warning)">${typeof d.message === 'string' ? d.message : 'Loop detected'}</span>`;
      case 'memory_stored': {
        const category = typeof d.category === 'string' ? d.category : '';
        const preview = typeof d.content_preview === 'string' ? d.content_preview : '';
        const memId = typeof d.memory_id === 'string' ? d.memory_id : '';
        return html`<span class="tl-content"><code>${category}</code> ${preview ? truncate(preview, 80) : memId}</span>`;
      }
      default:
        return html`<span class="tl-content-dim">${truncate(JSON.stringify(d), 100)}</span>`;
    }
  }

  private renderDetail(event: AuditEvent) {
    const d = event.data as Record<string, unknown>;
    switch (event.event_type) {
      case 'think_request':
        return html`<div class="detail">${this.row('model', d.model)}${this.row('max_tokens', d.max_tokens)}${this.row('messages', d.message_count)}</div>`;
      case 'think_response': {
        const text = typeof d.content === 'string' ? d.content : '';
        return html`<div class="detail">
          ${this.row('model', d.model)}${this.satsRow('paid', d.sats_paid)}${this.satsRow('effective', d.sats_effective)}${this.satsRow('refunded', d.sats_refunded)}
          ${this.row('prompt tokens', d.prompt_tokens)}${this.row('completion tokens', d.completion_tokens)}${this.row('finish reason', d.finish_reason)}
          ${this.row('duration', typeof d.duration_ms === 'number' ? `${d.duration_ms}ms` : d.duration_ms)}
          ${d.tool_calls ? html`${this.row('tool calls', Array.isArray(d.tool_calls) ? `${(d.tool_calls as unknown[]).length} calls` : '?')}` : nothing}
          ${text ? html`<div class="detail-block">${text}</div>` : nothing}
        </div>`;
      }
      case 'tool_call': {
        const args = d.arguments;
        let formatted = '';
        if (typeof args === 'object' && args) { try { formatted = JSON.stringify(args, null, 2); } catch { formatted = String(args); } }
        else if (typeof args === 'string') { try { formatted = JSON.stringify(JSON.parse(args), null, 2); } catch { formatted = args; } }
        return html`<div class="detail">${this.row('name', d.name)}${this.row('call_id', d.call_id)}${formatted ? html`<div class="detail-block">${formatted}</div>` : nothing}</div>`;
      }
      case 'tool_result': {
        const output = typeof d.content === 'string' ? d.content : '';
        const success = d.success !== false;
        return html`<div class="detail">
          ${this.row('name', d.name)}${this.row('call_id', d.call_id)}
          <div class="detail-row"><span class="detail-label">success</span><span class="detail-value ${success ? 'success' : 'error'}">${success ? 'true' : 'false'}</span></div>
          ${typeof d.sats_paid === 'number' && d.sats_paid > 0 ? this.satsRow('sats paid', d.sats_paid) : nothing}
          ${output ? html`<div class="detail-block">${output}</div>` : nothing}
        </div>`;
      }
      case 'proof_created':
        return html`<div class="detail">
          ${this.row('type', d.proof_type)}${this.row('txid', d.txid)}${this.row('hash', d.hash)}
          ${typeof d.proof_data === 'string' ? html`${this.row('proof_timestamp', d.proof_timestamp)}<div class="detail-block">${d.proof_data}</div>` : nothing}
          ${typeof d.sats_cost === 'number' ? this.satsRow('cost', d.sats_cost) : nothing}
          ${typeof d.txid === 'string' ? html`<div class="detail-row"><span class="detail-label">on-chain</span><a class="verify-link" href="https://whatsonchain.com/tx/${d.txid}" target="_blank" @click=${(e: Event) => e.stopPropagation()}>View on WhatsOnChain</a></div>` : nothing}
        </div>`;
      case 'checkpoint_created':
        return html`<div class="detail">${this.row('token_type', d.token_type)}${this.row('basket', d.basket)}${this.row('txid', d.txid)}${d.checkpoint_data ? html`<div class="detail-block">${JSON.stringify(d.checkpoint_data, null, 2)}</div>` : nothing}</div>`;
      case 'receipt_stored':
        return html`<div class="detail">${this.row('path', d.path)}${this.row('txid', d.txid)}${this.satsRow('sats', d.sats)}</div>`;
      case 'budget_check':
        return html`<div class="detail">${this.satsRow('balance', d.balance)}${this.satsRow('session spent', d.spent_session)}${this.satsRow('hourly spent', d.spent_hour)}</div>`;
      case 'error': {
        const errorMsg = typeof d.error === 'string' ? d.error : '';
        return html`<div class="detail">${errorMsg ? html`<div class="detail-block" style="color: var(--error)">${errorMsg}</div>` : nothing}${d.context ? html`<div class="detail-block">${JSON.stringify(d.context, null, 2)}</div>` : nothing}</div>`;
      }
      case 'user':
      case 'system': {
        const content = typeof d.content === 'string' ? d.content : '';
        return content ? html`<div class="detail"><div class="detail-block">${content}</div></div>` : nothing;
      }
      case 'session_start':
        return html`<div class="detail">${this.row('content', d.content)}</div>`;
      case 'session_end':
        return html`<div class="detail">${this.row('status', d.status)}${this.row('iterations', d.iterations)}${this.row('result', d.result)}${this.row('duration', typeof d.duration_secs === 'number' ? formatDuration(d.duration_secs as number) : d.duration_secs)}${d.error ? html`${this.row('error', d.error)}` : nothing}</div>`;
      case 'continuation_save':
        return html`<div class="detail">${this.row('continuation_id', d.continuation_id)}${this.row('reason', d.reason)}${this.row('wake_at', d.wake_at)}</div>`;
      case 'continuation_resume':
        return html`<div class="detail">${this.row('continuation_id', d.continuation_id)}${this.row('original_task', d.original_task)}${this.row('paused_iteration', d.paused_iteration)}</div>`;
      case 'loop_warning': {
        const message = typeof d.message === 'string' ? d.message : '';
        return message ? html`<div class="detail"><div class="detail-block" style="color: var(--warning)">${message}</div></div>` : nothing;
      }
      case 'memory_stored': {
        const tags = Array.isArray(d.tags) ? (d.tags as string[]).join(', ') : '';
        return html`<div class="detail">${this.row('memory_id', d.memory_id)}${this.row('category', d.category)}${tags ? this.row('tags', tags) : nothing}${typeof d.content_preview === 'string' ? html`<div class="detail-block">${d.content_preview}</div>` : nothing}</div>`;
      }
      default:
        return html`<div class="detail"><div class="detail-block">${JSON.stringify(d, null, 2)}</div></div>`;
    }
  }

  // --------------------------------------------------------------------------
  // Budget breakdown
  // --------------------------------------------------------------------------

  private renderBudgetBreakdown() {
    const { categories, tools } = this.computeCostBreakdown();
    const totalCat = categories.reduce((s, c) => s + c.sats, 0);

    return html`
      <div class="budget-breakdown">
        ${categories.length > 0 ? html`
          <div style="margin-bottom:12px">
            <div style="display:flex;gap:16px;flex-wrap:wrap;margin-bottom:8px">
              ${categories.map(c => {
                const pct = totalCat > 0 ? ((c.sats / totalCat) * 100).toFixed(0) : '0';
                return html`<span style="font-size:12px;font-family:var(--mono,monospace)">
                  <span style="color:${c.color}">\u25CF</span> ${c.name}
                  <span style="color:var(--warning,#fcbe2d)" title="${formatInlineCurrencyAlt(c.sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(c.sats, this.usdRate, this.currencyMode)}</span>
                  <span style="color:var(--text-dim,rgba(255,255,255,0.5))">${pct}%</span>
                </span>`;
              })}
            </div>
            <div style="display:flex;height:6px;border-radius:3px;overflow:hidden;background:var(--border,#1A3550)">
              ${categories.map(c => {
                const pct = totalCat > 0 ? (c.sats / totalCat) * 100 : 0;
                return pct > 0 ? html`<div style="width:${pct}%;background:${c.color}"></div>` : nothing;
              })}
            </div>
          </div>
        ` : nothing}
        ${tools.length > 0 ? html`
          <table class="budget-table">
            <thead><tr><th>Tool</th><th>Calls</th><th>Total Sats</th><th>Avg Sats</th></tr></thead>
            <tbody>${tools.map(t => html`<tr><td>${t.name}</td><td>${t.count}</td><td class="sats" title="${formatInlineCurrencyAlt(t.sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(t.sats, this.usdRate, this.currencyMode)}</td><td class="sats" title="${formatInlineCurrencyAlt(t.count > 0 ? Math.round(t.sats / t.count) : 0, this.usdRate, this.currencyMode)}">${t.count > 0 ? formatInlineCurrency(Math.round(t.sats / t.count), this.usdRate, this.currencyMode) : '0'}</td></tr>`)}</tbody>
          </table>
        ` : nothing}
        ${this.budgetDetail && Object.keys(this.budgetDetail.by_service).length > 0 ? html`
          <table class="budget-table" style="margin-top:8px">
            <thead><tr><th>Service</th><th>Operation</th><th>Count</th><th>Total Sats</th><th>Avg Sats</th></tr></thead>
            <tbody>
              ${Object.entries(this.budgetDetail.by_service).map(([service, breakdown]) =>
                Object.entries(breakdown.operations).map(([op, stats]) => html`
                  <tr><td>${service}</td><td>${op}</td><td>${stats.count}</td><td class="sats" title="${formatInlineCurrencyAlt(stats.total_sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(stats.total_sats, this.usdRate, this.currencyMode)}</td><td class="sats" title="${formatInlineCurrencyAlt(stats.avg_sats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(stats.avg_sats, this.usdRate, this.currencyMode)}</td></tr>
                `)
              )}
            </tbody>
          </table>
        ` : nothing}
      </div>
    `;
  }

  private computeCostBreakdown() {
    let llmSats = 0;
    let proofSats = 0;
    const toolMap = new Map<string, { count: number; sats: number }>();

    for (const event of this.events) {
      const d = event.data as Record<string, unknown>;
      if (event.event_type === 'think_response') llmSats += typeof d.sats_effective === 'number' ? d.sats_effective : 0;
      if (event.event_type === 'tool_result' && typeof d.sats_paid === 'number' && d.sats_paid > 0) {
        const name = typeof d.name === 'string' ? d.name : 'unknown';
        const existing = toolMap.get(name) || { count: 0, sats: 0 };
        existing.count++;
        existing.sats += d.sats_paid as number;
        toolMap.set(name, existing);
      }
      if (event.event_type === 'proof_created' || event.event_type === 'checkpoint_created') proofSats += typeof d.sats_cost === 'number' ? d.sats_cost : 200;
    }

    let stateSats = 0;
    if (this.budgetDetail && Object.keys(this.budgetDetail.by_service).length > 0) {
      const bs = this.budgetDetail.by_service;
      if (bs['llm']) llmSats = bs['llm'].total_sats;
      if (bs['proofs']) proofSats = bs['proofs'].total_sats;
      if (bs['state']) stateSats = bs['state'].total_sats;
      if (bs['tool']) { toolMap.clear(); for (const [op, stats] of Object.entries(bs['tool'].operations)) toolMap.set(op, { count: stats.count, sats: stats.total_sats }); }
    }

    const toolSats = Array.from(toolMap.values()).reduce((s, t) => s + t.sats, 0);
    const tools = Array.from(toolMap.entries()).map(([name, data]) => ({ name, ...data })).sort((a, b) => b.sats - a.sats);
    const categories = [
      { name: 'LLM', sats: llmSats, color: 'var(--accent, #14A8C4)' },
      { name: 'Tools', sats: toolSats, color: 'var(--text-bright, #fff)' },
      { name: 'Proofs', sats: proofSats, color: 'var(--success, #00b69b)' },
      { name: 'State', sats: stateSats, color: 'var(--warning, #fcbe2d)' },
    ].filter(c => c.sats > 0);
    return { categories, tools };
  }

  // --------------------------------------------------------------------------
  // Helpers
  // --------------------------------------------------------------------------

  private toggleEvent(eid: string) {
    const next = new Set(this.expandedEvents);
    if (next.has(eid)) next.delete(eid);
    else next.add(eid);
    this.expandedEvents = next;
  }

  private toggleIteration(num: number) {
    const next = new Set(this.collapsedIterations);
    if (next.has(num)) next.delete(num);
    else next.add(num);
    this.collapsedIterations = next;
    this.allCollapsed = false;  // User has interacted, disable auto-collapse
  }

  private expandAll(iterations: IterationGroup[]) {
    void iterations;  // used for intent clarity
    this.collapsedIterations = new Set();
    this.allCollapsed = false;
  }

  private collapseAll(iterations: IterationGroup[]) {
    this.collapsedIterations = new Set(iterations.map(it => it.number));
    this.allCollapsed = false;
  }

  private passesFilter(eventType: string): boolean {
    if (this.filter === 'all') return true;
    return (FILTER_MAP[this.filter] ?? []).includes(eventType);
  }

  private row(label: string, value: unknown) {
    if (value === undefined || value === null || value === '') return nothing;
    return html`<div class="detail-row"><span class="detail-label">${label}</span><span class="detail-value">${String(value)}</span></div>`;
  }

  private satsRow(label: string, value: unknown) {
    if (value === undefined || value === null) return nothing;
    const num = typeof value === 'number' ? value : 0;
    return html`<div class="detail-row"><span class="detail-label">${label}</span><span class="detail-value sats" title="${formatInlineCurrencyAlt(num, this.usdRate, this.currencyMode)}">${formatInlineCurrency(num, this.usdRate, this.currencyMode)}</span></div>`;
  }
}
