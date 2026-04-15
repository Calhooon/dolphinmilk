/**
 * Tiny inline SVG sparkline chart — no axes, no labels, just a trend line.
 * Used in dashboard hero cards for at-a-glance spending trends.
 */

import { LitElement, html, css, svg } from 'lit';
import { customElement, property } from 'lit/decorators.js';

@customElement('dm-sparkline')
export class WormSparkline extends LitElement {
  @property({ type: Array }) data: number[] = [];
  @property() color = 'var(--accent, #14A8C4)';
  @property() width = '80px';
  @property() height = '24px';

  static styles = css`
    :host { display: inline-block; vertical-align: middle; }
    svg { display: block; }
    @media (prefers-reduced-motion: reduce) {
      svg { animation: none !important; }
    }
  `;

  render() {
    if (this.data.length < 2) return html``;

    const w = parseInt(this.width, 10) || 80;
    const h = parseInt(this.height, 10) || 24;
    const pad = 1;
    const max = Math.max(...this.data);
    const min = Math.min(...this.data);
    const range = max - min || 1;
    const step = (w - pad * 2) / (this.data.length - 1);

    const points = this.data.map((v, i) => {
      const x = pad + i * step;
      const y = h - pad - ((v - min) / range) * (h - pad * 2);
      return `${x},${y}`;
    });

    const linePath = `M${points.join(' L')}`;
    const fillPath = `${linePath} L${pad + (this.data.length - 1) * step},${h} L${pad},${h} Z`;

    return html`
      <svg width=${this.width} height=${this.height} viewBox="0 0 ${w} ${h}" preserveAspectRatio="none">
        <defs>
          <linearGradient id="spark-fill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color=${this.color} stop-opacity="0.3" />
            <stop offset="100%" stop-color=${this.color} stop-opacity="0.02" />
          </linearGradient>
        </defs>
        ${svg`<path d=${fillPath} fill="url(#spark-fill)" />`}
        ${svg`<path d=${linePath} fill="none" stroke=${this.color} stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />`}
      </svg>
    `;
  }
}
