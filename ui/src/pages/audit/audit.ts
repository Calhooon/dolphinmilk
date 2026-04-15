/**
 * Per-task audit page -- timeline, conversation, and chain views.
 * Timeline rendering delegated to <dm-audit-timeline>.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import { formatDuration } from '../../lib/util.js';
import { fetchBsvUsdRate, formatInlineCurrency, formatInlineCurrencyAlt } from '../../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../../lib/storage.js';
import type { ReceiptDetail, BudgetDetailResponse, ProofDetail, ProofsResponse, AuditSummary, AuditConversationMessage } from '../../lib/shared-types.js';
import { pageHost, centerState, pageTitle, stateFeedback, renderLoading, renderBreadcrumb, renderError } from '../../lib/shared-styles.js';
import { FetchController } from '../../controllers/fetch-controller.js';
import '../../components/proof-chain.js';
import '../../components/share-menu.js';
import './audit-timeline.js';
import './audit-artifacts.js';
import type { AuditEvent } from './audit-timeline.js';
import type { WormAuditTimeline } from './audit-timeline.js';

type ViewMode = 'timeline' | 'conversation' | 'chain' | 'artifacts';

interface AuditPageData {
  events: AuditEvent[];
  summary: AuditSummary | null;
  taskDescription: string;
  receiptsByIteration: Map<number, ReceiptDetail>;
  budgetDetail: BudgetDetailResponse | null;
  /** BRC-18 OP_RETURN proof chain — used for hash chain continuity check. */
  proofs: ProofDetail[];
  /** BRC-48 state tokens — spendable UTXOs, shown in chain view but not in continuity check. */
  checkpoints: ProofDetail[];
  conversationId: string | null;
}

