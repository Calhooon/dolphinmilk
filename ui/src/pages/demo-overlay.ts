import { LitElement, html, css, nothing } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import '../components/stat-ticker.js';

interface ComparisonPoint {
  icon: string;
  text: string;
}

interface ComparisonSide {
  name: string;
  points: ComparisonPoint[];
  stat?: string;
  statSource?: string;
}

interface ComparisonData {
  competitor: ComparisonSide;
  us: ComparisonSide;
  quote: string;
  quoteSource: string;
}

const COMPARISONS: Record<string, ComparisonData> = {
  budgeting: {
    competitor: {
      name: 'OpenClaw',
      points: [
        { icon: '\u2715', text: 'No budget controls' },
        { icon: '\u2715', text: 'No spending limits' },
        { icon: '\u2715', text: 'No real-time visibility' },
      ],
      stat: '$480/mo \u2192 $1,245/mo',
      statSource: '@_avdept, X (34K views)',
    },
    us: {
      name: 'Dolphin Milk',
      points: [
        { icon: '\u2713', text: 'Per-task budget limits' },
        { icon: '\u2713', text: 'Per-service spending caps' },
        { icon: '\u2713', text: 'Real-time budget gauges' },
        { icon: '\u2713', text: 'Automatic pause at limit' },
        { icon: '\u2713', text: 'Certificate-based capability enforcement' },
        { icon: '\u2713', text: 'On-chain kill switch (revocation)' },
      ],
    },
    quote: '40% of agentic AI projects cancelled by 2027 due to unanticipated costs',
    quoteSource: 'Gartner via Deloitte',
  },
  auditability: {
    competitor: {
      name: 'OpenClaw',
      points: [
        { icon: '\u2715', text: 'No audit trails (SEC 17a-4)' },
        { icon: '\u2715', text: 'Cannot reconstruct decisions' },
        { icon: '\u2715', text: '512 vulnerabilities, 8 critical' },
      ],
      stat: '"A security nightmare"',
      statSource: 'Cisco AI Security',
    },
    us: {
      name: 'Dolphin Milk',
      points: [
        { icon: '\u2713', text: 'On-chain cryptographic proofs' },
        { icon: '\u2713', text: 'Every decision reconstructible' },
        { icon: '\u2713', text: 'Unified cross-task timeline' },
        { icon: '\u2713', text: 'Immutable \u2014 WORM compliant' },
        { icon: '\u2713', text: 'Signed tamper-evident audit export' },
        { icon: '\u2713', text: 'WORM compliance mode' },
      ],
    },
    quote: 'No audit trails meeting SEC Rule 17a-4 or FINRA Rule 3110',
    quoteSource: 'Institutional Investor',
  },
  accounting: {
    competitor: {
      name: 'OpenClaw',
      points: [
        { icon: '\u2715', text: 'Log into 20 dashboards' },
        { icon: '\u2715', text: 'Manual aggregation' },
        { icon: '\u2715', text: 'No compliance reports' },
      ],
      stat: '15 hours/week on yaml files',
      statSource: '@_avdept, X (34K views)',
    },
    us: {
      name: 'Dolphin Milk',
      points: [
        { icon: '\u2713', text: 'One-click CSV export' },
        { icon: '\u2713', text: 'Every row has a receipt' },
        { icon: '\u2713', text: 'Sats + USD on every line' },
        { icon: '\u2713', text: 'Auditor-ready format' },
        { icon: '\u2713', text: 'Reports page with trend charts' },
        { icon: '\u2713', text: 'Date range: MTD, QTD, YTD, custom' },
      ],
    },
    quote: 'Lacks segregation of duties and approval workflows... no compliance reporting infrastructure',
    quoteSource: 'Institutional Investor',
  },
};

const STATS = [
  { value: '8.5', prefix: '$', suffix: ' BILLION', label: 'AI Agent Market (2026)', sublabel: '\u2192 $35B by 2030', source: 'Deloitte' },
  { value: '40', prefix: '', suffix: '%', label: 'Agentic AI projects cancelled by 2027', sublabel: 'Due to unanticipated costs', source: 'Gartner via Deloitte' },
  { value: '512', prefix: '', suffix: '', label: 'Vulnerabilities in OpenClaw', sublabel: '8 critical, 26 high severity', source: 'Cisco AI Security' },
  { value: '15', prefix: '$', suffix: 'M', label: 'Portkey Series A (Feb 2026)', sublabel: 'For an \u201CAI control plane\u201D', source: 'Inc42' },
];

