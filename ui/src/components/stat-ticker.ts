import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

@customElement('dm-stat-ticker')
export class WormStatTicker extends LitElement {
  @property({ type: String }) value = '';
  @property({ type: String }) prefix = '';
  @property({ type: String }) suffix = '';
  @property({ type: String }) label = '';
  @property({ type: String }) sublabel = '';
  @property({ type: String }) source = '';
  @property({ type: Number }) duration = 1500;
  @property({ type: Boolean, attribute: 'animate' }) shouldAnimate = true;

  @state() private displayValue = '0';

  static styles = css`
    :host {
      display: block;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      padding: 32px 40px;
      text-align: center;
      max-width: 400px;
    }
    .stat-value {
      font-family: var(--sans, sans-serif);
      font-size: 44px;
      font-weight: 800;
      color: var(--text-bright, #fff);
      line-height: 1.1;
    }
    .stat-label {
      font-size: 14px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-top: 8px;
    }
    .stat-sublabel {
      font-size: 13px;
      color: var(--accent, #14A8C4);
      margin-top: 12px;
    }
    .stat-source {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      opacity: 0.5;
      margin-top: 8px;
      font-style: italic;
    }
    @media (max-width: 639px) {
      :host { padding: 20px 24px; max-width: 100%; }
      .stat-value { font-size: 32px; }
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    if (this.shouldAnimate) {
      this.animateValue();
    } else {
      this.displayValue = this.normalizeValue();
    }
  }

  /**
   * Normalize scientific notation (e.g. "1.5e7") to fixed decimal notation
   * so the ticker animation can correctly determine decimal places.
   */
  private normalizeValue(): string {
    const num = Number(this.value);
    if (isNaN(num)) return this.value;
    // If the string representation differs (e.g. "1.5e7" vs "15000000"),
    // convert to fixed notation
    if (/[eE]/.test(this.value)) {
      return num % 1 === 0 ? num.toFixed(0) : num.toPrecision(15).replace(/0+$/, '');
    }
    return this.value;
  }

  private animateValue() {
    const normalized = this.normalizeValue();
    const target = parseFloat(normalized);
    if (isNaN(target)) {
      this.displayValue = this.value;
      return;
    }
    const parts = normalized.split('.');
    const decimals = parts.length > 1 ? parts[1].length : 0;
    const startTime = performance.now();

    const step = (now: number) => {
      const elapsed = now - startTime;
      const progress = Math.min(elapsed / this.duration, 1);
      const eased = 1 - Math.pow(1 - progress, 3);
      this.displayValue = (target * eased).toFixed(decimals);
      if (progress < 1) {
        requestAnimationFrame(step);
      }
    };

    requestAnimationFrame(step);
  }

  render() {
    return html`
      <div class="stat-value">${this.prefix}${this.displayValue}${this.suffix}</div>
      ${this.label ? html`<div class="stat-label">${this.label}</div>` : ''}
      ${this.sublabel ? html`<div class="stat-sublabel">${this.sublabel}</div>` : ''}
      ${this.source ? html`<div class="stat-source">${this.source}</div>` : ''}
    `;
  }
}
