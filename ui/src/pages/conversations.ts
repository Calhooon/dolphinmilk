import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, parseTimestamp } from '../lib/util.js';
import { fetchBsvUsdRate, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import { pageHost, centerState, statsBar, badges, pageTitle, searchInput, stateFeedback, renderSmartEmpty, renderLoading, renderError } from '../lib/shared-styles.js';
import type { ConversationSummary, ChainVerification } from '../lib/shared-types.js';
import { FetchController } from '../controllers/fetch-controller.js';

@customElement('dm-conversations')
export class WormConversations extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @state() private usdRate = 0;
  @state() private searchQuery = '';
  @state() private sortBy: 'recent' | 'cost' | 'messages' = 'recent';
  @state() private costFilter: 'all' | 'free' | 'paid' | 'expensive' = 'all';
  @state() private verifyResults = new Map<string, ChainVerification>();
  @state() private verifyLoading = new Set<string>();
  @state() private verifyExpanded = new Set<string>();

  private ctrl = new FetchController<ConversationSummary[]>(this);

  static styles = [pageHost, centerState, statsBar, badges, pageTitle, searchInput, stateFeedback, css`
    .page-header {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 16px;
    }

    .page-title {
      flex: 1;
    }

    .new-chat-btn {
      background: var(--accent, #14A8C4);
      color: #fff;
      border: none;
      padding: 6px 14px;
      border-radius: 8px;
      font-size: 13px;
      font-weight: 500;
      cursor: pointer;
      transition: background 0.15s;
      font-family: var(--sans, sans-serif);
      text-decoration: none;
    }

    .new-chat-btn:hover {
      background: var(--accent-dim, #0E8FA8);
    }

    /* Conversation list */
    .conv-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .conv-card {
      display: flex;
      flex-direction: column;
      gap: 6px;
      padding: 14px 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px;
      cursor: pointer;
      transition: background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease;
      text-decoration: none;
      color: inherit;
    }

    .conv-card:hover {
      background: rgba(255, 255, 255, 0.03);
      border-color: var(--accent-dim, #0E8FA8);
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
    }

    .conv-card:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    .conv-headline {
      font-size: 14px;
      font-weight: 500;
      color: var(--text, #e0e0e8);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .conv-subhead {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      display: flex;
      align-items: center;
      gap: 6px;
    }

    .conv-subhead .sep {
      color: var(--border, #1A3550);
    }

    .conv-meta {
      display: flex;
      align-items: center;
      gap: 10px;
      margin-top: 2px;
    }

    .meta-item {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      white-space: nowrap;
    }

    .meta-item.sats {
      color: var(--warning, #fcbe2d);
    }

    /* Chain verification badges */
    .chain-pill {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 2px 8px;
      border-radius: 999px;
      font-size: 10px;
      font-weight: 600;
      font-family: var(--mono, monospace);
      text-transform: uppercase;
      letter-spacing: 0.3px;
    }

    .chain-pill.valid {
      color: var(--success, #00b69b);
      background: rgba(0, 182, 155, 0.12);
    }

    .chain-pill.invalid {
      color: var(--error, #fd5454);
      background: rgba(253, 84, 84, 0.12);
    }

    .verify-btn {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      background: none;
      border: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      padding: 2px 8px;
      border-radius: 999px;
      cursor: pointer;
      font-size: 10px;
      font-family: var(--mono, monospace);
      transition: border-color 0.15s, color 0.15s;
      white-space: nowrap;
      text-transform: uppercase;
      letter-spacing: 0.3px;
    }

    .verify-btn:hover {
      border-color: var(--success, #00b69b);
      color: var(--success, #00b69b);
    }

    .verify-btn:disabled {
      opacity: 0.4;
      cursor: not-allowed;
    }

    /* Verification detail panel */
    .verify-panel {
      margin-top: 8px;
      padding: 12px;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
    }

    .verify-summary {
      display: flex;
      gap: 16px;
      flex-wrap: wrap;
      margin-bottom: 10px;
      font-size: 12px;
      font-family: var(--mono, monospace);
    }

    .verify-summary-label { color: var(--text-dim, rgba(255,255,255,0.5)); }
    .verify-summary-value { color: var(--text, #e0e0e8); word-break: break-all; }
    .verify-summary-value.copyable { cursor: pointer; transition: color 0.15s; }
    .verify-summary-value.copyable:hover { color: var(--accent, #14A8C4); }

    .verify-messages {
      display: flex;
      flex-direction: column;
      gap: 4px;
    }

    .verify-msg-row {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 4px 8px;
      border-radius: 4px;
      font-size: 11px;
      font-family: var(--mono, monospace);
    }

    .verify-msg-row.valid { background: rgba(0, 182, 155, 0.04); }
    .verify-msg-row.invalid { background: rgba(253, 84, 84, 0.06); }

    .verify-msg-seq { color: var(--text-dim, rgba(255,255,255,0.5)); min-width: 30px; }
    .verify-msg-role { color: var(--accent, #14A8C4); min-width: 60px; }
    .verify-msg-hash { color: var(--text-dim, rgba(255,255,255,0.5)); flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; transition: color 0.15s; }
    .verify-msg-hash:hover { color: var(--accent, #14A8C4); }
    .verify-msg-status { flex-shrink: 0; }
    .verify-msg-status.ok { color: var(--success, #00b69b); }
    .verify-msg-status.fail { color: var(--error, #fd5454); }

    .expand-verify-btn {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      background: none;
      border: none;
      color: var(--text-dim, rgba(255,255,255,0.5));
      padding: 2px 4px;
      cursor: pointer;
      font-size: 10px;
      font-family: var(--mono, monospace);
      transition: color 0.15s ease;
    }
    .expand-verify-btn:hover { color: var(--text, #e0e0e8); }

    .task-link {
      font-family: var(--mono, monospace);
      font-size: 11px;
      color: var(--accent, #14A8C4);
      text-decoration: none;
      padding: 1px 4px;
      border-radius: 3px;
      background: rgba(20, 168, 196, 0.08);
      transition: background 0.15s;
    }
    .task-link:hover {
      background: rgba(20, 168, 196, 0.18);
      text-decoration: underline;
    }

    .verify-explain {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      line-height: 1.5;
      margin: 0 0 10px 0;
      padding: 0;
    }

    /* No results */
    .no-results {
      text-align: center;
      padding: 32px 16px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 13px;
    }

    .filter-row {
      display: flex;
      align-items: center;
      gap: 10px;
      margin-bottom: 12px;
      flex-wrap: wrap;
    }
    .filter-pills {
      display: flex;
      gap: 6px;
      flex: 1;
      flex-wrap: wrap;
    }
    .fpill {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 20px;
      padding: 4px 12px;
      font-size: 12px;
      cursor: pointer;
      color: var(--text-dim);
      font-family: var(--sans, sans-serif);
      transition: background 0.15s, color 0.15s, border-color 0.15s;
    }
    .fpill:hover { border-color: var(--accent); color: var(--text); }
    .fpill.active {
      background: var(--accent, #14A8C4);
      color: #fff;
      border-color: var(--accent);
    }
    .sort-select {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      padding: 4px 8px;
      font-size: 12px;
      color: var(--text);
      font-family: var(--sans, sans-serif);
    }

    @media (max-width: 639px) {
      .conv-card {
        padding: 12px;
      }

      .conv-headline {
        white-space: normal;
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
      }

      .conv-meta {
        flex-wrap: wrap;
        gap: 8px;
      }
    }
  `];

  private get conversations(): ConversationSummary[] {
    return this.ctrl.data ?? [];
  }

  private get filteredConversations(): ConversationSummary[] {
    let list = this.conversations;

    // Text search
    if (this.searchQuery) {
      const q = this.searchQuery.toLowerCase();
      list = list.filter(c =>
        c.title.toLowerCase().includes(q) || c.id.toLowerCase().includes(q)
      );
    }

    // Cost filter
    if (this.costFilter === 'free') list = list.filter(c => (c.total_sats ?? 0) === 0);
    else if (this.costFilter === 'paid') list = list.filter(c => (c.total_sats ?? 0) > 0);
    else if (this.costFilter === 'expensive') list = list.filter(c => (c.total_sats ?? 0) > 100000);

    // Sort
    if (this.sortBy === 'cost') {
      list = [...list].sort((a, b) => (b.total_sats ?? 0) - (a.total_sats ?? 0));
    } else if (this.sortBy === 'messages') {
      list = [...list].sort((a, b) => (b.message_count ?? 0) - (a.message_count ?? 0));
    }
    // 'recent' is default sort from server

    return list;
  }

  firstUpdated() {
    fetchBsvUsdRate().then(r => { this.usdRate = r; });
    this.ctrl.fetch(async () => {
      const res = await this.fetchFn('/conversations');
      if (!res.ok) throw new Error(`Failed to load conversations: ${res.status}`);
      const data = await res.json();
      return Array.isArray(data) ? data : [];
    });
  }

  private async verifyConversation(id: string, e: Event) {
    e.preventDefault();
    e.stopPropagation();
    if (this.verifyLoading.has(id)) return;

    const loading = new Set(this.verifyLoading);
    loading.add(id);
    this.verifyLoading = loading;

    try {
      const res = await this.fetchFn(`/conversations/${id}/verify`);
      if (!res.ok) throw new Error(`Verify failed: ${res.status}`);
      const data: ChainVerification = await res.json();
      const next = new Map(this.verifyResults);
      next.set(id, data);
      this.verifyResults = next;
    } catch (err) {
      // Show error inline
      console.error('Verify failed:', err);
    } finally {
      const done = new Set(this.verifyLoading);
      done.delete(id);
      this.verifyLoading = done;
    }
  }

  private toggleVerifyExpanded(id: string, e: Event) {
    e.preventDefault();
    e.stopPropagation();
    const next = new Set(this.verifyExpanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.verifyExpanded = next;
  }

  private renderVerificationPanel(id: string) {
    const result = this.verifyResults.get(id);
    if (!result) return nothing;

    const expanded = this.verifyExpanded.has(id);

    return html`
      <div class="verify-panel" @click=${(e: Event) => e.stopPropagation()}>
        <p class="verify-explain">
          BRC-60 hash chain verification — each message's hash is computed from (prev_hash + role + content + timestamp).
          A valid chain proves no messages have been tampered with, deleted, or reordered since the conversation began.
        </p>
        <div class="verify-summary">
          <span>
            <span class="verify-summary-label">Genesis:</span>
            <span class="verify-summary-value copyable" title="Click to copy" @click=${(e: Event) => { e.stopPropagation(); navigator.clipboard.writeText(result.genesis_hash ?? ''); }}>${(result.genesis_hash ?? '').substring(0, 12)}...</span>
          </span>
          <span>
            <span class="verify-summary-label">Head:</span>
            <span class="verify-summary-value copyable" title="Click to copy" @click=${(e: Event) => { e.stopPropagation(); navigator.clipboard.writeText(result.head_hash ?? ''); }}>${(result.head_hash ?? '').substring(0, 12)}...</span>
          </span>
          <span>
            <span class="verify-summary-label">Messages:</span>
            <span class="verify-summary-value">${result.message_count ?? 0}</span>
          </span>
          <span class="chain-badge ${result.valid ? 'valid' : 'invalid'}">
            ${result.valid ? '\u2713 Chain Valid' : '\u2717 Chain Broken'}
          </span>
        </div>
        ${(result.messages ?? []).length > 0 ? html`
          <button class="expand-verify-btn" @click=${(e: Event) => this.toggleVerifyExpanded(id, e)}>
            ${expanded ? '\u25BC' : '\u25B6'} Per-message detail (${(result.messages ?? []).length})
          </button>
          ${expanded ? html`
            <div class="verify-messages">
              ${(result.messages ?? []).map(msg => html`
                <div class="verify-msg-row ${msg.valid ? 'valid' : 'invalid'}">
                  <span class="verify-msg-seq">#${msg.seq}</span>
                  <span class="verify-msg-role">${msg.role}</span>
                  <span class="verify-msg-hash" title="Click to copy: ${msg.hash}" @click=${(e: Event) => { e.stopPropagation(); navigator.clipboard.writeText(msg.hash); }}>${msg.hash.substring(0, 12)}...</span>
                  <span class="verify-msg-status ${msg.valid ? 'ok' : 'fail'}">
                    ${msg.valid ? '\u2713' : '\u2717'}
                    ${!msg.prev_hash_valid ? ' (prev_hash)' : ''}
                    ${msg.hash !== msg.expected_hash ? ' (hash)' : ''}
                  </span>
                </div>
              `)}
            </div>
          ` : nothing}
        ` : nothing}
      </div>
    `;
  }

  render() {
    if (this.ctrl.loading) {
      return renderLoading('Loading conversations...');
    }

    if (this.ctrl.hasError) {
      return renderError(this.ctrl.error ?? 'Failed to load conversations', 'Check that the agent server is running.');
    }

    if (this.conversations.length === 0) {
      return renderSmartEmpty({
        icon: '\u{1F4AC}',
        title: 'No conversations yet',
        description: 'Each chat session becomes a conversation. Your agent remembers context across messages and builds a verifiable hash chain for every exchange.',
        action: { label: 'Start your first conversation \u2192', href: '#' },
      });
    }

    const totalSats = this.conversations.reduce((sum, c) => sum + (c.total_sats ?? 0), 0);
    const totalMessages = this.conversations.reduce((sum, c) => sum + (c.message_count ?? 0), 0);
    const filtered = this.filteredConversations;

    return html`
      <div class="page-header">
        <div class="page-title">Conversations</div>
        <a class="new-chat-btn" href="#">New Chat</a>
      </div>
      <div class="stats-bar">
        <div class="stat">
          <span class="stat-value">${this.conversations.length.toLocaleString()}</span>
          <span class="stat-label">conversations</span>
        </div>
        <div class="stat">
          <span class="stat-value">${totalMessages.toLocaleString()}</span>
          <span class="stat-label">messages</span>
        </div>
        <div class="stat">
          <span class="stat-value sats" title="${formatInlineCurrencyAlt(totalSats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(totalSats, this.usdRate, this.currencyMode)}</span>
          <span class="stat-label">spent</span>
        </div>
      </div>
      <div class="search-wrapper">
        <span class="search-icon">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>
          </svg>
        </span>
        <input
          class="search-input"
          type="text"
          placeholder="Search conversations..."
          .value=${this.searchQuery}
          @input=${(e: Event) => { this.searchQuery = (e.target as HTMLInputElement).value; }}
        />
      </div>
      <div class="filter-row">
        <div class="filter-pills">
          ${(['all', 'paid', 'expensive', 'free'] as const).map(f => html`
            <button class="fpill ${this.costFilter === f ? 'active' : ''}"
              @click=${() => { this.costFilter = f; }}>
              ${f === 'all' ? 'All' : f === 'paid' ? 'Has cost' : f === 'expensive' ? '> 100k sats' : 'Free'}
            </button>
          `)}
        </div>
        <select class="sort-select" .value=${this.sortBy}
          @change=${(e: Event) => { this.sortBy = (e.target as HTMLSelectElement).value as any; }}>
          <option value="recent">Recent</option>
          <option value="cost">Highest cost</option>
          <option value="messages">Most messages</option>
        </select>
      </div>
      ${filtered.length === 0
        ? html`<div class="no-results">No conversations matching "${this.searchQuery}"</div>`
        : html`
          <div class="conv-list">
            ${filtered.map((c) => {
              const verifyResult = this.verifyResults.get(c.id);
              const isLoading = this.verifyLoading.has(c.id);
              return html`
                <div>
                  <div class="conv-card" @click=${() => { window.location.hash = `conversation/${c.id}`; }}>
                    <span class="conv-headline" title=${c.title}>${c.title}</span>
                    <span class="conv-subhead">
                      ${c.message_count ?? 0} messages
                      <span class="sep">&middot;</span>
                      ${formatRelativeTime(parseTimestamp(c.updated_at ?? ''))}
                    </span>
                    <div class="conv-meta">
                      ${(c.total_sats ?? 0) > 0
                        ? html`<span class="meta-item sats" title="${formatInlineCurrencyAlt(c.total_sats ?? 0, this.usdRate, this.currencyMode)}">${formatInlineCurrency(c.total_sats ?? 0, this.usdRate, this.currencyMode)}</span>`
                        : nothing}
                      ${(c.task_ids ?? []).length <= 3
                        ? (c.task_ids ?? []).map(tid => html`
                            <a class="task-link" href="#task/${tid}" @click=${(e: Event) => e.stopPropagation()}
                               title=${tid}>${tid.slice(0, 8)}</a>
                          `)
                        : html`<span class="meta-item">${(c.task_ids ?? []).length} tasks</span>`
                      }
                      ${verifyResult
                        ? html`<span class="chain-pill ${verifyResult.valid ? 'valid' : 'invalid'}">
                            ${verifyResult.valid ? '\u2713 Verified' : '\u2717 Broken'}
                          </span>`
                        : html`<button
                            class="verify-btn"
                            ?disabled=${isLoading}
                            @click=${(e: Event) => this.verifyConversation(c.id, e)}
                          >${isLoading ? '...' : 'Verify Chain'}</button>`
                      }
                    </div>
                  </div>
                  ${this.renderVerificationPanel(c.id)}
                </div>
              `;
            })}
          </div>
        `
      }
    `;
  }
}
