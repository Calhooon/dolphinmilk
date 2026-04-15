import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import type { ProofDetail } from '../lib/shared-types.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import { formatInlineCurrency } from '../lib/usd.js';
import { TYPE_COLORS, TYPE_SHAPES, PROOF_TYPE_LABELS } from '../lib/constants.js';
import { PROOF_LABELS } from '../lib/labels.js';
import './proof-inline.js';

@customElement('dm-proof-chain')
export class WormProofChain extends LitElement {
  @property({ type: Array }) proofs: ProofDetail[] = [];
  @property() currencyMode: CurrencyDisplay = 'usd-first';
  @property({ type: Number }) usdRate = 0;
  @state() private selectedNode: string | null = null;
  @state() private hoveredNode: string | null = null;
  @state() private verifyResults = new Map<string, 'verified' | 'mismatch'>();
  @state() private verifyingAll = false;

  static styles = css`
    :host {
      display: block;
    }

    .chain-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 16px;
    }

    .chain-title {
      font-size: 13px;
      font-weight: 600;
      color: var(--text-bright, #fff);
    }

    .chain-stats {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .verify-all-btn {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text, #e0e0e8);
      padding: 4px 12px;
      border-radius: 8px;
      cursor: pointer;
      font-size: 11px;
    }

    .verify-all-btn:hover {
      border-color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .verify-all-btn.verifying {
      opacity: 0.6;
      cursor: wait;
    }

    /* Chain visualization */
    .chain {
      display: flex;
      flex-wrap: wrap;
      gap: 4px;
      align-items: center;
      margin-bottom: 16px;
    }
    @media (max-width: 639px) {
      .chain {
        flex-wrap: nowrap;
        overflow-x: auto;
        -webkit-overflow-scrolling: touch;
        padding-bottom: 4px;
      }
    }

    .node {
      display: flex;
      align-items: center;
      justify-content: center;
      width: 32px;
      height: 32px;
      border-radius: 6px;
      font-size: 16px;
      cursor: pointer;
      border: 2px solid transparent;
      transition: border-color 0.15s ease, transform 0.15s ease;
      position: relative;
    }

    .node:hover {
      transform: scale(1.15);
      filter: drop-shadow(0 0 4px currentColor);
    }
    @media (prefers-reduced-motion: reduce) {
      .node { transition: none; }
      .node:hover { transform: none; }
    }

    .node.selected {
      border-color: var(--text-bright, #fff);
      transform: scale(1.15);
    }

    .node-tooltip {
      position: absolute;
      bottom: 100%;
      left: 50%;
      transform: translateX(-50%);
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px;
      padding: 6px 10px;
      font-size: 10px;
      color: var(--text, #e0e0e8);
      white-space: nowrap;
      pointer-events: none;
      margin-bottom: 6px;
      z-index: 10;
      line-height: 1.5;
    }
    .node-tooltip .tt-type {
      font-weight: 600;
      color: var(--text-bright, #fff);
    }
    .node-tooltip .tt-hash {
      font-family: var(--mono);
      color: var(--text-dim);
    }
    .node-tooltip .tt-cost {
      color: var(--warning, #fcbe2d);
      font-family: var(--mono);
    }

    .connector {
      display: flex;
      align-items: center;
      width: 20px;
      justify-content: center;
    }

    .connector-line {
      width: 16px;
      height: 2px;
      border-radius: 1px;
      animation: draw-line 0.8s ease-out forwards;
    }

    @keyframes draw-line {
      0% { transform: scaleX(0); }
      100% { transform: scaleX(1); }
    }

    .connector-line.verified {
      background: var(--success, #00b69b);
    }

    .connector-line.broken {
      background: var(--error, #fd5454);
      border-top: 1px dashed var(--error, #fd5454);
      height: 0;
    }

    .connector-line.unknown {
      background: var(--border, #1A3550);
    }

    /* Chain integrity status banner */
    .chain-status {
      padding: 8px 16px;
      border-radius: 14px;
      font-size: 13px;
      margin-bottom: 16px;
      display: flex;
      align-items: center;
      gap: 8px;
      animation: badge-pop 0.3s cubic-bezier(0.34, 1.56, 0.64, 1);
    }
    @keyframes badge-pop {
      0% { transform: scale(0.95); opacity: 0; }
      100% { transform: scale(1); opacity: 1; }
    }
    .chain-status.verified {
      background: rgba(0, 182, 155, 0.1);
      border: 1px solid rgba(0, 182, 155, 0.3);
      color: #00b69b;
    }
    .chain-status.broken {
      background: rgba(253, 84, 84, 0.1);
      border: 1px solid rgba(253, 84, 84, 0.3);
      color: #fd5454;
    }
    .chain-status.unknown {
      background: rgba(255, 255, 255, 0.06);
      border: 1px solid rgba(255, 255, 255, 0.12);
      color: rgba(255,255,255,0.5);
    }

    /* Legend */
    .legend {
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      margin-bottom: 16px;
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .legend-item {
      display: flex;
      align-items: center;
      gap: 4px;
    }

    .legend-shape {
      font-size: 12px;
    }

    /* Detail panel */
    .detail-panel {
      margin-top: 12px;
    }

    .empty {
      text-align: center;
      padding: 24px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 12px;
    }
  `;

  private async verifyAll() {
    if (this.verifyingAll) return;
    this.verifyingAll = true;

    try {
      // Use the server-side batch verify endpoint
      const taskId = this.extractTaskId();
      if (taskId) {
        const res = await fetch(`/task/${taskId}/proofs/verify`);
        if (res.ok) {
          const data = await res.json();
          const results = new Map<string, 'verified' | 'mismatch'>();
          if (data.proofs) {
            for (const p of data.proofs) {
              results.set(p.txid, p.valid ? 'verified' : 'mismatch');
            }
          }
          this.verifyResults = results;
        }
      }
    } catch {
      // Silently fail
    } finally {
      this.verifyingAll = false;
    }
  }

