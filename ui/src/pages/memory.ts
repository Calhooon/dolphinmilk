import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, truncate } from '../lib/util.js';
import type {
  MemoryListItem,
  MemoryListResponse,
  MemoryDetail,
  MemorySearchResult,
} from '../lib/shared-types.js';
import { centerState, searchInput, pageHost, pageTitle, tabBar, statsBar, stateFeedback, renderEmpty, renderSmartEmpty, renderLoading, renderError } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';

type CategoryFilter = 'all' | 'knowledge' | 'session' | 'execution';

const CATEGORY_COLORS: Record<string, string> = {
  knowledge: 'var(--accent, #14A8C4)',
  session: 'var(--warning, #fcbe2d)',
  execution: 'var(--success, #00b69b)',
};

@customElement('dm-memory')
export class WormMemory extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);

  @state() private filter: CategoryFilter = 'all';
  @state() private searchQuery = '';
  @state() private searchResults: MemorySearchResult[] | null = null;
  @state() private searchLoading = false;
  @state() private expandedId: string | null = null;
  @state() private expandedDetail: MemoryDetail | null = null;
  @state() private expandedLoading = false;

  private ctrl = new FetchController<MemoryListResponse>(this);
  private searchTimer: number | null = null;

  static styles = [pageHost, centerState, pageTitle, searchInput, tabBar, statsBar, stateFeedback, css`
    .tab-count {
      font-size: 10px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-left: 4px;
    }

    /* Memory cards */
    .card-list {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .memory-card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px;
      padding: 12px 16px;
      cursor: pointer;
      transition: border-color 0.15s ease;
    }
    .memory-card:hover {
      border-color: var(--text-dim, rgba(255,255,255,0.5));
    }
    .memory-card.expanded {
      border-color: var(--accent, #14A8C4);
    }

    .card-header {
      display: flex;
      align-items: center;
      gap: 8px;
      margin-bottom: 6px;
      flex-wrap: wrap;
    }

    .category-badge {
      display: inline-flex;
      align-items: center;
      padding: 1px 8px;
      border-radius: 4px;
      font-size: 10px;
      font-weight: 600;
      font-family: var(--mono, monospace);
      text-transform: uppercase;
      letter-spacing: 0.3px;
    }

    .card-meta {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      display: flex;
      gap: 12px;
      flex-wrap: wrap;
    }

    .card-preview {
      font-size: 13px;
      color: var(--text, #e0e0e8);
      line-height: 1.5;
      margin-top: 6px;
      white-space: pre-wrap;
      word-break: break-word;
    }

    .tag-list {
      display: flex;
      gap: 4px;
      flex-wrap: wrap;
      margin-top: 6px;
    }

    .tag-pill {
      padding: 1px 6px;
      font-size: 10px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 3px;
    }

    /* Expanded detail */
    .detail-content {
      margin-top: 12px;
      padding: 12px;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px;
      font-size: 13px;
      font-family: var(--mono, monospace);
      line-height: 1.6;
      white-space: pre-wrap;
      word-break: break-word;
      color: var(--text, #e0e0e8);
      max-height: 400px;
      overflow-y: auto;
    }

    .search-score {
      font-size: 10px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .empty-state {
      text-align: center;
      padding: 40px 0;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 13px;
    }

    /* Responsive */
    @media (max-width: 639px) {
      :host { padding: 16px 12px; }
      .card-header { flex-direction: column; align-items: flex-start; gap: 4px; }
      .card-meta { gap: 8px; }
    }
  `];

  firstUpdated() {
    this.loadMemories();
  }

  private loadMemories() {
    this.ctrl.fetch(async () => {
      const categoryParam = this.filter !== 'all' ? `&category=${this.filter}` : '';
      const res = await this.fetchFn(`/memory?limit=100&offset=0${categoryParam}`);
      if (!res.ok) throw new Error(`Failed to load memories: ${res.status}`);
      return res.json();
    });
  }

  private get entries(): MemoryListItem[] {
    return this.ctrl.data?.entries ?? [];
  }

  private get categories(): Record<string, number> {
    return this.ctrl.data?.categories ?? {};
  }

  private setFilter(f: CategoryFilter) {
    this.filter = f;
    this.searchQuery = '';
    this.searchResults = null;
    this.expandedId = null;
    this.expandedDetail = null;
    this.loadMemories();
  }

  private onSearchInput(e: Event) {
    const input = e.target as HTMLInputElement;
    this.searchQuery = input.value;

    if (this.searchTimer) {
      clearTimeout(this.searchTimer);
    }

    if (!this.searchQuery.trim()) {
      this.searchResults = null;
      return;
    }

    // Debounce 300ms
    this.searchTimer = window.setTimeout(() => this.doSearch(), 300);
  }

  private async doSearch() {
    if (!this.searchQuery.trim()) {
      this.searchResults = null;
      return;
    }

    this.searchLoading = true;
    try {
      const res = await this.fetchFn(
        `/memory/search?q=${encodeURIComponent(this.searchQuery)}&limit=20`
      );
      if (!res.ok) throw new Error(`Search failed: ${res.status}`);
      const data = await res.json();
      this.searchResults = data.results ?? [];
    } catch {
      this.searchResults = [];
    } finally {
      this.searchLoading = false;
    }
  }

  private async toggleCard(id: string) {
    if (this.expandedId === id) {
      this.expandedId = null;
      this.expandedDetail = null;
      return;
    }

    this.expandedId = id;
    this.expandedDetail = null;
    this.expandedLoading = true;

    try {
      const res = await this.fetchFn(`/memory/${id}`);
      if (!res.ok) throw new Error(`Failed: ${res.status}`);
      this.expandedDetail = await res.json();
    } catch {
      this.expandedDetail = null;
    } finally {
      this.expandedLoading = false;
    }
  }

  private categoryColor(cat: string): string {
    return CATEGORY_COLORS[cat] ?? 'var(--text-dim, rgba(255,255,255,0.5))';
  }

  private totalCount(): number {
    return Object.values(this.categories).reduce((a, b) => a + b, 0);
  }

  private renderCard(item: MemoryListItem) {
    const color = this.categoryColor(item.category);
    const expanded = this.expandedId === item.id;
    const ts = new Date(item.created).getTime() / 1000;

    return html`
      <div
        class="memory-card ${expanded ? 'expanded' : ''}"
        @click=${() => this.toggleCard(item.id)}
      >
        <div class="card-header">
          <span
            class="category-badge"
            style="color: ${color}; background: ${color}18;"
            >${item.category}</span
          >
          <div class="card-meta">
            <span>${formatRelativeTime(ts)}</span>
            ${item.source
              ? html`<span>src: ${truncate(item.source, 24)}</span>`
              : nothing}
          </div>
        </div>
        <div class="card-preview">
          ${truncate(item.content_preview, 200)}
        </div>
        ${(item.tags ?? []).length > 0
          ? html`
              <div class="tag-list">
                ${(item.tags ?? []).map(
                  (t) => html`<span class="tag-pill">${t}</span>`
                )}
              </div>
            `
          : nothing}
        ${expanded ? this.renderExpanded() : nothing}
      </div>
    `;
  }

  private renderSearchCard(item: MemorySearchResult) {
    const color = this.categoryColor(item.category);
    const expanded = this.expandedId === item.id;
    const ts = new Date(item.created).getTime() / 1000;

    return html`
      <div
        class="memory-card ${expanded ? 'expanded' : ''}"
        @click=${() => this.toggleCard(item.id)}
      >
        <div class="card-header">
          <span
            class="category-badge"
            style="color: ${color}; background: ${color}18;"
            >${item.category}</span
          >
          <span class="search-score">score: ${(item.score ?? 0).toFixed(2)}</span>
          <div class="card-meta">
            <span>${formatRelativeTime(ts)}</span>
          </div>
        </div>
        <div class="card-preview">${truncate(item.content, 200)}</div>
        ${(item.tags ?? []).length > 0
          ? html`
              <div class="tag-list">
                ${(item.tags ?? []).map(
                  (t) => html`<span class="tag-pill">${t}</span>`
                )}
              </div>
            `
          : nothing}
        ${expanded ? this.renderExpanded() : nothing}
      </div>
    `;
  }

  private renderExpanded() {
    if (this.expandedLoading) {
      return html`<div class="detail-content">Loading...</div>`;
    }
    if (!this.expandedDetail) {
      return html`<div class="detail-content">Failed to load detail.</div>`;
    }
    return html`<div class="detail-content">${this.expandedDetail.content}</div>`;
  }

  render() {
    if (this.ctrl.loading && this.entries.length === 0) {
      return renderLoading('Loading memories...');
    }

    if (this.ctrl.hasError) {
      return renderError(this.ctrl.error ?? 'Failed to load memories', 'Check that the agent server is running.');
    }

    const total = this.totalCount();

    return html`
      <div class="page-title">Memory Browser</div>

      <div class="stats-bar">
        <div class="stat">
          <span class="stat-value">${total}</span>
          <span class="stat-label">total</span>
        </div>
        ${Object.keys(this.categories).length > 0
          ? Object.entries(this.categories).map(
              ([cat, count]) => html`
                <div class="stat">
                  <span class="stat-value" style="color: ${this.categoryColor(cat)}">${count}</span>
                  <span class="stat-label">${cat}</span>
                </div>
              `
            )
          : total === 0
            ? html`<div class="stat"><span class="stat-value" style="color: var(--text-dim)">--</span><span class="stat-label">no categories</span></div>`
            : nothing
        }
      </div>

      <div class="search-bar">
        <input
          class="search-input"
          type="text"
          placeholder="Search memories..."
          .value=${this.searchQuery}
          @input=${this.onSearchInput}
        />
      </div>

      <div class="tab-bar">
        ${(
          [
            ['all', 'All'],
            ['knowledge', 'Knowledge'],
            ['session', 'Sessions'],
            ['execution', 'Execution'],
          ] as [CategoryFilter, string][]
        ).map(
          ([key, label]) => html`
            <button
              class="tab-btn ${this.filter === key ? 'active' : ''}"
              @click=${() => this.setFilter(key)}
            >
              ${label}
              <span class="tab-count"
                >${key === 'all'
                  ? total
                  : this.categories[key] ?? 0}</span
              >
            </button>
          `
        )}
      </div>

      ${this.searchResults !== null
        ? html`
            ${this.searchLoading
              ? renderLoading('Searching...')
              : this.searchResults.length === 0
                ? renderEmpty(`No results for "${this.searchQuery}"`, 'Try a different search term.')
                : html`
                    <div class="card-list">
                      ${this.searchResults.map((r) =>
                        this.renderSearchCard(r)
                      )}
                    </div>
                  `}
          `
        : html`
            ${this.searchQuery.trim() && !this.searchResults
              ? renderLoading('Searching...')
              : this.entries.length === 0
                ? renderSmartEmpty({
                    icon: '\u{1F9E0}',
                    title: 'Your agent\'s memory is empty',
                    description: 'As your agent works, it stores knowledge for future recall. Memory persists across sessions \u2014 the more you use it, the smarter it gets.',
                  })
                : html`
                    <div class="card-list">
                      ${this.entries.map((e) => this.renderCard(e))}
                    </div>
                  `}
          `}
    `;
  }
}
