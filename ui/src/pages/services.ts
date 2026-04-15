/**
 * Services Browser — x402 service catalog with expandable detail panels.
 * Data sourced from skills/x402/providers/*.md tip files.
 */

import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import {
  pageHost, centerState, pageTitle, stateFeedback, searchInput,
  renderLoading, renderEmpty,
} from '../lib/shared-styles.js';
import type { CurrencyDisplay } from '../lib/storage.js';

/* ------------------------------------------------------------------ */
/* Service catalog — sourced from skills/x402/providers/*.md           */
/* ------------------------------------------------------------------ */

interface Endpoint {
  path: string;
  method?: string;
  cost: string;
  desc: string;
}

interface ServiceEntry {
  key: string;
  name: string;
  tagline: string;
  delivery: 'Synchronous' | 'Async-poll' | 'Two-step';
  icon: string;
  costSummary: string;
  endpoints: Endpoint[];
  example: string;
  constraints: string[];
  chatPrompt: string;
}

const SERVICES: ServiceEntry[] = [
  {
    key: 'banana',
    name: 'Image Generation',
    tagline: 'Generate images in 1K, 2K, or 4K resolution from text prompts.',
    delivery: 'Async-poll',
    icon: '\u{1F5BC}',
    costSummary: '$0.19\u2013$0.38 per image',
    endpoints: [
      { path: '/generate', cost: '~37K sats (1K/2K)', desc: 'Generate image from prompt' },
      { path: '/generate', cost: '~75K sats (4K)', desc: 'Generate 4K image from prompt' },
    ],
    example: `x402_call({
  "service": "banana/generate",
  "parameters": {
    "prompt": "a dolphin swimming through data streams",
    "resolution": "1K"
  }
})`,
    constraints: [
      'resolution must be "1K", "2K", or "4K" \u2014 not pixel dimensions',
      'prompt is required',
      'Poll with full URL: https://nano-banana-pro.x402agency.com/status/{id}',
      'Generation takes ~2 minutes. Poll every 15\u201320s.',
    ],
    chatPrompt: 'Use the banana x402 service to generate an image of ',
  },
  {
    key: 'veo',
    name: 'Video Generation (Veo)',
    tagline: 'Generate videos from text prompts. Expensive \u2014 check budget first.',
    delivery: 'Async-poll',
    icon: '\u{1F3AC}',
    costSummary: '$0.19+/sec of video',
    endpoints: [
      { path: '/generate', cost: '~200K+ sats', desc: 'Generate video from prompt' },
    ],
    example: `x402_call({
  "service": "veo/generate",
  "parameters": {
    "prompt": "a dolphin leaping over blockchain waves",
    "duration": 5
  }
})`,
    constraints: [
      'Very expensive: ~200K+ sats per ~5 seconds',
      'Generation takes 2\u20135 minutes',
      'Poll with full URL: https://veo-3-1-fast.x402agency.com/status/{id}',
    ],
    chatPrompt: 'Use the veo x402 service to generate a short video of ',
  },
  {
    key: 'kling',
    name: 'Video Generation (Kling)',
    tagline: '24+ video model variants. Run discover_endpoints first to see models.',
    delivery: 'Async-poll',
    icon: '\u{1F3A5}',
    costSummary: 'Variable by model',
    endpoints: [
      { path: '/generate', cost: 'Variable', desc: 'Generate video (model required)' },
    ],
    example: `// First: discover_endpoints({"agent": "kling"})
x402_call({
  "service": "kling/generate",
  "parameters": {
    "prompt": "a dolphin exploring an ocean of data",
    "model": "kling-v2-master",
    "duration": 5
  }
})`,
    constraints: [
      '24+ model variants \u2014 always run discover_endpoints first',
      'Generation takes 2\u201310 minutes depending on model',
      'Poll with full URL: https://kling.x402agency.com/status/{id}',
    ],
    chatPrompt: 'Use the kling x402 service to generate a video of ',
  },
  {
    key: 'whisper',
    name: 'Speech-to-Text',
    tagline: 'Transcribe audio files to text with timestamps. Supports mp3, wav, m4a, webm, ogg, flac.',
    delivery: 'Synchronous',
    icon: '\u{1F399}',
    costSummary: '$0.0006/min (~1.3K sats/min)',
    endpoints: [
      { path: '/transcribe', cost: '~1.3K sats/min', desc: 'Transcribe audio to text' },
    ],
    example: `x402_call({
  "service": "whisper/transcribe",
  "parameters": {
    "audio": "<base64-encoded audio>",
    "language": "en"
  }
})`,
    constraints: [
      'audio must be base64-encoded bytes, not a URL',
      'Supported: mp3, wav, m4a, webm, ogg, flac',
      'Large files cost more proportionally',
    ],
    chatPrompt: 'Use the whisper x402 service to transcribe this audio: ',
  },
  {
    key: 'x-research',
    name: 'X/Twitter Search',
    tagline: 'Full-archive search back to 2006. Profiles, threads, trending, single tweets.',
    delivery: 'Synchronous',
    icon: '\u{1F426}',
    costSummary: '$0.006\u2013$0.06 per call',
    endpoints: [
      { path: '/search', cost: '~6K sats', desc: 'Full-archive tweet search' },
      { path: '/profile', cost: '~1.2K sats', desc: 'User profile info' },
      { path: '/thread', cost: '~6K sats', desc: 'Full conversation thread' },
      { path: '/trending', cost: '~600 sats', desc: 'Current trending topics' },
      { path: '/tweet', cost: '~600 sats', desc: 'Single tweet by ID' },
    ],
    example: `x402_call({
  "service": "x-research/search",
  "parameters": {
    "query": "BSV micropayments",
    "sort": "likes",
    "pages": 1,
    "limit": 20
  }
})`,
    constraints: [
      'Each page is a separate payment (~6K sats). Start with pages: 1',
      'query max 1024 chars. Supports from:, since:, #hashtag',
      'sort: "likes", "retweets", "recency", "views", "relevance"',
    ],
    chatPrompt: 'Use the x-research x402 service to search Twitter for ',
  },
  {
    key: 'polymirror',
    name: 'Polymarket Whale Tracking',
    tagline: 'Leaderboards, trader profiles, portfolio monitoring, alert management.',
    delivery: 'Synchronous',
    icon: '\u{1F4CA}',
    costSummary: '$0.001\u2013$0.02 per call',
    endpoints: [
      { path: '/leaderboard', cost: '~33K sats', desc: 'Top traders by profit or volume' },
      { path: '/search', cost: '~33K sats', desc: 'Find traders by username/wallet' },
      { path: '/trader', cost: '~65K sats', desc: 'Deep profile analytics' },
      { path: '/portfolio', cost: '~52K sats', desc: 'Monitor watched wallets' },
      { path: '/subscribe', cost: '~131K sats', desc: 'Telegram alert management' },
      { path: '/history', cost: '~7K sats', desc: 'Past alert records' },
    ],
    example: `x402_call({
  "service": "polymirror/leaderboard",
  "parameters": {
    "category": "OVERALL",
    "time_period": "DAY",
    "order_by": "PNL",
    "limit": 25
  }
})`,
    constraints: [
      'All enum values must be UPPERCASE (e.g. "OVERALL" not "overall")',
      'category: OVERALL, POLITICS, SPORTS, CRYPTO, CULTURE, ECONOMICS, TECH, FINANCE',
      'time_period: DAY, WEEK, MONTH, ALL',
    ],
    chatPrompt: 'Use the polymirror x402 service to show me the top Polymarket traders ',
  },
  {
    key: 'nanostore',
    name: 'Permanent File Storage',
    tagline: 'Content-addressed storage on BSV. Two-step: reserve (paid) then upload (free).',
    delivery: 'Two-step',
    icon: '\u{1F4E6}',
    costSummary: '~730 sats/MB/year',
    endpoints: [
      { path: '/quote', method: 'POST', cost: 'FREE', desc: 'Preview cost before committing' },
      { path: '/upload', method: 'POST', cost: '~10+ sats', desc: 'Reserve storage, get upload URL' },
      { path: '/list', method: 'GET', cost: 'FREE', desc: 'List your uploaded files' },
    ],
    example: `// Or use the recipe tool:
upload_to_nanostore({
  "content": "<html>Hello</html>",
  "content_type": "text/html",
  "retention_minutes": 525600
})`,
    constraints: [
      'fileSize is in BYTES (not KB). Required.',
      'retentionPeriod is in MINUTES. Min 180 (3 hours). 525600 = 1 year.',
      'After /upload you MUST PUT bytes to the uploadURL via curl.',
      'Include ALL requiredHeaders or GCS returns 403.',
    ],
    chatPrompt: 'Use the nanostore x402 service to upload this to permanent storage: ',
  },
  {
    key: '1sat',
    name: 'BSV Inscriptions',
    tagline: 'Inscribe data permanently on the BSV blockchain.',
    delivery: 'Synchronous',
    icon: '\u26D3',
    costSummary: '200+ sats',
    endpoints: [
      { path: '/inscribe', cost: '200+ sats', desc: 'Inscribe data on-chain' },
    ],
    example: `x402_call({
  "service": "1sat/inscribe",
  "parameters": {
    "data": "Hello from dolphin-milk!",
    "contentType": "text/plain"
  }
})`,
    constraints: [
      'data and contentType are both required',
      'Inscription is permanent. There is no undo.',
      'Larger data = higher cost',
    ],
    chatPrompt: 'Use the 1sat x402 service to inscribe this on BSV: ',
  },
  {
    key: 'openai-chat',
    name: 'OpenAI LLM',
    tagline: 'GPT-5 models via x402. Used internally by think() \u2014 call directly for one-off queries.',
    delivery: 'Synchronous',
    icon: '\u{1F916}',
    costSummary: 'Per-token (model-dependent)',
    endpoints: [
      { path: '/chat', cost: 'Per-token', desc: 'Chat completions' },
    ],
    example: `x402_call({
  "service": "openai/chat",
  "parameters": {
    "model": "gpt-5-mini",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 500
  }
})`,
    constraints: [
      'Models: gpt-5-nano, gpt-5-mini, gpt-5.2, o4-mini',
      'Do NOT use bare "gpt-5" \u2014 use specific variant',
      'Reasoning models (gpt-5.2, o4-mini): use max_completion_tokens, not max_tokens',
    ],
    chatPrompt: 'Use the openai x402 service to ask GPT-5: ',
  },
  {
    key: 'claude-chat',
    name: 'Claude LLM',
    tagline: 'Claude models via x402. Native Anthropic Messages API format.',
    delivery: 'Synchronous',
    icon: '\u{1F9E0}',
    costSummary: 'Per-token (model-dependent)',
    endpoints: [
      { path: '/chat', cost: 'Per-token', desc: 'Chat completions (Messages API)' },
    ],
    example: `x402_call({
  "service": "claude/chat",
  "parameters": {
    "model": "claude-sonnet-4-6",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 500
  }
})`,
    constraints: [
      'Models: claude-haiku-4-5, claude-sonnet-4-6, claude-opus-4-6',
      'max_tokens is always required (1\u2013128,000)',
      'System messages go in "system" param, NOT in messages array',
    ],
    chatPrompt: 'Use the claude x402 service to ask Claude: ',
  },
];

