import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';

@customElement('dm-snackbar')
export class WormSnackbar extends LitElement {
  @property() type: 'info' | 'warning' | 'error' | 'success' = 'info';
  @property() message = '';
  @property({ type: Number }) duration = 5000;

  private timer: number | null = null;

  static styles = css`
    :host {
      display: block;
      position: fixed;
      bottom: 24px;
      left: 50%;
      transform: translateX(-50%);
      z-index: 200;
      animation: slide-up 0.25s ease;
    }

    @keyframes slide-up {
      from { transform: translateX(-50%) translateY(20px); opacity: 0; }
      to { transform: translateX(-50%) translateY(0); opacity: 1; }
    }

    .snack {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 10px 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      font-size: 13px;
      color: var(--text, #e0e0e8);
      box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
      max-width: 440px;
    }

    .snack.info { border-left: 3px solid var(--accent, #14A8C4); }
    .snack.warning { border-left: 3px solid var(--warning, #fcbe2d); }
    .snack.error { border-left: 3px solid var(--error, #fd5454); }
    .snack.success { border-left: 3px solid var(--success, #00b69b); }

    .icon {
      font-size: 16px;
      flex-shrink: 0;
    }

    .msg {
      flex: 1;
      min-width: 0;
    }

    .close {
      background: none;
      border: none;
      color: var(--text-dim, rgba(255,255,255,0.5));
      cursor: pointer;
      font-size: 16px;
      padding: 0 2px;
      line-height: 1;
      flex-shrink: 0;
    }

    .close:hover { color: var(--text, #e0e0e8); }
  `;

  connectedCallback() {
    super.connectedCallback();
    if (this.duration > 0) {
      this.timer = window.setTimeout(() => this.dismiss(), this.duration);
    }
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this.timer) clearTimeout(this.timer);
  }

  private dismiss() {
    this.dispatchEvent(new CustomEvent('dismiss'));
    this.remove();
  }

  private get icon(): string {
    switch (this.type) {
      case 'error': return '\u274C';
      case 'warning': return '\u26A0\uFE0F';
      case 'success': return '\u2705';
      default: return '\u2139\uFE0F';
    }
  }

  render() {
    return html`
      <div class="snack ${this.type}" role="alert" aria-live="polite" aria-atomic="true">
        <span class="icon">${this.icon}</span>
        <span class="msg">${this.message}</span>
        <button class="close" @click=${this.dismiss}>\u00D7</button>
      </div>
    `;
  }
}
