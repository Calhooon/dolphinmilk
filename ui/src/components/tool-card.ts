import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import type { CurrencyDisplay } from '../lib/storage.js';

/** Keys scanned for the smart argument preview (order = priority). */
const PREVIEW_KEYS = ['prompt', 'query', 'url', 'endpoint', 'message', 'text', 'title', 'description', 'action', 'content'];

@customElement('dm-tool-card')
export class WormToolCard extends LitElement {
  @property() name = '';
  @property() callId = '';
  @property() arguments = '';
  @property() output = '';
  @property({ type: Boolean }) success = true;
  @property({ type: Boolean }) pending = false;
  @property({ type: Boolean }) compact = false;
  @property({ type: Number }) satsCost = 0;
  @property({ type: Number }) durationMs = 0;
  @property() currencyMode: CurrencyDisplay = 'usd-first';
  @property({ type: Number }) usdRate = 0;

  @state() private expanded = false;
  @state() private copied = false;
  @state() private codeExpanded = false;
  @state() private imgError = false;

  static styles = css`
    :host {
      display: block;
    }

    /* ── Card shell ── */
    .card {
      background: var(--tool-bg, #0D1E30);
      border: 1px solid rgba(255, 255, 255, 0.06);
      border-radius: 8px;
      overflow: hidden;
      transition: border-color 0.15s ease, box-shadow 0.15s ease;
    }

    .card:hover:not(.expanded) {
      border-color: rgba(255, 255, 255, 0.1);
    }

    .card.error {
      border-color: rgba(253, 84, 84, 0.3);
    }

    .card.expanded {
      border-color: rgba(255, 255, 255, 0.1);
      box-shadow: 0 1px 6px rgba(0, 0, 0, 0.2);
    }

    /* ── Header — single compact line ── */
    .header {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 7px 12px;
      cursor: pointer;
      user-select: none;
      transition: background 0.12s ease;
    }

    .header:hover {
      background: rgba(255, 255, 255, 0.02);
    }

    .status-dot {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      flex-shrink: 0;
    }

    .status-dot.ok { background: var(--success, #00b69b); }
    .status-dot.fail { background: var(--error, #fd5454); }
    .status-dot.pending {
      background: var(--accent, #14A8C4);
      animation: pulse 1.4s ease-in-out infinite;
    }

    @keyframes pulse {
      0%, 100% { opacity: 1; transform: scale(1); }
      50% { opacity: 0.4; transform: scale(0.85); }
    }

    .name {
      font-family: var(--mono, monospace);
      font-size: 12px;
      font-weight: 600;
      color: var(--accent, #14A8C4);
      white-space: nowrap;
      flex-shrink: 0;
    }

    .preview {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      min-width: 0;
    }

    .arrow {
      color: var(--text-dim, rgba(255,255,255,0.2));
      font-size: 10px;
      flex-shrink: 0;
    }

    .summary {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      min-width: 0;
    }

    .summary.fail { color: var(--error, #fd5454); }

    .spacer { flex: 1; min-width: 4px; }

    .cost {
      font-family: var(--mono, monospace);
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.35));
      white-space: nowrap;
      flex-shrink: 0;
    }

    .chevron {
      font-size: 8px;
      color: var(--text-dim, rgba(255,255,255,0.25));
      transition: transform 0.2s ease;
      flex-shrink: 0;
    }

    .chevron.open { transform: rotate(90deg); }

    /* ── Details (expanded) ── */
    .details {
      border-top: 1px solid rgba(255, 255, 255, 0.05);
      padding: 8px 12px 10px;
      animation: details-in 0.2s ease-out;
    }
    @keyframes details-in {
      from { opacity: 0; transform: translateY(-4px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @media (prefers-reduced-motion: reduce) {
      .details { animation: none; }
    }

    .section + .section {
      margin-top: 8px;
    }

    .section-label {
      font-size: 10px;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.4px;
      color: var(--text-dim, rgba(255,255,255,0.3));
      margin-bottom: 4px;
    }

    .code-block {
      background: var(--bg, #0B1929);
      border: 1px solid rgba(255, 255, 255, 0.04);
      border-radius: 6px;
      padding: 8px 10px;
      font-family: var(--mono, monospace);
      font-size: 11px;
      line-height: 1.45;
      white-space: pre-wrap;
      word-break: break-word;
      max-height: 280px;
      overflow-y: auto;
      color: var(--text-dim, rgba(255,255,255,0.55));
    }

    .code-block.output { color: var(--text, rgba(255,255,255,0.75)); }
    .code-block.fail { color: var(--error, #fd5454); }

    .code-actions {
      display: flex;
      justify-content: flex-end;
      margin-top: 4px;
    }

    /* ── Receipt ── */
    .receipt {
      display: grid;
      grid-template-columns: auto 1fr;
      gap: 2px 10px;
      font-size: 11px;
    }

    .receipt-label { color: var(--text-dim, rgba(255,255,255,0.35)); }

    .receipt-value {
      font-family: var(--mono, monospace);
      color: var(--text, rgba(255,255,255,0.65));
      text-align: right;
    }

    .receipt-value.paid { color: var(--warning, #fcbe2d); font-weight: 600; }

    /* ── Copy button ── */
    .action-btn {
      background: none;
      border: 1px solid rgba(255, 255, 255, 0.08);
      color: var(--text-dim, rgba(255,255,255,0.4));
      cursor: pointer;
      font-size: 10px;
      font-family: var(--mono, monospace);
      padding: 2px 8px;
      border-radius: 4px;
      transition: color 0.12s, border-color 0.12s;
    }

    .action-btn:hover {
      color: var(--accent, #14A8C4);
      border-color: var(--accent, #14A8C4);
    }

    .action-btn.copied {
      color: var(--success, #00b69b);
      border-color: var(--success, #00b69b);
    }

    /* ── Rich preview (inline artifacts) ── */
    .rich-preview {
      border-top: 1px solid rgba(255, 255, 255, 0.05);
      padding: 8px 12px 10px;
      animation: details-in 0.2s ease-out;
    }
    @media (prefers-reduced-motion: reduce) {
      .rich-preview { animation: none; }
    }

    .rich-preview img {
      max-width: 100%;
      border-radius: var(--radius-sm, 8px);
      display: block;
      background: var(--bg, #0B1929);
    }

    .rich-preview-broken {
      display: flex;
      align-items: center;
      justify-content: center;
      height: 80px;
      background: var(--bg, #0B1929);
      border-radius: var(--radius-sm, 8px);
      color: var(--text-dim, rgba(255,255,255,0.3));
      font-size: 12px;
    }

    .rich-preview-actions {
      display: flex;
      gap: 6px;
      margin-top: 6px;
    }

    .rich-file-header {
      display: flex;
      align-items: center;
      gap: 8px;
      margin-bottom: 6px;
    }

    .rich-file-icon {
      font-size: 16px;
      line-height: 1;
    }

    .rich-file-name {
      font-family: var(--mono, monospace);
      font-size: 12px;
      font-weight: 600;
      color: var(--text-bright, #fff);
    }

    .rich-file-meta {
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      margin-left: auto;
      font-family: var(--mono, monospace);
    }

    .rich-code-block {
      background: var(--bg, #0B1929);
      border: 1px solid rgba(255, 255, 255, 0.04);
      border-radius: 6px;
      padding: 8px 10px;
      font-family: var(--mono, monospace);
      font-size: 11px;
      line-height: 1.45;
      white-space: pre-wrap;
      word-break: break-word;
      max-height: 160px;
      overflow: hidden;
      color: var(--text-dim, rgba(255,255,255,0.55));
    }

    .rich-code-block.full {
      max-height: none;
    }

    .uhrp-link {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      background: var(--bg, #0B1929);
      border: 1px solid rgba(255, 255, 255, 0.04);
      border-radius: 6px;
      font-family: var(--mono, monospace);
      font-size: 11px;
      color: var(--accent, #14A8C4);
      word-break: break-all;
    }

    .uhrp-badge {
      font-size: 10px;
      color: var(--success, #00b69b);
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.4px;
      margin-top: 4px;
    }

    .screenshot-path {
      display: flex;
      align-items: center;
      justify-content: center;
      height: 60px;
      background: var(--bg, #0B1929);
      border: 1px dashed rgba(255, 255, 255, 0.1);
      border-radius: 6px;
      color: var(--text-dim, rgba(255,255,255,0.3));
      font-size: 11px;
      font-family: var(--mono, monospace);
    }

    @media (max-width: 639px) {
      .preview { display: none; }
      .summary { max-width: 120px; }
    }
  `;