/* ------------------------------------------------------------------ */
/* Helpers                                                             */
/* ------------------------------------------------------------------ */

function deliveryColor(d: string): string {
  if (d.includes('Async')) return 'var(--accent, #14A8C4)';
  if (d.includes('Two')) return '#a78bfa';
  return 'var(--success, #00b69b)';
}

function deliveryBg(d: string): string {
  if (d.includes('Async')) return 'rgba(20, 168, 196, 0.12)';
  if (d.includes('Two')) return 'rgba(167, 139, 250, 0.12)';
  return 'rgba(0, 182, 155, 0.12)';
}

/* ------------------------------------------------------------------ */
/* Component                                                           */
/* ------------------------------------------------------------------ */

interface LiveService {
  name: string;
  display_name: string;
  url: string;
  tagline: string;
  category: string;
  description: string;
}

@customElement('dm-services')
export class WormServices extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property() currencyMode: CurrencyDisplay = 'usd-first';

  @state() private searchQuery = '';
  @state() private expandedKey: string | null = null;
  @state() private liveServices: LiveService[] = [];
  @state() private liveLoaded = false;

  static styles = [
    pageHost, centerState, pageTitle, stateFeedback, searchInput,
    css`
      .page-title { margin-bottom: 4px; }

      .page-subtitle {
        font-size: 13px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin-bottom: 20px;
        line-height: 1.4;
      }

      .service-count {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.4));
        margin-bottom: 12px;
        font-family: var(--mono, monospace);
      }

      /* List layout — full-width rows */
      .service-grid {
        display: flex;
        flex-direction: column;
        gap: 6px;
      }

      /* Card */
      .svc {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-sm, 8px);
        overflow: hidden;
        transition: border-color 0.15s ease, box-shadow 0.15s ease;
      }
      .svc:hover {
        border-color: rgba(255,255,255,0.08);
        background: rgba(39, 49, 66, 0.9);
      }
      .svc.open {
        border-color: rgba(20, 168, 196, 0.3);
        box-shadow: 0 4px 16px rgba(0,0,0,0.15);
      }

      /* Card header — single clean row */
      .svc-head {
        display: flex;
        align-items: center;
        gap: 14px;
        padding: 12px 16px;
        cursor: pointer;
        user-select: none;
      }
      .svc-icon {
        font-size: 18px;
        width: 34px;
        height: 34px;
        display: flex;
        align-items: center;
        justify-content: center;
        background: rgba(255,255,255,0.04);
        border-radius: 6px;
        flex-shrink: 0;
      }
      .svc-info {
        flex: 1;
        min-width: 0;
        display: flex;
        align-items: baseline;
        gap: 10px;
      }
      .svc-name {
        font-size: 13px;
        font-weight: 700;
        color: var(--text-bright, #fff);
        white-space: nowrap;
        flex-shrink: 0;
      }
      .svc-tagline {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.4));
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
        min-width: 0;
      }
      .svc-pills {
        display: flex;
        gap: 6px;
        flex-shrink: 0;
        align-items: center;
      }
      .pill {
        font-size: 10px;
        font-weight: 600;
        font-family: var(--mono, monospace);
        padding: 2px 8px;
        border-radius: 999px;
        white-space: nowrap;
      }
      .pill-cost {
        background: rgba(252, 190, 45, 0.1);
        color: var(--warning, #fcbe2d);
      }
      .svc-chevron {
        color: var(--text-dim, rgba(255,255,255,0.3));
        transition: transform 0.2s ease;
        font-size: 12px;
        flex-shrink: 0;
      }
      .svc.open .svc-chevron { transform: rotate(180deg); }

      @media (max-width: 639px) {
        .svc-info { flex-direction: column; gap: 2px; }
        .svc-tagline { white-space: normal; -webkit-line-clamp: 1; }
        .svc-pills { display: none; }
      }

      /* Expanded detail panel */
      .svc-detail {
        border-top: 1px solid var(--border, #1A3550);
        padding: 20px 24px;
        display: grid;
        grid-template-columns: 1fr 1fr;
        gap: 24px;
        animation: detailIn 0.15s ease;
      }
      @keyframes detailIn {
        from { opacity: 0; }
        to { opacity: 1; }
      }
      @media (prefers-reduced-motion: reduce) {
        .svc-detail { animation: none; }
      }
      @media (max-width: 900px) {
        .svc-detail { grid-template-columns: 1fr; }
      }

      .detail-section h3 {
        font-size: 11px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        margin: 0 0 10px;
      }

      /* Endpoints table */
      .ep-table {
        width: 100%;
        border-collapse: collapse;
        font-size: 12px;
      }
      .ep-table th {
        text-align: left;
        font-size: 10px;
        font-weight: 600;
        color: var(--text-dim, rgba(255,255,255,0.4));
        text-transform: uppercase;
        letter-spacing: 0.4px;
        padding: 0 8px 6px 0;
        border-bottom: 1px solid var(--border, #1A3550);
      }
      .ep-table td {
        padding: 6px 8px 6px 0;
        border-bottom: 1px solid rgba(49,61,79,0.4);
        color: var(--text, rgba(255,255,255,0.8));
      }
      .ep-path {
        font-family: var(--mono, monospace);
        color: var(--accent, #14A8C4);
        font-weight: 600;
      }
      .ep-cost {
        font-family: var(--mono, monospace);
        color: var(--warning, #fcbe2d);
        white-space: nowrap;
      }
      .ep-free {
        color: var(--success, #00b69b);
      }

      /* Example code block */
      .code-block {
        background: rgba(0,0,0,0.25);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-sm, 8px);
        padding: 12px 14px;
        font-family: var(--mono, monospace);
        font-size: 11px;
        line-height: 1.5;
        color: var(--text, rgba(255,255,255,0.8));
        white-space: pre-wrap;
        word-break: break-word;
        overflow-x: auto;
        max-height: 220px;
        overflow-y: auto;
      }

      /* Constraints list */
      .constraints {
        list-style: none;
        padding: 0;
        margin: 0;
      }
      .constraints li {
        font-size: 12px;
        color: var(--text-dim, rgba(255,255,255,0.6));
        line-height: 1.5;
        padding: 4px 0;
        padding-left: 16px;
        position: relative;
      }
      .constraints li::before {
        content: '\u2022';
        position: absolute;
        left: 0;
        color: var(--warning, #fcbe2d);
      }

      /* Service key display */
      .service-key-display {
        font-family: var(--mono, monospace);
        font-size: 14px;
        font-weight: 600;
        color: var(--accent, #14A8C4);
        background: rgba(20, 168, 196, 0.08);
        padding: 6px 12px;
        border-radius: 6px;
        display: inline-block;
      }

      /* Static prompt hint */
      .prompt-hint {
        padding: 10px 14px;
        background: rgba(20, 168, 196, 0.06);
        border-left: 3px solid rgba(20, 168, 196, 0.4);
        border-radius: 0 6px 6px 0;
        color: var(--text, rgba(255,255,255,0.8));
        font-size: 13px;
        font-style: italic;
        font-family: var(--sans, sans-serif);
        line-height: 1.4;
      }

      .no-results {
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-size: 13px;
        text-align: center;
        padding: 40px 16px;
      }
    `,
  ];

  connectedCallback() {
    super.connectedCallback();
    this.loadLiveData();
  }

  private async loadLiveData() {
    try {
      // Use plain fetch — this endpoint doesn't require auth
      const res = await fetch('/services/catalog');
      if (res.ok) {
        const data = await res.json();
        this.liveServices = data.services || [];
        this.liveLoaded = true;
      }
    } catch {
      // Fallback to hardcoded catalog — already the default
    }
  }

  private get filtered(): ServiceEntry[] {
    const base = SERVICES;

    // Add any live services not in the hardcoded catalog
    // Match flexibly: "openai" matches "openai-chat", "claude" matches "claude-chat"
    const knownKeys = new Set(SERVICES.map(s => s.key));
    const matchesKnown = (name: string) =>
      knownKeys.has(name) || knownKeys.has(`${name}-chat`) || knownKeys.has(name.replace(/-chat$/, ''));
    const discovered: ServiceEntry[] = this.liveServices
      .filter(ls => !matchesKnown(ls.name))
      .map(ls => ({
        key: ls.name,
        name: ls.display_name || ls.name,
        tagline: ls.tagline || ls.description || 'x402 service',
        delivery: 'Synchronous' as const,
        icon: '\u{1F310}',
        costSummary: 'See manifest',
        endpoints: [{ path: '/', cost: 'Variable', desc: ls.tagline || 'x402 service endpoint' }],
        example: `discover_endpoints({"agent": "${ls.name}"})`,
        constraints: ['Run discover_endpoints first to see pricing and parameters'],
        chatPrompt: `Use the ${ls.name} x402 service to `,
      }));

    const all = [...base, ...discovered];

    if (!this.searchQuery) return all;
    const q = this.searchQuery.toLowerCase();
    return all.filter(s =>
      s.name.toLowerCase().includes(q) ||
      s.key.toLowerCase().includes(q) ||
      s.tagline.toLowerCase().includes(q) ||
      s.endpoints.some(e => e.desc.toLowerCase().includes(q))
    );
  }

  private toggle(key: string) {
    this.expandedKey = this.expandedKey === key ? null : key;
  }


  render() {
    const list = this.filtered;

    return html`
      <h1 class="page-title">Services</h1>
      <p class="page-subtitle">
        x402 services your agent can discover and pay for. Click any service for endpoints, pricing, and examples.
        ${this.liveLoaded ? html`<span style="color: var(--success); font-size: 11px;"> (live from registry)</span>` : nothing}
      </p>

      <div class="search-wrapper" style="margin-bottom: 16px;">
        <input
          class="search-input"
          type="text"
          placeholder="Search services..."
          .value=${this.searchQuery}
          @input=${(e: InputEvent) => { this.searchQuery = (e.target as HTMLInputElement).value; }}
        >
      </div>

      ${list.length === 0
        ? html`<div class="no-results">No services match "${this.searchQuery}"</div>`
        : html`
          <div class="service-count">${list.length} service${list.length !== 1 ? 's' : ''}</div>
          <div class="service-grid">
            ${list.map(svc => this.renderCard(svc))}
          </div>
        `
      }
    `;
  }

  private renderCard(svc: ServiceEntry): TemplateResult {
    const open = this.expandedKey === svc.key;

    return html`
      <div class="svc ${open ? 'open' : ''}">
        <div class="svc-head" @click=${() => this.toggle(svc.key)}>
          <div class="svc-icon">${svc.icon}</div>
          <div class="svc-info">
            <div class="svc-name">${svc.name}</div>
            <div class="svc-tagline">${svc.tagline}</div>
          </div>
          <div class="svc-pills">
            <span class="pill pill-cost">${svc.costSummary}</span>
            <span class="pill" style="background:${deliveryBg(svc.delivery)};color:${deliveryColor(svc.delivery)}">
              ${svc.delivery}
            </span>
          </div>
          <span class="svc-chevron">\u25BE</span>
        </div>
        ${open ? this.renderDetail(svc) : nothing}
      </div>
    `;
  }

  private renderDetail(svc: ServiceEntry): TemplateResult {
    return html`
      <div class="svc-detail">
        <div class="detail-section">
          <h3>Endpoints</h3>
          <table class="ep-table">
            <thead>
              <tr><th>Path</th><th>Cost</th><th>Description</th></tr>
            </thead>
            <tbody>
              ${svc.endpoints.map(ep => html`
                <tr>
                  <td class="ep-path">${svc.key}${ep.path}</td>
                  <td class="${ep.cost === 'FREE' ? 'ep-cost ep-free' : 'ep-cost'}">${ep.cost}</td>
                  <td>${ep.desc}</td>
                </tr>
              `)}
            </tbody>
          </table>

          ${svc.constraints.length > 0 ? html`
            <h3 style="margin-top: 16px;">Key Constraints</h3>
            <ul class="constraints">
              ${svc.constraints.map(c => html`<li>${c}</li>`)}
            </ul>
          ` : nothing}
        </div>

        <div class="detail-section">
          <h3>x402 Service Name</h3>
          <div class="service-key-display">${svc.key}</div>

          <h3 style="margin-top: 16px;">Try it — say something like</h3>
          <div class="prompt-hint">"${svc.chatPrompt}..."</div>

          <h3 style="margin-top: 16px;">API Call</h3>
          <div class="code-block">${svc.example}</div>
        </div>
      </div>
    `;
  }
}
