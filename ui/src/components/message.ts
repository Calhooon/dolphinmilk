import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';
import { marked } from 'marked';
import DOMPurify from 'dompurify';
import { formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import type { Attachment } from '../lib/ui-types.js';

@customElement('dm-message')
export class WormMessage extends LitElement {
  @property() role = 'user';
  @property() content = '';
  @property({ type: Number }) timestamp = 0;
  @property({ type: Boolean }) pending = false;
  @property({ type: Number }) satsCost = 0;
  @property() currencyMode: CurrencyDisplay = 'usd-first';
  @property({ type: Number }) usdRate = 0;
  @property({ attribute: false }) attachments: Attachment[] = [];
  @state() private _lightboxSrc: string | null = null;

  static styles = css`
    :host {
      display: block;
      margin-bottom: 12px;
    }

    .bubble {
      max-width: 85%;
      padding: 10px 14px;
      border-radius: 16px;
      line-height: 1.5;
      word-wrap: break-word;
      overflow-wrap: break-word;
    }

    .user {
      margin-left: auto;
      background: var(--bg-user, #14A8C4);
      color: var(--text-bright, #fff);
      border-bottom-right-radius: 4px;
    }

    .assistant {
      margin-right: auto;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-bottom-left-radius: 4px;
    }

    .error {
      margin-right: auto;
      background: rgba(253, 84, 84, 0.1);
      border: 1px solid var(--error, #fd5454);
      color: var(--error, #fd5454);
      border-radius: 8px;
    }

    .system {
      margin: 0 auto;
      text-align: center;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 12px;
      padding: 4px 12px;
    }

    /* Markdown content styling */
    .bubble :first-child { margin-top: 0; }
    .bubble :last-child { margin-bottom: 0; }
    .bubble p { margin: 6px 0; }
    .bubble ul, .bubble ol { padding-left: 20px; margin: 6px 0; }
    .bubble h1, .bubble h2, .bubble h3 {
      margin: 12px 0 6px;
      color: var(--text-bright, #fff);
    }
    .bubble h1 { font-size: 1.2em; }
    .bubble h2 { font-size: 1.1em; }
    .bubble h3 { font-size: 1em; }
    .bubble a { color: var(--accent, #14A8C4); }
    .bubble table { border-collapse: collapse; margin: 8px 0; width: 100%; }
    .bubble th, .bubble td {
      border: 1px solid var(--border, #1A3550);
      padding: 4px 8px;
      text-align: left;
    }
    .bubble th { background: var(--bg, #0B1929); }
    .bubble img {
      max-width: 100%;
      border-radius: 8px;
      margin: 8px 0;
      cursor: pointer;
    }
    .bubble video,
    .bubble audio {
      max-width: 100%;
      border-radius: 8px;
      margin: 8px 0;
    }
    .bubble img:hover {
      opacity: 0.9;
    }

    .msg-attachments {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin-top: 8px;
    }

    .msg-attachments a {
      display: block;
      line-height: 0;
    }

    .msg-attachments img {
      max-width: 300px;
      border-radius: var(--radius-sm, 8px);
      cursor: pointer;
      transition: opacity 0.15s;
    }

    .msg-attachments img:hover {
      opacity: 0.85;
    }

    /* Lightbox overlay for fullscreen image viewing */
    .lightbox {
      position: fixed;
      inset: 0;
      z-index: 99999;
      background: rgba(0, 0, 0, 0.88);
      display: flex;
      align-items: center;
      justify-content: center;
      cursor: zoom-out;
      animation: lb-fade-in 0.15s ease;
    }
    @keyframes lb-fade-in {
      from { opacity: 0; }
      to { opacity: 1; }
    }
    .lightbox img {
      max-width: 92vw;
      max-height: 90vh;
      border-radius: var(--radius-md, 14px);
      object-fit: contain;
      box-shadow: 0 8px 48px rgba(0, 0, 0, 0.6);
    }
    .lightbox-close {
      position: absolute;
      top: 16px;
      right: 20px;
      background: rgba(255,255,255,0.15);
      border: none;
      color: #fff;
      font-size: 28px;
      width: 40px;
      height: 40px;
      border-radius: 50%;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      transition: background 0.15s;
    }
    .lightbox-close:hover { background: rgba(255,255,255,0.3); }

    @media (max-width: 639px) {
      .bubble { max-width: 92%; padding: 8px 12px; font-size: 14px; }
      .msg-attachments img { max-width: 200px; }
    }

    /* Cost stamp — postage on assistant messages */
    .msg-footer {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-top: 2px;
      padding: 0 4px;
      min-height: 16px;
    }

    .msg-footer.user-footer { justify-content: flex-end; }

    .timestamp {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .cost-stamp {
      font-family: var(--mono, monospace);
      font-size: 11px;
      font-weight: 500;
      opacity: 0.7;
      cursor: default;
      transition: opacity 0.15s ease;
      display: inline-flex;
      align-items: center;
      gap: 3px;
    }

    .cost-stamp:hover { opacity: 1; }

    .cost-stamp.tier-low { color: #00b69b; }
    .cost-stamp.tier-mid { color: #fcbe2d; }
    .cost-stamp.tier-high { color: #fd5454; }

    .pending {
      opacity: 0.6;
      position: relative;
    }

    .pending::after {
      content: '';
      position: absolute;
      bottom: 6px;
      right: 10px;
      width: 8px;
      height: 8px;
      border: 2px solid rgba(255,255,255,0.4);
      border-top-color: transparent;
      border-radius: 50%;
      animation: pending-spin 0.8s linear infinite;
    }

    @keyframes pending-spin {
      to { transform: rotate(360deg); }
    }
  `;

  private renderMarkdown(text: string): string {
    const raw = marked.parse(text) as string;
    let sanitized = DOMPurify.sanitize(raw, {
      ADD_TAGS: ['img', 'video', 'source', 'audio'],
      ADD_ATTR: ['src', 'alt', 'controls', 'poster', 'width', 'height', 'loading', 'type'],
    });
    // Add lazy loading to all images
    sanitized = sanitized.replace(/<img /g, '<img loading="lazy" ');
    // Wrap images in clickable links to open full-size in new tab.
    // Skip images already inside an <a> tag (e.g. from [![alt](img)](link) markdown).
    sanitized = sanitized.replace(
      /(<a\s[^>]*>)?(<img[^>]+src="([^"]+)"[^>]*>)(<\/a>)?/g,
      (_match, openA, imgTag, src, closeA) => {
        if (openA) {
          // Already wrapped in an anchor — return unchanged
          return (openA || '') + imgTag + (closeA || '');
        }
        return `<a href="${src}" target="_blank" rel="noopener">${imgTag}</a>`;
      }
    );
    return sanitized;
  }

  private formatTime(ts: number): string {
    if (!ts) return '';
    // `ts` is milliseconds since epoch. Callers in chat.ts construct it via
    // `(m.ts ?? 0) * 1000` (converting the server's unix-seconds timestamp
    // to ms) before setting `this.timestamp`. Previously this function also
    // multiplied by 1000, producing an ms value ~1000× too large — the
    // resulting Date wrapped into a meaningless modular time-of-day, which
    // is why a 10:25 AM message displayed as "08:24 PM".
    const d = new Date(ts);
    return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
  }

  private costTier(sats: number): string {
    if (sats < 10_000) return 'tier-low';
    if (sats < 50_000) return 'tier-mid';
    return 'tier-high';
  }

  private _closeLightbox = () => { this._lightboxSrc = null; };
  private _boundEsc = (e: KeyboardEvent) => { if (e.key === 'Escape') this._closeLightbox(); };

  connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this._boundEsc);
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._boundEsc);
  }

  render() {
    const isAssistant = this.role === 'assistant';
    const showStamp = isAssistant && this.satsCost > 0;
    const hasAttachments = this.attachments.length > 0;

    return html`
      <div class="bubble ${this.role} ${this.pending ? 'pending' : ''}">
        ${this.role === 'user'
          ? html`${this.content}`
          : html`${unsafeHTML(this.renderMarkdown(this.content))}`}
        ${hasAttachments ? html`
          <div class="msg-attachments">
            ${this.attachments.map(att => {
              const src = att.data
                ? `data:${att.mime_type};base64,${att.data}`
                : att.url ?? '';
              return src ? html`
                <img src="${src}" alt="${att.filename}" loading="lazy"
                  @click=${() => { this._lightboxSrc = src; }} />
              ` : nothing;
            })}
          </div>
        ` : nothing}
      </div>
      ${this._lightboxSrc ? html`
        <div class="lightbox" @click=${this._closeLightbox}>
          <button class="lightbox-close" @click=${this._closeLightbox} title="Close">&times;</button>
          <img src="${this._lightboxSrc}" @click=${(e: Event) => e.stopPropagation()} />
        </div>
      ` : nothing}
      ${this.timestamp || showStamp ? html`
        <div class="msg-footer ${this.role === 'user' ? 'user-footer' : ''}">
          ${this.timestamp ? html`<span class="timestamp">${this.formatTime(this.timestamp)}</span>` : html`<span></span>`}
          ${showStamp ? html`
            <span class="cost-stamp ${this.costTier(this.satsCost)}"
                  title="${formatInlineCurrencyAlt(this.satsCost, this.usdRate, this.currencyMode)}">
              ${formatInlineCurrency(this.satsCost, this.usdRate, this.currencyMode)}
            </span>
          ` : nothing}
        </div>
      ` : nothing}
    `;
  }
}
