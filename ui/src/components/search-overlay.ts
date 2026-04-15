import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import { formatRelativeTime, truncate, parseTimestamp } from '../lib/util.js';
import type { ConversationSummary, TaskSummary, MemorySearchResult } from '../lib/shared-types.js';

/** A single search result item. */
interface SearchResultItem {
  type: 'conversation' | 'memory' | 'activity' | 'audit';
  icon: string;
  title: string;
  subtitle: string;
  time: string;
  hash: string;
}

const RECENT_SEARCHES_KEY = 'dm-recent-searches';
const MAX_RECENT = 5;

function loadRecentSearches(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_SEARCHES_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.slice(0, MAX_RECENT);
    }
  } catch { /* ignore */ }
  return [];
}

function saveRecentSearch(query: string) {
  try {
    const recent = loadRecentSearches().filter(s => s !== query);
    recent.unshift(query);
    localStorage.setItem(RECENT_SEARCHES_KEY, JSON.stringify(recent.slice(0, MAX_RECENT)));
  } catch { /* ignore */ }
}

@customElement('dm-search-overlay')
export class WormSearchOverlay extends LitElement {
  @property({ type: Boolean, reflect: true }) open = false;
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);

  @state() private query = '';
  @state() private results: SearchResultItem[] = [];
  @state() private loading = false;
  @state() private selectedIndex = -1;
  @state() private recentSearches: string[] = loadRecentSearches();

  @query('.search-input') private inputEl!: HTMLInputElement;

  private searchTimer: number | null = null;
  private abortController: AbortController | null = null;

  static styles = css`
    :host {
      display: none;
    }
    :host([open]) {
      display: block;
      position: fixed;
      inset: 0;
      z-index: 200;
    }

    .backdrop {
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.6);
      backdrop-filter: blur(4px);
    }

    @media (prefers-reduced-motion: reduce) {
      .backdrop {
        backdrop-filter: none;
      }
    }

    .dialog {
      position: fixed;
      top: 20%;
      left: 50%;
      transform: translateX(-50%);
      width: min(600px, 90vw);
      max-height: 60vh;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
      overflow: hidden;
      display: flex;
      flex-direction: column;
    }

    /* Search input header */
    .search-header {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 14px 16px;
      border-bottom: 1px solid var(--border, #1A3550);
      flex-shrink: 0;
    }

    .search-icon {
      color: var(--text-dim, rgba(255, 255, 255, 0.5));
      display: flex;
      align-items: center;
      flex-shrink: 0;
    }

    .search-input {
      flex: 1;
      background: none;
      border: none;
      color: var(--text-bright, #fff);
      font-size: 15px;
      font-family: var(--sans, sans-serif);
      outline: none;
    }

    .search-input::placeholder {
      color: var(--text-dim, rgba(255, 255, 255, 0.5));
    }

    .shortcut-badge {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255, 255, 255, 0.4));
      background: rgba(255, 255, 255, 0.06);
      padding: 2px 8px;
      border-radius: 4px;
      border: 1px solid var(--border, #1A3550);
      flex-shrink: 0;
      white-space: nowrap;
    }

    /* Results area */
    .results {
      flex: 1;
      overflow-y: auto;
      padding: 8px 0;
      min-height: 0;
    }

    .category-label {
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-dim, rgba(255, 255, 255, 0.4));
      padding: 8px 16px 4px;
      display: flex;
      align-items: center;
      gap: 8px;
    }

    .category-label::after {
      content: '';
      flex: 1;
      height: 1px;
      background: var(--border, #1A3550);
    }

    .result-item {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 8px 16px;
      cursor: pointer;
      transition: background 0.1s ease;
      text-decoration: none;
      color: inherit;
    }

    .result-item:hover,
    .result-item.selected {
      background: rgba(255, 255, 255, 0.05);
    }

    .result-item.selected {
      outline: none;
      border-left: 2px solid var(--accent, #14A8C4);
      padding-left: 14px;
    }

    .result-icon {
      font-size: 16px;
      flex-shrink: 0;
      width: 24px;
      text-align: center;
    }

    .result-body {
      flex: 1;
      min-width: 0;
    }

    .result-title {
      font-size: 13px;
      font-weight: 500;
      color: var(--text, rgba(255, 255, 255, 0.8));
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .result-subtitle {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255, 255, 255, 0.5));
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .result-time {
      font-size: 11px;
      color: var(--text-dim, rgba(255, 255, 255, 0.4));
      white-space: nowrap;
      flex-shrink: 0;
    }

    /* Footer */
    .search-footer {
      display: flex;
      align-items: center;
      gap: 16px;
      padding: 8px 16px;
      border-top: 1px solid var(--border, #1A3550);
      flex-shrink: 0;
      font-size: 11px;
      color: var(--text-dim, rgba(255, 255, 255, 0.4));
      font-family: var(--mono, monospace);
    }

    .footer-hint {
      display: flex;
      align-items: center;
      gap: 4px;
    }

    .footer-hint kbd {
      background: rgba(255, 255, 255, 0.06);
      border: 1px solid var(--border, #1A3550);
      border-radius: 3px;
      padding: 1px 4px;
      font-size: 10px;
      font-family: var(--mono, monospace);
    }

    /* Empty / loading states */
    .empty-state {
      padding: 24px 16px;
      text-align: center;
      color: var(--text-dim, rgba(255, 255, 255, 0.5));
      font-size: 13px;
    }

    .loading-state {
      padding: 24px 16px;
      text-align: center;
      color: var(--text-dim, rgba(255, 255, 255, 0.5));
      font-size: 13px;
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 8px;
    }

    .spinner {
      display: inline-block;
      width: 14px;
      height: 14px;
      border: 2px solid rgba(255, 255, 255, 0.15);
      border-top-color: var(--accent, #14A8C4);
      border-radius: 50%;
      animation: spin 0.8s linear infinite;
    }

    @keyframes spin {
      to { transform: rotate(360deg); }
    }

    @media (prefers-reduced-motion: reduce) {
      .spinner { animation-duration: 2s; }
    }

    /* Recent searches */
    .recent-header {
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-dim, rgba(255, 255, 255, 0.4));
      padding: 8px 16px 4px;
    }

    .recent-item {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 6px 16px;
      cursor: pointer;
      transition: background 0.1s ease;
      font-size: 13px;
      color: var(--text-dim, rgba(255, 255, 255, 0.6));
    }

    .recent-item:hover,
    .recent-item.selected {
      background: rgba(255, 255, 255, 0.05);
      color: var(--text, rgba(255, 255, 255, 0.8));
    }

    .recent-item.selected {
      border-left: 2px solid var(--accent, #14A8C4);
      padding-left: 14px;
    }

    .recent-icon {
      color: var(--text-dim, rgba(255, 255, 255, 0.3));
      display: flex;
      align-items: center;
      flex-shrink: 0;
    }

    /* Responsive */
    @media (max-width: 639px) {
      .dialog {
        top: 10%;
        max-height: 75vh;
      }
      .shortcut-badge {
        display: none;
      }
      .search-footer {
        display: none;
      }
    }
  `;

  updated(changed: Map<string, unknown>) {
    if (changed.has('open') && this.open) {
      this.query = '';
      this.results = [];
      this.selectedIndex = -1;
      this.loading = false;
      this.recentSearches = loadRecentSearches();
      // Auto-focus after open transition
      requestAnimationFrame(() => {
        this.inputEl?.focus();
      });
    }
  }

  private close() {
    this.dispatchEvent(new CustomEvent('close', { bubbles: true, composed: true }));
  }

  private navigate(hash: string) {
    if (this.query.trim()) {
      saveRecentSearch(this.query.trim());
    }
    this.dispatchEvent(new CustomEvent('navigate', {
      detail: { hash },
      bubbles: true,
      composed: true,
    }));
  }

  private onBackdropClick() {
    this.close();
  }

  private onDialogKeyDown(e: KeyboardEvent) {
    const totalItems = this.query.trim()
      ? this.results.length
      : this.recentSearches.length;

    switch (e.key) {
      case 'Escape':
        e.preventDefault();
        this.close();
        break;
      case 'ArrowDown':
        e.preventDefault();
        if (totalItems > 0) {
          this.selectedIndex = (this.selectedIndex + 1) % totalItems;
        }
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (totalItems > 0) {
          this.selectedIndex = this.selectedIndex <= 0
            ? totalItems - 1
            : this.selectedIndex - 1;
        }
        break;
      case 'Enter':
        e.preventDefault();
        if (this.query.trim() && this.selectedIndex >= 0 && this.selectedIndex < this.results.length) {
          this.navigate(this.results[this.selectedIndex].hash);
        } else if (!this.query.trim() && this.selectedIndex >= 0 && this.selectedIndex < this.recentSearches.length) {
          // Select recent search: populate the query and search
          this.query = this.recentSearches[this.selectedIndex];
          this.selectedIndex = -1;
          this.doSearch();
          if (this.inputEl) {
            this.inputEl.value = this.query;
          }
        }
        break;
      case 'Tab':
        // Trap focus within dialog
        e.preventDefault();
        this.inputEl?.focus();
        break;
    }
  }

  private onInput(e: Event) {
    const input = e.target as HTMLInputElement;
    this.query = input.value;
    this.selectedIndex = -1;

    if (this.searchTimer) {
      clearTimeout(this.searchTimer);
    }

    if (!this.query.trim()) {
      this.results = [];
      this.loading = false;
      return;
    }

    this.loading = true;
    // Debounce 300ms
    this.searchTimer = window.setTimeout(() => this.doSearch(), 300);
  }

  private async doSearch() {
    const q = this.query.trim();
    if (!q) {
      this.results = [];
      this.loading = false;
      return;
    }

    // Cancel previous in-flight requests
    if (this.abortController) {
      this.abortController.abort();
    }
    this.abortController = new AbortController();

    this.loading = true;

    const items: SearchResultItem[] = [];

    // Fetch all sources in parallel
    const [conversations, memory, tasks, audit] = await Promise.allSettled([
      this.fetchConversations(q),
      this.fetchMemory(q),
      this.fetchTasks(q),
      this.fetchAudit(q),
    ]);

    // If the query changed while we were fetching, discard results
    if (q !== this.query.trim()) return;

    if (conversations.status === 'fulfilled') {
      items.push(...conversations.value);
    }
    if (memory.status === 'fulfilled') {
      items.push(...memory.value);
    }
    if (tasks.status === 'fulfilled') {
      items.push(...tasks.value);
    }
    if (audit.status === 'fulfilled') {
      items.push(...audit.value);
    }

    this.results = items;
    this.loading = false;
  }

  private async fetchConversations(q: string): Promise<SearchResultItem[]> {
    try {
      const res = await this.fetchFn('/conversations');
      if (!res.ok) return [];
      const data = await res.json();
      const convs: ConversationSummary[] = Array.isArray(data) ? data : [];
      const lower = q.toLowerCase();
      return convs
        .filter(c => c.title.toLowerCase().includes(lower) || c.id.toLowerCase().includes(lower))
        .slice(0, 5)
        .map(c => ({
          type: 'conversation' as const,
          icon: '\u{1F4AC}',
          title: c.title,
          subtitle: `${c.message_count ?? 0} msgs`,
          time: formatRelativeTime(parseTimestamp(c.updated_at ?? '')),
          hash: `conversation/${c.id}`,
        }));
    } catch {
      return [];
    }
  }

  private async fetchMemory(q: string): Promise<SearchResultItem[]> {
    try {
      const res = await this.fetchFn(`/memory/search?q=${encodeURIComponent(q)}&limit=5`);
      if (!res.ok) return [];
      const data = await res.json();
      const results: MemorySearchResult[] = data.results ?? [];
      return results.map(m => ({
        type: 'memory' as const,
        icon: '\u{1F9E0}',
        title: truncate(m.content, 80),
        subtitle: m.category,
        time: formatRelativeTime(parseTimestamp(m.created ?? '')),
        hash: 'agent/memory',
      }));
    } catch {
      return [];
    }
  }

  private async fetchTasks(q: string): Promise<SearchResultItem[]> {
    try {
      const res = await this.fetchFn('/tasks');
      if (!res.ok) return [];
      const data = await res.json();
      const tasks: TaskSummary[] = Array.isArray(data) ? data : (data.tasks ?? []);
      const lower = q.toLowerCase();
      return tasks
        .filter(t =>
          (t.task ?? '').toLowerCase().includes(lower) ||
          (t.id ?? '').toLowerCase().includes(lower) ||
          (t.result ?? '').toLowerCase().includes(lower)
        )
        .slice(0, 5)
        .map(t => ({
          type: 'activity' as const,
          icon: '\u{1F4CB}',
          title: truncate(t.task ?? t.id, 80),
          subtitle: t.status,
          time: formatRelativeTime(parseTimestamp(t.started_at ?? '')),
          hash: `task/${t.id}`,
        }));
    } catch {
      return [];
    }
  }

  private async fetchAudit(q: string): Promise<SearchResultItem[]> {
    try {
      const res = await this.fetchFn(`/audit/search?q=${encodeURIComponent(q)}`);
      if (!res.ok) return []; // 404 is expected
      const data = await res.json();
      const events: Array<{ snippet?: string; task_id?: string; event_type?: string }> = Array.isArray(data) ? data : (data.results ?? []);
      return events.slice(0, 5).map(ev => ({
        type: 'audit' as const,
        icon: '\u{1F4DC}',
        title: truncate(ev.snippet ?? ev.event_type ?? 'Audit event', 80),
        subtitle: ev.task_id ? `task ${ev.task_id.slice(0, 8)}` : '',
        time: '',
        hash: 'audit-trail',
      }));
    } catch {
      return [];
    }
  }

  private get groupedResults(): Map<string, SearchResultItem[]> {
    const map = new Map<string, SearchResultItem[]>();
    const order = ['conversation', 'memory', 'activity', 'audit'];
    for (const type of order) {
      const items = this.results.filter(r => r.type === type);
      if (items.length > 0) {
        map.set(type, items);
      }
    }
    return map;
  }

  private categoryLabel(type: string): string {
    switch (type) {
      case 'conversation': return 'Conversations';
      case 'memory': return 'Memory';
      case 'activity': return 'Activity';
      case 'audit': return 'Audit';
      default: return type;
    }
  }

  /** Get the flat index of a result item across all groups. */
  private flatIndex(type: string, indexInGroup: number): number {
    let flat = 0;
    for (const [groupType, items] of this.groupedResults) {
      if (groupType === type) return flat + indexInGroup;
      flat += items.length;
    }
    return flat;
  }

  private renderResults() {
    if (this.loading && this.results.length === 0) {
      return html`<div class="loading-state"><span class="spinner"></span> Searching...</div>`;
    }

    if (this.query.trim() && !this.loading && this.results.length === 0) {
      return html`<div class="empty-state">No results for "${truncate(this.query, 40)}"</div>`;
    }

    if (!this.query.trim()) {
      // Show recent searches
      if (this.recentSearches.length === 0) {
        return html`<div class="empty-state">Type to search conversations, memory, tasks, and audit events</div>`;
      }
      return html`
        <div class="recent-header">Recent Searches</div>
        ${this.recentSearches.map((s, i) => html`
          <div
            class="recent-item ${this.selectedIndex === i ? 'selected' : ''}"
            @click=${() => { this.query = s; if (this.inputEl) this.inputEl.value = s; this.selectedIndex = -1; this.doSearch(); }}
          >
            <span class="recent-icon">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="1 4 1 10 7 10"/><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"/>
              </svg>
            </span>
            ${s}
          </div>
        `)}
      `;
    }

    const grouped = this.groupedResults;
    let flatIdx = 0;

    return html`
      ${[...grouped.entries()].map(([type, items]) => html`
        <div class="category-label">${this.categoryLabel(type)}</div>
        ${items.map((item, _i) => {
          const idx = flatIdx++;
          return html`
            <div
              class="result-item ${this.selectedIndex === idx ? 'selected' : ''}"
              @click=${() => this.navigate(item.hash)}
              @mouseenter=${() => { this.selectedIndex = idx; }}
            >
              <span class="result-icon">${item.icon}</span>
              <div class="result-body">
                <div class="result-title">${item.title}</div>
                ${item.subtitle ? html`<div class="result-subtitle">${item.subtitle}</div>` : nothing}
              </div>
              ${item.time ? html`<span class="result-time">${item.time}</span>` : nothing}
            </div>
          `;
        })}
      `)}
    `;
  }

  render() {
    if (!this.open) return nothing;

    const isMac = navigator.platform?.includes('Mac') ?? false;
    const shortcutLabel = isMac ? '\u2318K' : 'Ctrl+K';

    return html`
      <div class="backdrop" @click=${this.onBackdropClick}></div>
      <div
        class="dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Search everything"
        @keydown=${this.onDialogKeyDown}
      >
        <div class="search-header">
          <span class="search-icon">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>
            </svg>
          </span>
          <input
            class="search-input"
            type="text"
            placeholder="Search everything..."
            .value=${this.query}
            @input=${this.onInput}
            autocomplete="off"
            spellcheck="false"
          />
          <span class="shortcut-badge">${shortcutLabel}</span>
        </div>

        <div class="results">
          ${this.renderResults()}
        </div>

        <div class="search-footer">
          <span class="footer-hint"><kbd>&uarr;</kbd><kbd>&darr;</kbd> Navigate</span>
          <span class="footer-hint"><kbd>Enter</kbd> Select</span>
          <span class="footer-hint"><kbd>Esc</kbd> Close</span>
        </div>
      </div>
    `;
  }
}