  private toggle() {
    this.expanded = !this.expanded;
  }

  private async copyOutput() {
    if (!this.output) return;
    try {
      await navigator.clipboard.writeText(this.output);
      this.copied = true;
      setTimeout(() => { this.copied = false; }, 2000);
    } catch { /* clipboard access denied */ }
  }

  private formatJson(s: string): string {
    try {
      return JSON.stringify(JSON.parse(s), null, 2);
    } catch {
      return s;
    }
  }

  /** Extract the most meaningful argument value for the collapsed preview. */
  private getPreview(): string {
    if (!this.arguments) return '';
    try {
      const args = JSON.parse(this.arguments);
      for (const key of PREVIEW_KEYS) {
        if (typeof args[key] === 'string' && args[key].length > 0) {
          return args[key].length > 60 ? args[key].slice(0, 60) + '\u2026' : args[key];
        }
      }
      for (const val of Object.values(args)) {
        if (typeof val === 'string' && val.length > 0) {
          return val.length > 60 ? (val as string).slice(0, 60) + '\u2026' : val as string;
        }
      }
    } catch { /* not JSON */ }
    return '';
  }

  /** Extract a human-readable one-liner from the tool output. */
  private getOutputSummary(): string {
    if (this.pending) return '';
    if (!this.output) return '';

    if (!this.success) {
      try {
        const p = JSON.parse(this.output);
        const msg = p.error || p.message || '';
        if (typeof msg === 'string' && msg.length > 0) return this._trunc(msg, 50);
      } catch { /* not JSON */ }
      return this._trunc(this.output, 50);
    }

    try {
      const o = JSON.parse(this.output);

      // Inbox pattern: { count, messages[] }
      if (typeof o.count === 'number' && 'messages' in o) {
        return o.count === 0 ? 'empty' : `${o.count} message${o.count !== 1 ? 's' : ''}`;
      }
      // Certificate pattern: { certificate_count }
      if ('certificate_count' in o) {
        const n = o.certificate_count ?? 0;
        return n === 0 ? 'no certificates' : `${n} cert${n !== 1 ? 's' : ''}`;
      }
      // Balance pattern: { balance, identity_key }
      if (typeof o.balance === 'number' && typeof o.identity_key === 'string') {
        return `${o.balance.toLocaleString()} sats`;
      }
      // Message field (most common response pattern)
      if (typeof o.message === 'string' && o.message.length > 0) {
        return this._trunc(o.message, 50);
      }
      // Status field
      if (typeof o.status === 'string') return o.status;
      // Array results
      if (Array.isArray(o.results)) {
        return `${o.results.length} result${o.results.length !== 1 ? 's' : ''}`;
      }
      if (Array.isArray(o)) {
        return `${o.length} item${o.length !== 1 ? 's' : ''}`;
      }
      // Generic: first short string value
      for (const v of Object.values(o)) {
        if (typeof v === 'string' && v.length > 0 && v.length <= 60) {
          return this._trunc(v, 50);
        }
      }
    } catch { /* not JSON */ }

    return this._trunc(this.output, 40);
  }

