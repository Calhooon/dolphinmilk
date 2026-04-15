/**
 * Share/export dropdown menu for conversations and tasks.
 * Supports copy link, JSON export, CSV export, and Markdown export.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

@customElement('dm-share-menu')
export class WormShareMenu extends LitElement {
  @property() taskId = '';
  @property() sessionId = '';
  @property({ type: Boolean }) open = false;
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);

  @state() private copied = false;
  @state() private exporting: string | null = null;

  private _boundClickOutside: ((e: MouseEvent) => void) | null = null;

  static styles = css`
    :host {
      position: relative;
      display: inline-block;
    }

    .menu {
      position: absolute;
      right: 0;
      top: 100%;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-sm, 8px);
      box-shadow: 0 8px 24px rgba(0, 0, 0, 0.3);
      min-width: 200px;
      padding: 4px 0;
      z-index: 50;
      animation: menu-in 0.1s ease;
    }

    @keyframes menu-in {
      from { opacity: 0; transform: translateY(-4px); }
      to { opacity: 1; transform: translateY(0); }
    }

    @media (prefers-reduced-motion: reduce) {
      .menu { animation: none; }
    }

    .menu-item {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      color: var(--text, #e0e0e8);
      font-size: 13px;
      font-family: var(--sans, sans-serif);
      cursor: pointer;
      transition: background 0.1s;
      border: none;
      background: none;
      width: 100%;
      text-align: left;
    }

    .menu-item:hover {
      background: rgba(255, 255, 255, 0.05);
    }

    .menu-item:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: -2px;
      border-radius: 4px;
    }

    .menu-item:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }

    .menu-item:disabled:hover {
      background: none;
    }

    .menu-icon {
      width: 16px;
      text-align: center;
      flex-shrink: 0;
      font-size: 14px;
    }

    .menu-label {
      flex: 1;
    }

    .menu-check {
      color: var(--success, #00b69b);
      font-size: 12px;
    }

    .menu-sep {
      height: 1px;
      background: var(--border, #1A3550);
      margin: 4px 0;
    }
  `;

  updated(changed: Map<string, unknown>) {
    if (changed.has('open')) {
      if (this.open) {
        requestAnimationFrame(() => {
          this._addClickOutside();
        });
      } else {
        this._removeClickOutside();
        this.copied = false;
      }
    }
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this._removeClickOutside();
  }

  private _addClickOutside() {
    this._removeClickOutside();
    this._boundClickOutside = (e: MouseEvent) => {
      const path = e.composedPath();
      // Check if click is inside this component or parent share button
      if (!path.includes(this) && !(this.parentElement && path.includes(this.parentElement))) {
        this.dispatchEvent(new CustomEvent('close'));
      }
    };
    document.addEventListener('click', this._boundClickOutside, true);
  }

  private _removeClickOutside() {
    if (this._boundClickOutside) {
      document.removeEventListener('click', this._boundClickOutside, true);
      this._boundClickOutside = null;
    }
  }

  private async _copyLink() {
    const link = this.sessionId
      ? `${window.location.origin}${window.location.pathname}#conversation/${this.sessionId}`
      : `${window.location.origin}${window.location.pathname}#task/${this.taskId}`;
    try {
      await navigator.clipboard.writeText(link);
      this.copied = true;
      setTimeout(() => { this.copied = false; }, 2000);
    } catch {
      this._showSnackbar('error', 'Failed to copy link to clipboard');
    }
  }

  private async _exportAs(format: 'json' | 'csv') {
    if (!this.taskId || this.exporting) return;
    this.exporting = format;
    try {
      const res = await this.fetchFn(`/task/${this.taskId}/audit/export?format=${format}`);
      if (!res.ok) throw new Error(`Server returned ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `task-${this.taskId.slice(0, 8)}.${format}`;
      a.style.display = 'none';
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
      this.dispatchEvent(new CustomEvent('close'));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      this._showSnackbar('error', `Export failed: ${msg}`);
    } finally {
      this.exporting = null;
    }
  }

  private async _exportMarkdown() {
    if (this.exporting) return;
    this.exporting = 'md';
    try {
      // Try to load conversation messages for markdown export
      let mdContent = '';
      if (this.sessionId) {
        const res = await this.fetchFn(`/conversations/${this.sessionId}`);
        if (res.ok) {
          const data = await res.json();
          const messages = data.messages ?? [];
          mdContent = this._formatMessagesAsMarkdown(messages, data.conversation?.title);
        }
      }
      if (!mdContent && this.taskId) {
        // Fallback: use task conversation endpoint
        const res = await this.fetchFn(`/task/${this.taskId}/conversation`);
        if (res.ok) {
          const data = await res.json();
          const messages = data.messages ?? [];
          mdContent = this._formatMessagesAsMarkdown(messages);
        }
      }
      if (!mdContent) {
        mdContent = '# Conversation\n\nNo messages found.\n';
      }

      const blob = new Blob([mdContent], { type: 'text/markdown;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      const filename = this.sessionId
        ? `conversation-${this.sessionId.slice(0, 8)}.md`
        : `task-${this.taskId.slice(0, 8)}.md`;
      a.download = filename;
      a.style.display = 'none';
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
      this.dispatchEvent(new CustomEvent('close'));
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      this._showSnackbar('error', `Markdown export failed: ${msg}`);
    } finally {
      this.exporting = null;
    }
  }

  private _formatMessagesAsMarkdown(
    messages: Array<{ role: string; content: string; tool_calls?: Array<{ function: { name: string; arguments: string } }> }>,
    title?: string,
  ): string {
    const lines: string[] = [];
    lines.push(`# ${title ?? 'Conversation'}`);
    lines.push('');
    lines.push(`*Exported ${new Date().toISOString()}*`);
    lines.push('');
    lines.push('---');
    lines.push('');

    for (const msg of messages) {
      const roleLabel = msg.role === 'user' ? 'User' : msg.role === 'assistant' ? 'Assistant' : msg.role === 'tool' ? 'Tool Result' : msg.role;
      lines.push(`### ${roleLabel}`);
      lines.push('');
      if (msg.content) {
        lines.push(msg.content);
        lines.push('');
      }
      if (msg.tool_calls && msg.tool_calls.length > 0) {
        for (const tc of msg.tool_calls) {
          lines.push(`**Tool Call: \`${tc.function.name}\`**`);
          lines.push('```json');
          try {
            lines.push(JSON.stringify(JSON.parse(tc.function.arguments), null, 2));
          } catch {
            lines.push(tc.function.arguments);
          }
          lines.push('```');
          lines.push('');
        }
      }
      lines.push('---');
      lines.push('');
    }

    return lines.join('\n');
  }

  private _showSnackbar(type: 'info' | 'warning' | 'error' | 'success', message: string) {
    const snack = document.createElement('dm-snackbar') as HTMLElement & { type: string; message: string };
    snack.type = type;
    snack.message = message;
    document.body.appendChild(snack);
  }

  render() {
    if (!this.open) return nothing;

    const hasTask = !!this.taskId;

    return html`
      <div class="menu" role="menu" aria-label="Share and export options">
        <button class="menu-item" role="menuitem" @click=${this._copyLink}>
          <span class="menu-icon">\u{1F4CB}</span>
          <span class="menu-label">${this.copied ? 'Copied!' : 'Copy Link'}</span>
          ${this.copied ? html`<span class="menu-check">\u2713</span>` : nothing}
        </button>

        <div class="menu-sep"></div>

        <button class="menu-item" role="menuitem"
                @click=${() => this._exportAs('json')}
                ?disabled=${!hasTask || this.exporting !== null}>
          <span class="menu-icon">{}</span>
          <span class="menu-label">${this.exporting === 'json' ? 'Exporting...' : 'Export as JSON'}</span>
        </button>

        <button class="menu-item" role="menuitem"
                @click=${() => this._exportAs('csv')}
                ?disabled=${!hasTask || this.exporting !== null}>
          <span class="menu-icon">\u{1F4CA}</span>
          <span class="menu-label">${this.exporting === 'csv' ? 'Exporting...' : 'Export as CSV'}</span>
        </button>

        <button class="menu-item" role="menuitem"
                @click=${this._exportMarkdown}
                ?disabled=${!hasTask && !this.sessionId || this.exporting !== null}>
          <span class="menu-icon">\u{1F517}</span>
          <span class="menu-label">${this.exporting === 'md' ? 'Exporting...' : 'Export as Markdown'}</span>
        </button>
      </div>
    `;
  }
}
