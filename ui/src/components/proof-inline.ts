import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { computeProofHash } from '../lib/proof-utils.js';
import type { ProofDetail } from '../lib/shared-types.js';
import { TYPE_COLORS, PROOF_TYPE_LABELS } from '../lib/constants.js';
import { PROOF_LABELS } from '../lib/labels.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import { formatInlineCurrency } from '../lib/usd.js';

@customElement('dm-proof-inline')
export class WormProofInline extends LitElement {
  @property({ type: Object }) proof!: ProofDetail;
  @property() verifyStatus: 'unverified' | 'verified' | 'mismatch' = 'unverified';
  @property() currencyMode: CurrencyDisplay = 'usd-first';
  @property({ type: Number }) usdRate = 0;
  @state() private expanded = false;
  @state() private verifying = false;
  @state() private copied = false;

  static styles = css`
    :host {
      display: block;
      margin: 4px 0;
    }

    .proof {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px;
      font-size: 12px;
      overflow: hidden;
    }

    .proof-header {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      cursor: pointer;
      transition: background 0.15s ease;
    }

    .proof-header:hover {
      background: rgba(255, 255, 255, 0.03);
    }

    .type-badge {
      display: inline-flex;
      align-items: center;
      padding: 2px 8px;
      border-radius: 999px;
      font-size: 10px;
      font-weight: 500;
      font-family: var(--mono, monospace);
      text-transform: uppercase;
      letter-spacing: 0.3px;
      background: rgba(255, 255, 255, 0.06);
    }

    .txid {
      font-family: var(--mono, monospace);
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .chain-link {
      color: var(--accent, #14A8C4);
      text-decoration: none;
      font-size: 12px;
      flex-shrink: 0;
    }

    .chain-link:hover { text-decoration: underline; }

    .verify-badge {
      font-size: 10px;
      padding: 1px 6px;
      border-radius: 4px;
      flex-shrink: 0;
    }

    .verify-badge.verified {
      color: var(--success, #00b69b);
      background: rgba(0, 182, 155, 0.15);
    }

    .verify-badge.mismatch {
      color: var(--error, #fd5454);
      background: rgba(253, 84, 84, 0.15);
    }

    .verify-badge.unverified {
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: rgba(255, 255, 255, 0.06);
    }

    .chevron {
      font-size: 9px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      transition: transform 0.15s;
      flex-shrink: 0;
    }

    .chevron.open { transform: rotate(90deg); }

    .proof-body {
      padding: 10px 12px;
      border-top: 1px solid var(--border, #1A3550);
    }

    .detail-row {
      display: flex;
      gap: 8px;
      margin-bottom: 6px;
      font-size: 11px;
    }

    .detail-label {
      color: var(--text-dim, rgba(255,255,255,0.5));
      width: 70px;
      flex-shrink: 0;
      text-transform: uppercase;
      font-size: 10px;
      letter-spacing: 0.3px;
    }

    .detail-value {
      color: var(--text, #e0e0e8);
      font-family: var(--mono, monospace);
      word-break: break-all;
      flex: 1;
      min-width: 0;
    }

    .actions {
      display: flex;
      gap: 8px;
      margin-top: 8px;
    }

    .action-btn {
      background: none;
      border: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      cursor: pointer;
      font-size: 11px;
      padding: 3px 8px;
      border-radius: 4px;
    }

    .action-btn:hover {
      color: var(--text, #e0e0e8);
      border-color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .action-btn.copied { color: var(--success, #00b69b); border-color: var(--success); }
    .action-btn.verifying { opacity: 0.6; }

    .field-hint {
      font-size: 9px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      font-style: italic;
      margin-top: 2px;
    }
  `;

  private toggle() {
    this.expanded = !this.expanded;
  }

  private async verify() {
    if (!this.proof.proof_data || !this.proof.proof_timestamp) return;
    this.verifying = true;
    try {
      const computed = await computeProofHash(
        this.proof.proof_data,
        this.proof.proof_timestamp,
        this.proof.prev_hash,
      );
      this.verifyStatus = computed === this.proof.hash ? 'verified' : 'mismatch';
    } catch {
      this.verifyStatus = 'mismatch';
    } finally {
      this.verifying = false;
    }
  }

  private async copyTxid() {
    try {
      await navigator.clipboard.writeText(this.proof?.txid ?? '');
      this.copied = true;
      setTimeout(() => { this.copied = false; }, 2000);
    } catch { /* clipboard denied */ }
  }

  render() {
    const p = this.proof;
    if (!p) return nothing;

    const color = TYPE_COLORS[p.proof_type] || 'var(--text-dim)';
    const txid = p.txid ?? '';
    const shortTxid = txid.length > 16 ? `${txid.slice(0, 8)}...${txid.slice(-6)}` : (txid || '(no txid)');

    return html`
      <div class="proof">
        <div class="proof-header" @click=${this.toggle}>
          <span class="chevron ${this.expanded ? 'open' : ''}">\u25B6</span>
          <span class="type-badge" style="color: ${color}">${PROOF_LABELS[p.proof_type]
            ? html`<dm-tooltip text=${PROOF_LABELS[p.proof_type].description}>${PROOF_LABELS[p.proof_type].label}</dm-tooltip>`
            : (PROOF_TYPE_LABELS[p.proof_type] ?? p.proof_type)}</span>
          <span class="txid" title=${txid}>${shortTxid}</span>
          <span class="verify-badge ${this.verifyStatus}">${this.verifyStatus}</span>
        </div>
        ${this.expanded ? html`
          <div class="proof-body">
            <div class="detail-row">
              <span class="detail-label">Txid</span>
              <span class="detail-value">${txid}</span>
            </div>
            <div class="detail-row">
              <span class="detail-label">Hash</span>
              <span class="detail-value">${p.hash ?? ''}</span>
            </div>
            ${p.iteration != null ? html`
              <div class="detail-row">
                <span class="detail-label">Iteration</span>
                <span class="detail-value">${p.iteration}</span>
              </div>
            ` : nothing}
            ${p.sats_cost != null ? html`
              <div class="detail-row">
                <span class="detail-label">Cost</span>
                <span class="detail-value">${formatInlineCurrency(p.sats_cost, this.usdRate, this.currencyMode)}</span>
              </div>
            ` : nothing}
            ${p.prev_hash ? html`
              <div class="detail-row">
                <span class="detail-label">Prev Hash</span>
                <span class="detail-value">${p.prev_hash}<div class="field-hint">Links to previous proof in chain</div></span>
              </div>
            ` : nothing}
            <div class="actions">
              <button class="action-btn ${this.copied ? 'copied' : ''}" @click=${this.copyTxid}>
                ${this.copied ? '\u2713 Copied' : 'Copy txid'}
              </button>
              ${p.proof_data ? html`
                <button class="action-btn ${this.verifying ? 'verifying' : ''}" @click=${this.verify}>
                  ${this.verifying ? 'Verifying...' : 'Verify hash'}
                </button>
              ` : nothing}
              <a class="chain-link" href="https://whatsonchain.com/tx/${txid}" target="_blank" rel="noopener">
                WhatsOnChain \u2197
              </a>
            </div>
          </div>
        ` : nothing}
      </div>
    `;
  }
}