  private _trunc(s: string, max: number): string {
    s = s.replace(/\n/g, ' ').trim();
    return s.length > max ? s.slice(0, max).trimEnd() + '\u2026' : s;
  }

  /** Try to extract receipt details from tool output for paid calls. */
  private getReceipt(): { service?: string; paid?: number; refund?: number; net?: number; duration?: number } | null {
    if (this.satsCost <= 0) return null;
    const receipt: { service?: string; paid?: number; refund?: number; net?: number; duration?: number } = {
      paid: this.satsCost,
    };
    if (this.durationMs > 0) receipt.duration = this.durationMs;
    if (this.output) {
      try {
        const parsed = JSON.parse(this.output);
        if (parsed.service) receipt.service = parsed.service;
        if (typeof parsed.sats_paid === 'number') receipt.paid = parsed.sats_paid;
        if (typeof parsed.sats_refunded === 'number') receipt.refund = parsed.sats_refunded;
        if (typeof parsed.sats_effective === 'number') receipt.net = parsed.sats_effective;
        if (typeof parsed.duration_ms === 'number') receipt.duration = parsed.duration_ms;
      } catch { /* not JSON */ }
    }
    return receipt;
  }

  /** Parsed arguments as an object, or null on failure. */
  private get parsedArgs(): Record<string, unknown> | null {
    if (!this.arguments) return null;
    try {
      const parsed = JSON.parse(this.arguments);
      return typeof parsed === 'object' && parsed !== null ? parsed as Record<string, unknown> : null;
    } catch { return null; }
  }

  /** Detect if this tool call produced a rich artifact. */
  private getArtifactType(): 'image' | 'code' | 'screenshot' | 'upload' | 'none' {
    if (this.name === 'generate_image' && this.output) return 'image';
    if (this.name === 'file_write') return 'code';
    if (this.name === 'browser') {
      const args = this.parsedArgs;
      if (args && args.action === 'screenshot') return 'screenshot';
    }
    if (this.name === 'upload_to_nanostore' && this.output) return 'upload';
    return 'none';
  }