@customElement('dm-demo-overlay')
export class WormDemoOverlay extends LitElement {
  @property({ type: String }) pillar = 'budgeting';

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      padding: 40px 24px;
      align-items: center;
    }

    .demo-title {
      font-size: 24px;
      font-weight: 700;
      color: var(--text-bright, #fff);
      margin-bottom: 32px;
      text-align: center;
    }

    /* Stat tickers layout */
    .stats-row {
      display: flex;
      gap: 24px;
      flex-wrap: wrap;
      justify-content: center;
      margin-bottom: 32px;
    }

    /* Comparison grid */
    .comparison-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 24px;
      margin-bottom: 24px;
      max-width: 800px;
      width: 100%;
    }
    @media (max-width: 639px) {
      .comparison-grid { grid-template-columns: 1fr; }
    }
    .comparison-card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      padding: 24px;
    }
    .comparison-card.competitor {
      border-color: rgba(253, 84, 84, 0.3);
    }
    .comparison-card.us {
      border-color: rgba(0, 182, 155, 0.3);
    }
    .comparison-name {
      font-size: 16px;
      font-weight: 700;
      margin-bottom: 16px;
    }
    .competitor .comparison-name { color: var(--text-dim, rgba(255,255,255,0.5)); }
    .us .comparison-name { color: var(--success, #00b69b); }

    .comparison-point {
      display: flex;
      align-items: flex-start;
      gap: 8px;
      margin-bottom: 8px;
      font-size: 14px;
      color: var(--text, #e0e0e8);
    }
    .point-icon.fail { color: var(--error, #fd5454); }
    .point-icon.pass { color: var(--success, #00b69b); }

    .comparison-stat {
      margin-top: 16px;
      padding-top: 12px;
      border-top: 1px solid var(--border, #1A3550);
      font-family: var(--mono, monospace);
      font-size: 15px;
      font-weight: 600;
    }
    .competitor .comparison-stat { color: var(--error, #fd5454); }
    .stat-source {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      opacity: 0.6;
      margin-top: 4px;
      font-style: italic;
    }

    .comparison-quote {
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      padding: 16px 20px;
      text-align: center;
      font-style: italic;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 14px;
      line-height: 1.6;
      max-width: 800px;
      width: 100%;
    }
    .quote-source {
      display: block;
      margin-top: 8px;
      font-size: 12px;
      font-style: normal;
      color: var(--text-dim, rgba(255,255,255,0.5));
      opacity: 0.6;
    }

    /* Nav pills */
    .demo-nav {
      display: flex;
      gap: 8px;
      margin-bottom: 32px;
    }
    .demo-nav-btn {
      padding: 8px 16px;
      border-radius: 6px;
      font-size: 13px;
      font-weight: 500;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      cursor: pointer;
      text-decoration: none;
      transition: all 0.15s ease;
    }
    .demo-nav-btn:hover { color: var(--text, #e0e0e8); }
    .demo-nav-btn.active {
      color: var(--accent, #14A8C4);
      border-color: var(--accent, #14A8C4);
    }
  `;

  render() {
    return html`
      <div class="demo-nav">
        <a class="demo-nav-btn ${this.pillar === 'stats' ? 'active' : ''}" href="#demo/stats">Stats</a>
        <a class="demo-nav-btn ${this.pillar === 'budgeting' ? 'active' : ''}" href="#demo/budgeting">Budgeting</a>
        <a class="demo-nav-btn ${this.pillar === 'auditability' ? 'active' : ''}" href="#demo/auditability">Auditability</a>
        <a class="demo-nav-btn ${this.pillar === 'accounting' ? 'active' : ''}" href="#demo/accounting">Accounting</a>
      </div>

      ${this.pillar === 'stats' ? this.renderStats() : this.renderComparison()}
    `;
  }

  private renderStats() {
    return html`
      <div class="demo-title">The AI Agent Problem</div>
      <div class="stats-row">
        ${STATS.map(s => html`
          <dm-stat-ticker
            value=${s.value}
            prefix=${s.prefix}
            suffix=${s.suffix}
            label=${s.label}
            sublabel=${s.sublabel}
            source=${s.source}
          ></dm-stat-ticker>
        `)}
      </div>
    `;
  }

  private renderComparison() {
    const data = COMPARISONS[this.pillar];
    if (!data) return html`<div>Unknown pillar: ${this.pillar}</div>`;

    const titles: Record<string, string> = {
      budgeting: 'Pillar 1: Budgeting',
      auditability: 'Pillar 2: Auditability',
      accounting: 'Pillar 3: Accounting',
    };

    return html`
      <div class="demo-title">${titles[this.pillar] ?? this.pillar}</div>

      <div class="comparison-grid">
        <div class="comparison-card competitor">
          <div class="comparison-name">${data.competitor.name}</div>
          ${data.competitor.points.map(p => html`
            <div class="comparison-point">
              <span class="point-icon fail">${p.icon}</span>
              <span>${p.text}</span>
            </div>
          `)}
          ${data.competitor.stat ? html`
            <div class="comparison-stat">${data.competitor.stat}</div>
            ${data.competitor.statSource ? html`<div class="stat-source">${data.competitor.statSource}</div>` : nothing}
          ` : nothing}
        </div>

        <div class="comparison-card us">
          <div class="comparison-name">${data.us.name}</div>
          ${data.us.points.map(p => html`
            <div class="comparison-point">
              <span class="point-icon pass">${p.icon}</span>
              <span>${p.text}</span>
            </div>
          `)}
        </div>
      </div>

      <div class="comparison-quote">
        "${data.quote}"
        <span class="quote-source">\u2014 ${data.quoteSource}</span>
      </div>
    `;
  }
}
