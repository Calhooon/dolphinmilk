import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, formatDuration, formatSats } from '../lib/util.js';
import { fetchBsvUsdRate, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import {
  pageHost, centerState, statsBar, badges, pageTitle, tabBar,
  stateFeedback, renderLoading, renderError, renderBreadcrumb, skeleton,
} from '../lib/shared-styles.js';
import type {
  ConversationDetail, ConversationSummaryStats, ConversationTaskDetail,
  ConversationMessage, ChainVerification,
} from '../lib/shared-types.js';

type Tab = 'messages' | 'artifacts' | 'audit' | 'chain';

interface ArtifactItem {
  name: string;
  type: string;
  size_bytes: number;
  created_by: string;
  created_at: number;
  url: string;
  task_id: string;
  turn: number;
  turn_prompt: string;
}

interface ArtifactsResponse {
  conversation_id: string;
  artifacts: ArtifactItem[];
  total: number;
  by_type: Record<string, number>;
}

interface TaskAuditCache {
  events: any[];
  summary: any;
}

@customElement('dm-conversation-detail')
export class ConversationDetailPage extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @property() conversationId = '';
  @property() initialTab = '';

  @state() private data: ConversationDetail | null = null;
  @state() private loading = true;
  @state() private error = '';
  @state() private activeTab: Tab = 'messages';
  @state() private usdRate = 0;

  // Artifacts tab
  @state() private artifacts: ArtifactsResponse | null = null;
  @state() private artifactsLoading = false;
  @state() private artifactTypeFilter = 'all';

  // Audit tab
  @state() private auditCache = new Map<string, TaskAuditCache>();
  @state() private auditExpanded = new Set<string>();
  @state() private auditLoading = new Set<string>();

  // Chain tab
  @state() private chainVerification: ChainVerification | null = null;
  @state() private chainLoading = false;

  static styles = [pageHost, centerState, statsBar, badges, pageTitle, tabBar, stateFeedback, skeleton, css`
    :host { display: block; }

    .detail-header {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 16px;
    }
    .detail-header .back-link {
      color: var(--text-dim);
      text-decoration: none;
      font-size: 13px;
      display: flex;
      align-items: center;
      gap: 4px;
    }
    .detail-header .back-link:hover { color: var(--accent); }
    .detail-title {
      flex: 1;
      font-size: 20px;
      font-weight: 600;
      color: var(--text-bright, #fff);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .continue-btn {
      background: var(--accent, #14A8C4);
      color: #fff;
      border: none;
      padding: 6px 14px;
      border-radius: 8px;
      font-size: 13px;
      font-weight: 500;
      cursor: pointer;
      text-decoration: none;
      font-family: var(--sans);
    }
    .continue-btn:hover { opacity: 0.9; }

    /* Summary cards */
    .summary-cards {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
      gap: 10px;
      margin-bottom: 16px;
    }
    .stat-card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 12px;
      padding: 12px 14px;
      text-align: center;
    }
    .stat-card .value {
      font-size: 22px;
      font-weight: 700;
      color: var(--text-bright);
      font-family: var(--mono);
    }
    .stat-card .label {
      font-size: 11px;
      color: var(--text-dim);
      margin-top: 2px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }

    /* Tab bar */
    .tabs {
      display: flex;
      gap: 0;
      border-bottom: 1px solid var(--border);
      margin-bottom: 16px;
    }
    .tab-btn {
      background: none;
      border: none;
      padding: 10px 18px;
      font-size: 13px;
      font-weight: 500;
      color: var(--text-dim);
      cursor: pointer;
      border-bottom: 2px solid transparent;
      transition: color 0.15s, border-color 0.15s;
      font-family: var(--sans);
    }
    .tab-btn:hover { color: var(--text); }
    .tab-btn[data-active] {
      color: var(--accent);
      border-bottom-color: var(--accent);
    }

    /* Messages */
    .messages-list {
      display: flex;
      flex-direction: column;
      gap: 4px;
    }
    .task-boundary {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 0;
      font-size: 11px;
      color: var(--text-dim);
    }
    .task-boundary::before,
    .task-boundary::after {
      content: '';
      flex: 1;
      height: 1px;
      background: var(--border);
    }
    .task-boundary .cost {
      color: var(--warning, #fcbe2d);
      font-family: var(--mono);
    }
    .msg-row {
      display: flex;
      flex-direction: column;
      gap: 4px;
      padding: 10px 14px;
      border-radius: 12px;
    }
    .msg-row.user {
      background: var(--bg-user, #1E3A5F);
      align-self: flex-end;
      max-width: 85%;
      border-radius: 12px 12px 4px 12px;
    }
    .msg-row.assistant {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border);
      max-width: 92%;
    }
    .msg-row.tool {
      background: var(--bg-surface, #0F2233);
      border: 1px solid var(--border);
      font-family: var(--mono);
      font-size: 12px;
      max-width: 92%;
      opacity: 0.8;
    }
    .msg-role {
      font-size: 11px;
      font-weight: 600;
      color: var(--text-dim);
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }
    .msg-content {
      font-size: 14px;
      line-height: 1.5;
      color: var(--text);
      white-space: pre-wrap;
      word-break: break-word;
    }
    .msg-meta {
      display: flex;
      gap: 8px;
      font-size: 11px;
      color: var(--text-dim);
      font-family: var(--mono);
    }

    /* Artifacts */
    .artifact-filters {
      display: flex;
      gap: 6px;
      margin-bottom: 14px;
      flex-wrap: wrap;
    }
    .filter-pill {
      background: var(--bg-elevated);
      border: 1px solid var(--border);
      border-radius: 20px;
      padding: 4px 12px;
      font-size: 12px;
      cursor: pointer;
      color: var(--text-dim);
      font-family: var(--sans);
    }
    .filter-pill[data-active] {
      background: var(--accent);
      color: #fff;
      border-color: var(--accent);
    }
    .turn-group {
      margin-bottom: 20px;
    }
    .turn-header {
      font-size: 13px;
      color: var(--text-dim);
      margin-bottom: 8px;
      padding-bottom: 4px;
      border-bottom: 1px solid var(--border);
    }
    .turn-header strong { color: var(--text); }
    .artifact-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
      gap: 10px;
    }
    .artifact-card {
      background: var(--bg-elevated);
      border: 1px solid var(--border);
      border-radius: 10px;
      overflow: hidden;
      cursor: pointer;
      transition: border-color 0.15s;
    }
    .artifact-card:hover { border-color: var(--accent); }
    .artifact-card img {
      width: 100%;
      aspect-ratio: 1;
      object-fit: cover;
      display: block;
    }
    .artifact-card .info {
      padding: 8px 10px;
      font-size: 12px;
    }
    .artifact-card .name {
      color: var(--text);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .artifact-card .meta {
      color: var(--text-dim);
      font-family: var(--mono);
      font-size: 11px;
    }

    /* Audit */
    .audit-summary {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
      gap: 10px;
      margin-bottom: 16px;
    }
    .show-work-btn {
      background: none;
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 8px 14px;
      font-size: 13px;
      cursor: pointer;
      color: var(--text-dim);
      width: 100%;
      text-align: left;
      display: flex;
      align-items: center;
      gap: 6px;
      transition: background 0.15s;
      font-family: var(--sans);
    }
    .show-work-btn:hover { background: var(--bg-elevated); }
    .show-work-btn .arrow { display: inline-block; }
    .task-detail-link {
      font-size: 12px;
      color: var(--accent);
      text-decoration: none;
      white-space: nowrap;
      padding: 6px 10px;
    }
    .task-detail-link:hover { text-decoration: underline; }
    .work-detail {
      background: var(--bg-surface);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 12px;
      margin-top: 6px;
      font-size: 12px;
    }
    .iter-row {
      display: flex;
      gap: 8px;
      padding: 4px 0;
      border-bottom: 1px solid rgba(255,255,255,0.05);
      align-items: center;
    }
    .iter-row:last-child { border-bottom: none; }
    .iter-badge {
      background: var(--bg-elevated);
      border-radius: 4px;
      padding: 2px 6px;
      font-size: 11px;
      font-family: var(--mono);
      color: var(--text-dim);
      min-width: 28px;
      text-align: center;
    }
    .iter-type {
      font-size: 12px;
      color: var(--text);
      flex: 1;
    }
    .iter-cost {
      font-family: var(--mono);
      font-size: 11px;
      color: var(--warning);
    }

    /* Chain */
    .chain-msg {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 6px 10px;
      border-radius: 8px;
      font-size: 13px;
      border: 1px solid var(--border);
      margin-bottom: 4px;
    }
    .chain-msg .check { font-size: 16px; }
    .chain-msg .check.valid { color: var(--success); }
    .chain-msg .check.invalid { color: var(--error); }
    .chain-msg .hash {
      font-family: var(--mono);
      font-size: 11px;
      color: var(--text-dim);
      overflow: hidden;
      text-overflow: ellipsis;
      max-width: 200px;
    }
    .chain-status {
      padding: 12px;
      border-radius: 10px;
      margin-bottom: 12px;
      font-size: 14px;
      font-weight: 500;
    }
    .chain-status.valid {
      background: rgba(0, 182, 155, 0.1);
      border: 1px solid var(--success);
      color: var(--success);
    }
    .chain-status.invalid {
      background: rgba(253, 84, 84, 0.1);
      border: 1px solid var(--error);
      color: var(--error);
    }

    /* Empty state */
    .empty-state {
      text-align: center;
      padding: 40px 20px;
      color: var(--text-dim);
    }
    .empty-state .title { font-size: 16px; color: var(--text); margin-bottom: 6px; }
    .empty-state .subtitle { font-size: 13px; }

    /* Footer */
    .detail-footer {
      margin-top: 16px;
      padding: 10px 0;
      border-top: 1px solid var(--border);
      font-size: 12px;
      color: var(--text-dim);
      display: flex;
      gap: 12px;
      align-items: center;
    }

    @media (max-width: 639px) {
      .summary-cards { grid-template-columns: repeat(2, 1fr); }
      .artifact-grid { grid-template-columns: repeat(2, 1fr); }
      .msg-row.user, .msg-row.assistant { max-width: 100%; }
      .tab-btn { padding: 8px 12px; font-size: 12px; }
    }
  `];

  connectedCallback() {
    super.connectedCallback();
    if (this.initialTab && ['messages', 'artifacts', 'audit', 'chain'].includes(this.initialTab)) {
      this.activeTab = this.initialTab as Tab;
    }
    this.loadData();
    fetchBsvUsdRate(this.fetchFn).then(r => { this.usdRate = r; });
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('conversationId') && this.conversationId) {
      this.loadData();
    }
    if (changed.has('initialTab') && this.initialTab) {
      const t = this.initialTab as Tab;
      if (['messages', 'artifacts', 'audit', 'chain'].includes(t)) {
        this.setTab(t);
      }
    }
  }

  private async loadData() {
    if (!this.conversationId) return;
    this.loading = true;
    this.error = '';
    try {
      const resp = await this.fetchFn(`/conversations/${this.conversationId}`);
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      this.data = await resp.json();
    } catch (e: any) {
      this.error = e.message || 'Failed to load conversation';
    } finally {
      this.loading = false;
    }
  }

  private async loadArtifacts() {
    if (this.artifacts || this.artifactsLoading) return;
    this.artifactsLoading = true;
    try {
      const resp = await this.fetchFn(`/conversations/${this.conversationId}/artifacts`);
      if (resp.ok) this.artifacts = await resp.json();
    } finally {
      this.artifactsLoading = false;
    }
  }

  private async loadTaskAudit(taskId: string) {
    if (this.auditCache.has(taskId) || this.auditLoading.has(taskId)) return;
    this.auditLoading = new Set([...this.auditLoading, taskId]);
    try {
      const resp = await this.fetchFn(`/task/${taskId}/audit`);
      if (resp.ok) {
        const data = await resp.json();
        this.auditCache = new Map([...this.auditCache, [taskId, { events: data.events, summary: data.summary }]]);
      }
    } finally {
      const next = new Set(this.auditLoading);
      next.delete(taskId);
      this.auditLoading = next;
    }
  }

  private async loadChain() {
    if (this.chainVerification || this.chainLoading) return;
    this.chainLoading = true;
    try {
      const resp = await this.fetchFn(`/conversations/${this.conversationId}/verify`);
      if (resp.ok) this.chainVerification = await resp.json();
    } finally {
      this.chainLoading = false;
    }
  }

  private setTab(tab: Tab) {
    this.activeTab = tab;
    // Update URL to reflect current tab (enables deep linking + back/forward)
    const base = `#conversation/${this.conversationId}`;
    const newHash = tab === 'messages' ? base : `${base}/${tab}`;
    if (window.location.hash !== newHash) {
      history.replaceState(null, '', newHash);
    }
    if (tab === 'artifacts') this.loadArtifacts();
    if (tab === 'chain') this.loadChain();
  }

  private toggleAudit(taskId: string) {
    const next = new Set(this.auditExpanded);
    if (next.has(taskId)) {
      next.delete(taskId);
    } else {
      next.add(taskId);
      this.loadTaskAudit(taskId);
    }
    this.auditExpanded = next;
  }

  private fmtCost(sats: number): string {
    return formatInlineCurrency(sats, this.usdRate, this.currencyMode);
  }

  private fmtCostAlt(sats: number): string {
    return formatInlineCurrencyAlt(sats, this.usdRate, this.currencyMode);
  }

  private formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  render() {
    if (this.loading) return renderLoading('Loading conversation...');
    if (this.error) return renderError(this.error);
    if (!this.data) return renderError('Conversation not found');

    const { conversation: conv, summary } = this.data;

    return html`
      <div class="detail-header">
        <a class="back-link" href="#conversations">\u2190 Conversations</a>
        <div class="detail-title">${conv.title}</div>
        <a class="continue-btn" href="#chat/${conv.id}">Continue \u2192</a>
      </div>

      ${this.renderSummaryCards(summary)}

      <div class="tabs">
        ${(['messages', 'artifacts', 'audit', 'chain'] as Tab[]).map(t => html`
          <button class="tab-btn"
            data-tab=${t}
            ?data-active=${this.activeTab === t}
            @click=${() => this.setTab(t)}>
            ${t === 'messages' ? 'Messages' : t === 'artifacts' ? 'Artifacts' : t === 'audit' ? 'Audit' : 'Chain'}
          </button>
        `)}
      </div>

      ${this.activeTab === 'messages' ? this.renderMessages() : nothing}
      ${this.activeTab === 'artifacts' ? this.renderArtifacts() : nothing}
      ${this.activeTab === 'audit' ? this.renderAudit() : nothing}
      ${this.activeTab === 'chain' ? this.renderChain() : nothing}

      <div class="detail-footer">
        <span>${conv.message_count} messages</span>
        <span>${conv.task_ids.length} task${conv.task_ids.length !== 1 ? 's' : ''}</span>
        <span title="${this.fmtCostAlt(conv.total_sats)}">${this.fmtCost(conv.total_sats)}</span>
        ${summary.duration_secs > 0 ? html`<span>${formatDuration(summary.duration_secs)}</span>` : nothing}
      </div>
    `;
  }

  private renderSummaryCards(s: ConversationSummaryStats) {
    return html`
      <div class="summary-cards">
        <div class="stat-card">
          <div class="value" title="${this.fmtCostAlt(this.data!.conversation.total_sats)}">${this.fmtCost(this.data!.conversation.total_sats)}</div>
          <div class="label">Total Cost</div>
        </div>
        <div class="stat-card">
          <div class="value">${s.total_iterations}</div>
          <div class="label">Iterations</div>
        </div>
        <div class="stat-card">
          <div class="value">${s.total_proofs}</div>
          <div class="label">Proofs</div>
        </div>
        <div class="stat-card">
          <div class="value">${s.total_artifacts}</div>
          <div class="label">Artifacts</div>
        </div>
      </div>
    `;
  }

  private renderMessages() {
    if (!this.data) return nothing;
    const { messages } = this.data;
    const summary = this.data.summary;

    if (messages.length === 0) {
      return html`<div class="empty-state"><div class="title">No messages yet</div><div class="subtitle">Start a conversation to see messages here.</div></div>`;
    }

    // Group messages by task_id for boundary markers
    const taskCosts = new Map<string, number>();
    for (const t of summary.tasks) {
      taskCosts.set(t.id, t.sats_spent);
    }

    let lastTaskId: string | null = null;
    const items: any[] = [];

    for (const msg of messages) {
      const taskId = msg.task_id || null;
      if (taskId && taskId !== lastTaskId && lastTaskId !== null) {
        const cost = taskCosts.get(lastTaskId) || 0;
        items.push({ type: 'boundary', taskId: lastTaskId, cost });
      }
      lastTaskId = taskId;
      items.push({ type: 'message', msg });
    }

    return html`
      <div class="messages-list">
        ${items.map(item => {
          if (item.type === 'boundary') {
            return html`
              <div class="task-boundary">
                <span class="cost" title="${this.fmtCostAlt(item.cost)}">${this.fmtCost(item.cost)}</span>
              </div>
            `;
          }
          const m: ConversationMessage = item.msg;
          const roleClass = m.role === 'user' ? 'user' : m.role === 'assistant' ? 'assistant' : 'tool';
          return html`
            <div class="msg-row ${roleClass}">
              <div class="msg-role">${m.role}${m.name ? ` (${m.name})` : ''}</div>
              <div class="msg-content">${m.content}</div>
              <div class="msg-meta">
                <span>${formatRelativeTime(m.ts)}</span>
              </div>
            </div>
          `;
        })}
      </div>
    `;
  }

  private renderArtifacts() {
    if (this.artifactsLoading) return renderLoading('Loading artifacts...');
    if (!this.artifacts) return html`<div class="empty-state"><div class="title">No artifacts</div><div class="subtitle">Ask the agent to generate images, files, or documents.</div></div>`;

    const { artifacts, by_type, total } = this.artifacts;
    if (total === 0) {
      return html`<div class="empty-state"><div class="title">No artifacts yet</div><div class="subtitle">Ask the agent to generate something.</div></div>`;
    }

    // Filter
    const types = ['all', ...Object.keys(by_type)];
    const filtered = this.artifactTypeFilter === 'all'
      ? artifacts
      : artifacts.filter(a => a.type === this.artifactTypeFilter);

    // Group by turn
    const turns = new Map<number, ArtifactItem[]>();
    for (const a of filtered) {
      const list = turns.get(a.turn) || [];
      list.push(a);
      turns.set(a.turn, list);
    }

    return html`
      <div class="artifact-filters">
        ${types.map(t => html`
          <button class="filter-pill"
            ?data-active=${this.artifactTypeFilter === t}
            @click=${() => { this.artifactTypeFilter = t; }}>
            ${t === 'all' ? `All (${total})` : `${t} (${by_type[t] || 0})`}
          </button>
        `)}
      </div>
      ${[...turns.entries()].sort((a, b) => a[0] - b[0]).map(([turn, items]) => html`
        <div class="turn-group">
          <div class="turn-header"><strong>Turn ${turn}</strong>${items[0]?.turn_prompt ? `: ${items[0].turn_prompt}` : ''}</div>
          <div class="artifact-grid">
            ${items.map(a => this.renderArtifactCard(a))}
          </div>
        </div>
      `)}
    `;
  }

  private renderArtifactCard(a: ArtifactItem) {
    const isImage = a.type === 'image';
    return html`
      <a class="artifact-card" href="${a.url}" target="_blank" rel="noopener">
        ${isImage ? html`<img src="${a.url}" alt="${a.name}" loading="lazy">` : nothing}
        <div class="info">
          <div class="name">${a.name}</div>
          <div class="meta">${a.type} \u00B7 ${this.formatSize(a.size_bytes)}</div>
        </div>
      </a>
    `;
  }

  private renderAudit() {
    if (!this.data) return nothing;
    const { summary } = this.data;

    return html`
      <div class="audit-summary">
        <div class="stat-card">
          <div class="value" title="${this.fmtCostAlt(this.data.conversation.total_sats)}">${this.fmtCost(this.data.conversation.total_sats)}</div>
          <div class="label">Total Cost</div>
        </div>
        <div class="stat-card">
          <div class="value">${summary.total_proofs}</div>
          <div class="label">Proofs</div>
        </div>
        <div class="stat-card">
          <div class="value">${summary.models_used.length}</div>
          <div class="label">Models</div>
        </div>
      </div>

      ${summary.models_used.length > 0 ? html`
        <div style="margin-bottom:14px;font-size:12px;color:var(--text-dim)">
          Models: ${summary.models_used.join(', ')}
        </div>
      ` : nothing}

      ${summary.tasks.length === 0
        ? html`<div class="empty-state"><div class="title">No tasks yet</div><div class="subtitle">Submit a message to see audit data.</div></div>`
        : summary.tasks.map(t => this.renderTaskAuditRow(t))
      }
    `;
  }

  private renderTaskAuditRow(t: ConversationTaskDetail) {
    const expanded = this.auditExpanded.has(t.id);
    const loading = this.auditLoading.has(t.id);
    const cached = this.auditCache.get(t.id);

    return html`
      <div style="display:flex;gap:6px;align-items:center;margin-bottom:4px">
        <button class="show-work-btn" ?data-expanded=${expanded} @click=${() => this.toggleAudit(t.id)} style="flex:1">
          <span class="arrow">${expanded ? '\u25BC' : '\u25B6'}</span>
          <span style="flex:1">Show work</span>
          <span style="font-family:var(--mono);font-size:12px;color:var(--text-dim)">
            ${t.iterations} iter \u00B7
            <span title="${this.fmtCostAlt(t.sats_spent)}">${this.fmtCost(t.sats_spent)}</span>
            \u00B7 ${t.proof_count} proof${t.proof_count !== 1 ? 's' : ''}
          </span>
        </button>
        <a href="#task/${t.id}/proofs" class="task-detail-link">Proofs</a>
        <a href="#task/${t.id}" class="task-detail-link">Audit</a>
      </div>
      ${expanded ? html`
        <div class="work-detail">
          ${loading ? html`<div style="padding:8px;color:var(--text-dim)">Loading audit data...</div>` : nothing}
          ${cached ? this.renderAuditEvents(cached) : nothing}
        </div>
      ` : nothing}
    `;
  }

  private renderAuditEvents(cache: TaskAuditCache) {
    const events = cache.events || [];
    if (events.length === 0) return html`<div style="color:var(--text-dim)">No events recorded.</div>`;

    return html`
      ${events.map((e: any) => html`
        <div class="iter-row">
          <span class="iter-badge">${e.event_type}</span>
          <span class="iter-type">${e.data?.model || e.data?.name || e.data?.content?.substring(0, 60) || ''}</span>
          ${e.data?.sats_effective || e.data?.sats_paid
            ? html`<span class="iter-cost" title="${this.fmtCostAlt(e.data.sats_effective || e.data.sats_paid)}">${this.fmtCost(e.data.sats_effective || e.data.sats_paid)}</span>`
            : nothing}
        </div>
      `)}
    `;
  }

  private renderChain() {
    if (this.chainLoading) return renderLoading('Verifying chain...');

    if (!this.chainVerification) {
      return html`<button class="show-work-btn" @click=${() => this.loadChain()}>Verify BRC-60 hash chain</button>`;
    }

    const v = this.chainVerification;
    return html`
      <div class="chain-status ${v.valid ? 'valid' : 'invalid'}">
        ${v.valid ? '\u2713 Chain verified' : '\u2717 Chain broken'} \u2014
        ${v.message_count} messages, head hash ${v.head_hash.substring(0, 12)}...
      </div>
      ${(v.messages || []).map(m => html`
        <div class="chain-msg">
          <span class="check ${m.valid ? 'valid' : 'invalid'}">${m.valid ? '\u2713' : '\u2717'}</span>
          <span style="min-width:30px;font-size:11px;color:var(--text-dim)">#${m.seq}</span>
          <span style="flex:1;font-size:13px;color:var(--text)">${m.role}</span>
          <span class="hash" title="${m.hash}">${m.hash.substring(0, 16)}...</span>
        </div>
      `)}
    `;
  }
}