  /** Render a rich inline preview for artifact-producing tools. */
  private renderRichPreview(): ReturnType<typeof html> | typeof nothing {
    const type = this.getArtifactType();
    if (type === 'none') return nothing;

    try {
      switch (type) {
        case 'image': return this.renderImagePreview();
        case 'code': return this.renderCodePreview();
        case 'screenshot': return this.renderScreenshotPreview();
        case 'upload': return this.renderUploadPreview();
        default: return nothing;
      }
    } catch {
      // Graceful fallback — don't render rich preview if parsing fails
      return nothing;
    }
  }

  private renderImagePreview(): ReturnType<typeof html> | typeof nothing {
    let url = '';
    try {
      const parsed = JSON.parse(this.output);
      url = parsed.url || parsed.image_url || '';
    } catch {
      return nothing;
    }
    if (!url) return nothing;

    return html`
      <div class="rich-preview">
        ${this.imgError ? html`
          <div class="rich-preview-broken">Image unavailable</div>
        ` : html`
          <img src="${url}" alt="Generated image" loading="lazy"
               @error=${() => { this.imgError = true; }} />
        `}
        <div class="rich-preview-actions">
          <a class="action-btn" href="${url}" target="_blank" rel="noopener"
             style="text-decoration:none">Open Full Size</a>
          <button class="action-btn ${this.copied ? 'copied' : ''}"
                  @click=${() => this._copyToClipboard(url)}>
            ${this.copied ? '\u2713 Copied' : 'Copy URL'}
          </button>
        </div>
      </div>
    `;
  }

  private renderCodePreview(): ReturnType<typeof html> | typeof nothing {
    const args = this.parsedArgs;
    if (!args) return nothing;
    const path = typeof args.path === 'string' ? args.path : '';
    const content = typeof args.content === 'string' ? args.content : '';
    if (!content) return nothing;

    const fileName = path ? path.split('/').pop() || path : 'file';
    const ext = fileName.includes('.') ? fileName.split('.').pop()!.toLowerCase() : '';
    const lines = content.split('\n');
    const previewLines = this.codeExpanded ? lines : lines.slice(0, 8);
    const displayText = this._escapeHtml(previewLines.join('\n'));
    const hasMore = lines.length > 8;

    return html`
      <div class="rich-preview">
        <div class="rich-file-header">
          <span class="rich-file-icon">${this._fileIcon(ext)}</span>
          <span class="rich-file-name">${fileName}</span>
          <span class="rich-file-meta">${lines.length} lines</span>
        </div>
        <div class="rich-code-block ${this.codeExpanded ? 'full' : ''}">
          <pre style="margin:0"><code>${displayText}</code></pre>
        </div>
        ${hasMore ? html`
          <div class="rich-preview-actions">
            <button class="action-btn" @click=${() => { this.codeExpanded = !this.codeExpanded; }}>
              ${this.codeExpanded ? 'Collapse' : `Expand (${lines.length} lines)`}
            </button>
          </div>
        ` : nothing}
      </div>
    `;
  }

  private renderScreenshotPreview(): ReturnType<typeof html> | typeof nothing {
    let filepath = '';
    if (this.output) {
      try {
        const parsed = JSON.parse(this.output);
        filepath = parsed.path || parsed.file || parsed.screenshot || '';
      } catch {
        filepath = this.output.trim();
      }
    }
    if (!filepath) {
      const args = this.parsedArgs;
      filepath = args && typeof args.path === 'string' ? args.path : 'screenshot.png';
    }

    return html`
      <div class="rich-preview">
        <div class="screenshot-path">${filepath}</div>
      </div>
    `;
  }

  private renderUploadPreview(): ReturnType<typeof html> | typeof nothing {
    let uhrpUrl = '';
    try {
      const parsed = JSON.parse(this.output);
      uhrpUrl = parsed.uhrp_url || parsed.url || '';
    } catch {
      return nothing;
    }
    if (!uhrpUrl) return nothing;

    return html`
      <div class="rich-preview">
        <div class="uhrp-link">${uhrpUrl}</div>
        <div class="uhrp-badge">Stored permanently on-chain</div>
        <div class="rich-preview-actions">
          <button class="action-btn ${this.copied ? 'copied' : ''}"
                  @click=${() => this._copyToClipboard(uhrpUrl)}>
            ${this.copied ? '\u2713 Copied' : 'Copy UHRP'}
          </button>
        </div>
      </div>
    `;
  }