@customElement('dm-audit')
export class WormAudit extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property() taskId = '';
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  private ctrl = new FetchController<AuditPageData>(this);
  @state() private usdRate = 0;
  @state() private view: ViewMode = 'timeline';
  @state() private conversation: AuditConversationMessage[] = [];
  @state() private conversationLoading = false;
  @state() private budgetExpanded = false;
  @state() private exporting: string | null = null;
  @state() private exportError: string | null = null;
  @state() private shareOpen = false;

  @query('dm-audit-timeline') private timelineEl!: WormAuditTimeline;

  static styles = [pageHost, centerState, pageTitle, stateFeedback, css`
    .page-title { margin-bottom: 4px; }

    .task-desc { font-size: 13px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-bottom: 20px; max-width: 600px; }
    .proof-status {
      font-size: 10px;
      font-family: var(--mono, monospace);
      margin-top: 4px;
      letter-spacing: 0.3px;
    }
    .proof-status.intact { color: var(--success, #00b69b); }
    .proof-status.broken { color: var(--error, #fd5454); }

    /* View toggle */
    .toolbar { display: flex; align-items: center; gap: 8px; margin-bottom: 16px; flex-wrap: wrap; }

    .view-toggle {
      display: flex; background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550); border-radius: 6px; overflow: hidden;
    }

    .view-btn {
      padding: 5px 12px; font-size: 12px; font-family: var(--sans, sans-serif);
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: transparent; border: none; cursor: pointer;
      transition: color 0.15s ease, background 0.15s ease;
    }
    .view-btn:hover { color: var(--text, #e0e0e8); }
    .view-btn.active { color: var(--accent, #14A8C4); background: rgba(20, 168, 196, 0.1); }

    /* Export buttons */
    .export-group { display: flex; gap: 6px; margin-left: auto; }
    .export-btn {
      display: inline-flex; align-items: center; gap: 6px;
      padding: 5px 12px; font-size: 12px; font-family: var(--sans, sans-serif);
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px; cursor: pointer;
      transition: color 0.15s ease, background 0.15s ease, border-color 0.15s ease;
    }
    .export-btn:hover { background: var(--accent, #14A8C4); color: #fff; border-color: var(--accent, #14A8C4); }
    .export-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .export-btn:disabled:hover { background: var(--bg-elevated, #142D42); color: var(--text-dim, rgba(255,255,255,0.5)); border-color: var(--border, #1A3550); }
    .export-icon { font-size: 13px; line-height: 1; }
    .share-wrap { position: relative; display: inline-block; }

    /* Summary cards */
    .summary-row {
      display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
      gap: 8px; margin-bottom: 16px;
    }
    .summary-card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px; padding: 12px 16px; text-align: center;
    }
    .summary-value { font-size: 16px; font-weight: 600; font-family: var(--mono, monospace); color: var(--text-bright, #fff); }
    .summary-value.sats { color: var(--warning, #fcbe2d); }
    .summary-label { font-size: 10px; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-top: 4px; }
    .summary-card.clickable { cursor: pointer; transition: border-color 0.15s ease; }
    .summary-card.clickable:hover { border-color: var(--text-dim, rgba(255,255,255,0.5)); }

    /* Conversation view */
    .conversation { display: flex; flex-direction: column; gap: 12px; }

    .conv-msg {
      padding: 12px 16px; border-radius: 8px;
      max-width: 85%; line-height: 1.5;
    }
    .conv-msg.user { align-self: flex-end; background: var(--bg-user, #14A8C4); color: var(--text-bright, #fff); }
    .conv-msg.assistant { align-self: flex-start; background: var(--bg-elevated, #142D42); border: 1px solid var(--border, #1A3550); color: var(--text, #e0e0e8); }
    .conv-msg.system { align-self: center; background: transparent; color: var(--text-dim, rgba(255,255,255,0.5)); font-size: 12px; font-style: italic; max-width: 100%; text-align: center; padding: 4px; }
    .conv-msg.tool { align-self: flex-start; background: var(--bg, #0B1929); border: 1px solid var(--border, #1A3550); border-left: 3px solid var(--text-dim, rgba(255,255,255,0.5)); font-family: var(--mono, monospace); font-size: 12px; color: var(--text-dim, rgba(255,255,255,0.5)); max-width: 85%; }

    .conv-role { font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 4px; color: var(--text-dim, rgba(255,255,255,0.5)); }
    .conv-msg.user .conv-role { color: rgba(255,255,255,0.6); }

    .conv-content { font-size: 13px; white-space: pre-wrap; word-break: break-word; }

    .conv-tool-calls { margin-top: 8px; display: flex; flex-direction: column; gap: 6px; }
    .conv-tool-call { padding: 6px 10px; background: var(--bg, #0B1929); border: 1px solid var(--border, #1A3550); border-radius: 4px; font-size: 12px; font-family: var(--mono, monospace); }
    .conv-tool-name { color: var(--accent, #14A8C4); font-weight: 600; }
    .conv-tool-args { color: var(--text-dim, rgba(255,255,255,0.5)); margin-top: 4px; white-space: pre-wrap; word-break: break-word; max-height: 200px; overflow-y: auto; }
    .conv-tool-result-content { max-height: 200px; overflow-y: auto; white-space: pre-wrap; word-break: break-word; }

    @media (max-width: 639px) {
      .summary-row { grid-template-columns: repeat(2, 1fr); }
      .conv-msg { max-width: 95%; }
    }
  `];

  firstUpdated() { this.loadAudit(); }

  updated(changed: Map<string, unknown>) {
    if (changed.has('taskId') && this.taskId) this.loadAudit();
  }

  private async loadAudit() {
    if (!this.taskId) return;
    // Reset conversation when task changes
    this.conversation = [];
    fetchBsvUsdRate().then(r => { this.usdRate = r; });

    await this.ctrl.fetch(async () => {
      const [auditRes, receiptsRes, budgetRes, proofsRes] = await Promise.all([
        this.fetchFn(`/task/${this.taskId}/audit`),
        this.fetchFn(`/task/${this.taskId}/receipts`),
        this.fetchFn(`/budget/detail?task_id=${this.taskId}`),
        this.fetchFn(`/task/${this.taskId}/proofs`),
      ]);
      if (!auditRes.ok) throw new Error(`Failed to load audit: ${auditRes.status}`);
      const data = await auditRes.json();
      const events: AuditEvent[] = data.events ?? [];
      const summary: AuditSummary | null = data.summary ?? null;

      const receiptsByIteration = new Map<number, ReceiptDetail>();
      if (receiptsRes.ok) {
        const receiptsData = await receiptsRes.json();
        for (const r of receiptsData.receipts ?? []) receiptsByIteration.set(r.iteration, r);
      }

      let budgetDetail: BudgetDetailResponse | null = null;
      if (budgetRes.ok) budgetDetail = await budgetRes.json();

      let proofs: ProofDetail[] = [];
      let checkpoints: ProofDetail[] = [];
      if (proofsRes.ok) {
        const proofsData: ProofsResponse = await proofsRes.json();
        proofs = proofsData.proofs ?? [];
        checkpoints = proofsData.checkpoints ?? [];
      }

      // Extract task description
      let taskDescription = '';
      const userEvent = events.find((e) => e.event_type === 'user');
      if (userEvent?.data) {
        const content = (userEvent.data as Record<string, unknown>).content;
        if (typeof content === 'string') taskDescription = content;
      }
      if (!taskDescription) {
        const startEvent = events.find((e) => e.event_type === 'session_start');
        if (startEvent?.data) {
          const task = (startEvent.data as Record<string, unknown>).task;
          if (typeof task === 'string') taskDescription = task;
        }
      }

      const conversationId: string | null = data.conversation_id ?? null;

      return { events, summary, taskDescription, receiptsByIteration, budgetDetail, proofs, checkpoints, conversationId };
    });
  }

  private async loadConversation() {
    if (!this.taskId || this.conversation.length > 0) return;
    this.conversationLoading = true;
    try {
      const res = await this.fetchFn(`/task/${this.taskId}/conversation`);
      if (!res.ok) throw new Error(`Failed: ${res.status}`);
      const data = await res.json();
      this.conversation = data.messages ?? [];
    } catch {
      this.conversation = [];
    } finally {
      this.conversationLoading = false;
    }
  }

  private async exportCsv() {
    if (this.exporting || !this.taskId) return;
    this.exporting = 'csv';
    this.exportError = null;
    try {
      const res = await this.fetchFn(`/task/${this.taskId}/audit/export?format=csv`);
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      const text = await res.text();
      const blob = new Blob([text], { type: 'text/csv;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `audit-${this.taskId.slice(0, 8)}.csv`;
      a.style.display = 'none';
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
    } catch (e: any) {
      console.error('CSV export failed:', e);
      this.exportError = `CSV export failed: ${e.message ?? e}`;
    } finally {
      this.exporting = null;
    }
  }

  private async exportJson() {
    if (this.exporting || !this.taskId) return;
    this.exporting = 'json';
    this.exportError = null;
    try {
      const res = await this.fetchFn(`/task/${this.taskId}/audit/export?format=json`);
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      const text = await res.text();
      const blob = new Blob([text], { type: 'application/json;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `audit-${this.taskId.slice(0, 8)}.json`;
      a.style.display = 'none';
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
    } catch (e: any) {
      console.error('JSON export failed:', e);
      this.exportError = `JSON export failed: ${e.message ?? e}`;
    } finally {
      this.exporting = null;
    }
  }

  private setView(v: ViewMode) {
    this.view = v;
    if (v === 'conversation' && this.conversation.length === 0) this.loadConversation();
  }

  private renderProofChainStatus(proofs: ProofDetail[]) {
    if (proofs.length < 2) return nothing;
    // Check chain continuity: each proof's prev_hash should match the previous proof's hash
    let breaks = 0;
    for (let i = 1; i < proofs.length; i++) {
      if (proofs[i].prev_hash && proofs[i].prev_hash !== proofs[i - 1].hash) {
        breaks++;
      }
    }
    if (breaks === 0) {
      return html`<div class="proof-status intact">chain intact</div>`;
    }
    return html`<div class="proof-status broken">${breaks} break${breaks > 1 ? 's' : ''}</div>`;
  }

  private renderChain() {
    const d = this.ctrl.data;
    if (!d) return html`<div class="center-state">No proofs available for chain visualization</div>`;
    // Merge BRC-18 proofs + BRC-48 state tokens sorted by timestamp for the full on-chain picture.
    const allOnChain = [...d.proofs, ...d.checkpoints].sort((a, b) => a.timestamp - b.timestamp);
    if (allOnChain.length === 0) return html`<div class="center-state">No proofs available for chain visualization</div>`;
    return html`<dm-proof-chain .proofs=${allOnChain} .currencyMode=${this.currencyMode} .usdRate=${this.usdRate}></dm-proof-chain>`;
  }

  private renderConversation() {
    if (this.conversationLoading) return html`<div class="center-state">Loading conversation...</div>`;
    if (this.conversation.length === 0) return html`<div class="center-state">No conversation data available.</div>`;

    return html`
      <div class="conversation">
        ${this.conversation.map((msg) => {
          if (msg.role === 'tool') {
            return html`<div class="conv-msg tool"><div class="conv-role">tool result</div><div class="conv-tool-result-content">${msg.content}</div></div>`;
          }
          const hasToolCalls = msg.tool_calls && msg.tool_calls.length > 0;
          return html`
            <div class="conv-msg ${msg.role}">
              <div class="conv-role">${msg.role}</div>
              <div class="conv-content">${msg.content || (hasToolCalls ? '' : '(empty)')}</div>
              ${hasToolCalls ? html`
                <div class="conv-tool-calls">
                  ${msg.tool_calls!.map((tc) => {
                    let formattedArgs = tc.function.arguments;
                    try { formattedArgs = JSON.stringify(JSON.parse(tc.function.arguments), null, 2); } catch { /* keep raw */ }
                    return html`<div class="conv-tool-call"><span class="conv-tool-name">${tc.function.name}</span><div class="conv-tool-args">${formattedArgs}</div></div>`;
                  })}
                </div>
              ` : nothing}
            </div>
          `;
        })}
      </div>
    `;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) return renderLoading('Loading audit trail...');
    if (this.ctrl.hasError && !this.ctrl.hasData) return renderError(this.ctrl.error ?? 'Failed to load audit data', 'Check the task ID and that the agent server is running.');

    const d = this.ctrl.data;
    if (!d) return html`<div class="center-state">No audit data available.</div>`;

    const taskShort = this.taskId.length > 8 ? this.taskId.slice(0, 8) : this.taskId;

    return html`
      ${d.conversationId
        ? renderBreadcrumb([
            { label: 'Conversations', hash: '#conversations' },
            { label: 'Conversation', hash: `#conversation/${d.conversationId}` },
            { label: `Task #${taskShort}` },
          ])
        : renderBreadcrumb([
            { label: 'Activity', hash: '#tasks' },
            { label: `Task #${taskShort}` },
          ])
      }
      <div class="page-title">Audit Timeline</div>
      ${d.taskDescription ? html`<div class="task-desc">${d.taskDescription}</div>` : nothing}

      ${d.summary ? html`
        <div class="summary-row">
          <div class="summary-card">
            <div class="summary-value">${d.summary.iterations}</div>
            <div class="summary-label">Iterations</div>
          </div>
          <div class="summary-card clickable" @click=${() => { this.budgetExpanded = !this.budgetExpanded; if (this.timelineEl) this.timelineEl.toggleBudget(); }}
               title="${formatInlineCurrencyAlt(d.summary.sats_spent, this.usdRate, this.currencyMode)}">
            <div class="summary-value sats">${formatInlineCurrency(d.summary.sats_spent, this.usdRate, this.currencyMode)}</div>
            <div class="summary-label">Spent (LLM + tools) <span style="font-size:10px">${this.budgetExpanded ? '\u25B2' : '\u25BC'}</span></div>
          </div>
          <div class="summary-card">
            <div class="summary-value">${formatDuration(d.summary.duration_secs)}</div>
            <div class="summary-label">Duration</div>
          </div>
          <div class="summary-card">
            <div class="summary-value">${d.summary.proof_count}</div>
            <div class="summary-label">Proofs</div>
            ${this.renderProofChainStatus(d.proofs)}
          </div>
          <div class="summary-card">
            <div class="summary-value">${d.summary.tool_calls}</div>
            <div class="summary-label">Tool Calls</div>
          </div>
        </div>
      ` : nothing}

      <div class="toolbar">
        <div class="view-toggle">
          <button class="view-btn ${this.view === 'timeline' ? 'active' : ''}" @click=${() => this.setView('timeline')}>Timeline</button>
          <button class="view-btn ${this.view === 'conversation' ? 'active' : ''}" @click=${() => this.setView('conversation')}>Conversation</button>
          <button class="view-btn ${this.view === 'chain' ? 'active' : ''}" @click=${() => this.setView('chain')}>Chain</button>
          <button class="view-btn ${this.view === 'artifacts' ? 'active' : ''}" @click=${() => this.setView('artifacts')}>Artifacts</button>
          <a class="view-btn" href="#task/${this.taskId}/proofs" style="text-decoration:none">Proofs</a>
        </div>
        <div class="export-group">
          <button class="export-btn" @click=${this.exportCsv} ?disabled=${this.exporting !== null}>
            <span class="export-icon">\u2913</span>${this.exporting === 'csv' ? 'Exporting...' : 'Export CSV'}
          </button>
          <button class="export-btn" @click=${this.exportJson} ?disabled=${this.exporting !== null}>
            <span class="export-icon">{}</span>${this.exporting === 'json' ? 'Exporting...' : 'Export JSON'}
          </button>
          <div class="share-wrap">
            <button class="export-btn" @click=${() => { this.shareOpen = !this.shareOpen; }}
                    aria-label="Share task">
              <span class="export-icon">\u{1F4E4}</span>Share
            </button>
            <dm-share-menu
              .open=${this.shareOpen}
              .taskId=${this.taskId}
              .fetchFn=${this.fetchFn}
              @close=${() => { this.shareOpen = false; }}
            ></dm-share-menu>
          </div>
        </div>
      </div>
      ${this.exportError ? html`<div style="color:var(--error,#fd5454);font-size:12px;margin-bottom:12px;font-family:var(--mono,monospace)">${this.exportError}</div>` : nothing}

      ${this.view === 'chain' ? this.renderChain()
      : this.view === 'conversation' ? this.renderConversation()
      : this.view === 'artifacts' ? html`
          <dm-audit-artifacts
            .fetchFn=${this.fetchFn}
            .taskId=${this.taskId}
            .currencyMode=${this.currencyMode}
            .usdRate=${this.usdRate}
          ></dm-audit-artifacts>
        `
      : html`
          <dm-audit-timeline
            .events=${d.events}
            .receiptsByIteration=${d.receiptsByIteration}
            .budgetDetail=${d.budgetDetail}
            .taskId=${this.taskId}
            .usdRate=${this.usdRate}
            .currencyMode=${this.currencyMode}
          ></dm-audit-timeline>
        `}
    `;
  }
}