  private extractTaskId(): string | null {
    // Try to get task ID from the current URL hash
    const hash = window.location.hash.slice(1);
    if (hash.startsWith('task/')) {
      const parts = hash.split('/');
      return parts[1] || null;
    }
    return null;
  }

  private getVerifyStatus(txid: string): 'verified' | 'mismatch' | 'unverified' {
    return this.verifyResults.get(txid) ?? 'unverified';
  }

  private getConnectorStatus(prev: ProofDetail, curr: ProofDetail): 'verified' | 'broken' | 'unknown' {
    if (!curr.prev_hash) return 'unknown';
    if (curr.prev_hash === prev.hash) {
      const prevStatus = this.verifyResults.get(prev.txid);
      return prevStatus === 'verified' ? 'verified' : 'unknown';
    }
    return 'broken';
  }

  private getChainIntegrity(sorted: ProofDetail[]): { status: 'verified' | 'broken' | 'unknown'; breaks: number } {
    if (this.verifyResults.size === 0) return { status: 'unknown', breaks: 0 };
    let breaks = 0;
    for (let i = 1; i < sorted.length; i++) {
      if (sorted[i].prev_hash && sorted[i].prev_hash !== sorted[i - 1].hash) {
        breaks++;
      }
    }
    const allVerified = sorted.every(p => this.verifyResults.get(p.txid) === 'verified');
    if (breaks > 0) return { status: 'broken', breaks };
    if (allVerified) return { status: 'verified', breaks: 0 };
    return { status: 'unknown', breaks: 0 };
  }

  render() {
    if (this.proofs.length === 0) {
      return html`<div class="empty">No proofs available</div>`;
    }

    const sorted = [...this.proofs].sort((a, b) => a.timestamp - b.timestamp);
    const selectedProof = sorted.find((p) => p.txid === this.selectedNode);
    const integrity = this.getChainIntegrity(sorted);

    return html`
      <div class="chain-header">
        <div>
          <div class="chain-title">Proof Chain</div>
          <div class="chain-stats">${sorted.length} proofs</div>
        </div>
        <button
          class="verify-all-btn ${this.verifyingAll ? 'verifying' : ''}"
          @click=${this.verifyAll}
        >${this.verifyingAll ? 'Verifying...' : 'Verify All'}</button>
      </div>

      ${this.verifyResults.size > 0 ? html`
        <div class="chain-status ${integrity.status}">
          ${integrity.status === 'verified'
            ? html`<span>\u2713</span> All ${sorted.length} proofs verified \u2014 complete audit trail intact`
            : integrity.status === 'broken'
              ? html`<span>\u2717</span> ${integrity.breaks} gap${integrity.breaks !== 1 ? 's' : ''} found in proof chain \u2014 some records may be missing or altered`
              : html`<span>\u2022</span> ${this.verifyResults.size} of ${sorted.length} proofs checked so far`}
        </div>
      ` : nothing}

      <div class="legend">
        ${Object.entries(TYPE_SHAPES).map(([type, shape]) => html`
          <span class="legend-item">
            <span class="legend-shape" style="color: ${TYPE_COLORS[type] || 'var(--text-dim)'}">${shape}</span>
            ${PROOF_LABELS[type]
              ? html`<dm-tooltip text=${PROOF_LABELS[type].description}>${PROOF_LABELS[type].label}</dm-tooltip>`
              : (PROOF_TYPE_LABELS[type] ?? type)}
          </span>
        `)}
      </div>

      <div class="chain">
        ${sorted.map((proof, i) => {
          const color = TYPE_COLORS[proof.proof_type] || 'var(--text-dim)';
          const shape = TYPE_SHAPES[proof.proof_type] || '\u25CF';
          const isSelected = this.selectedNode === proof.txid;
          const isHovered = this.hoveredNode === proof.txid;

          return html`
            ${i > 0 ? html`
              <div class="connector">
                <div class="connector-line ${this.getConnectorStatus(sorted[i - 1], proof)}"
                     style="animation-delay: ${i * 0.1}s"></div>
              </div>
            ` : nothing}
            <div
              class="node ${isSelected ? 'selected' : ''}"
              style="background: ${color}20; color: ${color}"
              @click=${() => { this.selectedNode = isSelected ? null : proof.txid; }}
              @mouseenter=${() => { this.hoveredNode = proof.txid; }}
              @mouseleave=${() => { this.hoveredNode = null; }}
            >
              ${shape}
              ${isHovered ? html`
                <div class="node-tooltip">
                  <div class="tt-type">${PROOF_TYPE_LABELS[proof.proof_type] ?? proof.proof_type}${proof.iteration != null ? ` (Iter ${proof.iteration})` : ''}</div>
                  <div class="tt-hash">${proof.hash ? `${proof.hash.slice(0, 16)}... (${proof.hash.length} chars)` : 'No hash (state token)'}</div>
                  ${proof.sats_cost ? html`<div class="tt-cost">${formatInlineCurrency(proof.sats_cost, this.usdRate, this.currencyMode)}</div>` : nothing}
                </div>
              ` : nothing}
            </div>
          `;
        })}
      </div>

      ${selectedProof ? html`
        <div class="detail-panel">
          <dm-proof-inline
            .proof=${selectedProof}
            .verifyStatus=${this.getVerifyStatus(selectedProof.txid)}
            .currencyMode=${this.currencyMode}
            .usdRate=${this.usdRate}
          ></dm-proof-inline>
        </div>
      ` : nothing}
    `;
  }
}