  private async _copyToClipboard(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      this.copied = true;
      setTimeout(() => { this.copied = false; }, 2000);
    } catch { /* clipboard access denied */ }
  }

  private _escapeHtml(s: string): string {
    return s
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  private _fileIcon(ext: string): string {
    const icons: Record<string, string> = {
      rs: '\u{1F980}', ts: '\u{1F7E6}', js: '\u{1F7E8}', json: '{}',
      toml: '\u2699', yaml: '\u2699', yml: '\u2699', md: '\u{1F4DD}',
      html: '\u{1F310}', css: '\u{1F3A8}', py: '\u{1F40D}', sh: '$',
      svg: '\u25B3', png: '\u{1F5BC}', jpg: '\u{1F5BC}', txt: '\u{1F4C4}',
    };
    return icons[ext] || '\u{1F4C4}';
  }

  render() {
    const statusClass = this.pending ? 'pending' : this.success ? 'ok' : 'fail';
    const preview = this.getPreview();
    const summary = this.getOutputSummary();
    const hasCost = this.satsCost > 0;
    const artifactType = this.getArtifactType();

    return html`
      <div class="card ${!this.success && !this.pending ? 'error' : ''} ${this.expanded ? 'expanded' : ''}">
        <div class="header" @click=${this.toggle}>
          <span class="status-dot ${statusClass}"></span>
          <span class="name">${this.name}</span>
          ${preview ? html`<span class="preview">${preview}</span>` : nothing}
          ${summary ? html`
            <span class="arrow">\u2192</span>
            <span class="summary ${!this.success ? 'fail' : ''}">${summary}</span>
          ` : nothing}
          <span class="spacer"></span>
          ${hasCost ? html`
            <span class="cost" title="${formatInlineCurrencyAlt(this.satsCost, this.usdRate, this.currencyMode)}">${formatInlineCurrency(this.satsCost, this.usdRate, this.currencyMode)}</span>
          ` : nothing}
          <span class="chevron ${this.expanded ? 'open' : ''}">\u25B6</span>
        </div>
        ${artifactType !== 'none' ? this.renderRichPreview() : nothing}
        ${this.expanded ? html`
          <div class="details">
            ${this.arguments ? html`
              <div class="section">
                <div class="section-label">Arguments</div>
                <div class="code-block">${this.formatJson(this.arguments)}</div>
              </div>
            ` : nothing}
            ${this.output ? html`
              <div class="section">
                <div class="section-label">Result</div>
                <div class="code-block output ${!this.success ? 'fail' : ''}">${this.formatJson(this.output)}</div>
                <div class="code-actions">
                  <button class="action-btn ${this.copied ? 'copied' : ''}" @click=${this.copyOutput}>
                    ${this.copied ? '\u2713 Copied' : 'Copy'}
                  </button>
                </div>
              </div>
            ` : nothing}
            ${this.renderReceipt()}
          </div>
        ` : nothing}
      </div>
    `;
  }

  private renderReceipt() {
    const receipt = this.getReceipt();
    if (!receipt) return nothing;

    return html`
      <div class="section">
        <div class="section-label">Receipt</div>
        <div class="receipt">
          ${receipt.service ? html`
            <span class="receipt-label">Service</span>
            <span class="receipt-value">${receipt.service}</span>
          ` : nothing}
          ${receipt.paid != null ? html`
            <span class="receipt-label">Paid</span>
            <span class="receipt-value paid">${formatInlineCurrency(receipt.paid, this.usdRate, this.currencyMode)}</span>
          ` : nothing}
          ${receipt.refund != null && receipt.refund > 0 ? html`
            <span class="receipt-label">Refund</span>
            <span class="receipt-value" style="color:var(--success,#00b69b)">${formatInlineCurrency(receipt.refund, this.usdRate, this.currencyMode)}</span>
          ` : nothing}
          ${receipt.net != null ? html`
            <span class="receipt-label">Net</span>
            <span class="receipt-value paid">${formatInlineCurrency(receipt.net, this.usdRate, this.currencyMode)}</span>
          ` : nothing}
          ${receipt.duration != null ? html`
            <span class="receipt-label">Duration</span>
            <span class="receipt-value">${receipt.duration < 1000 ? receipt.duration + 'ms' : (receipt.duration / 1000).toFixed(1) + 's'}</span>
          ` : nothing}
        </div>
      </div>
    `;
  }
}
