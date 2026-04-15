/**
 * Proof viewer page -- SHA-256 verification, on-chain verification,
 * checkpoint decryption, and receipt cost table.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime, formatSats } from '../../lib/util.js';
import { computeProofHash, sha256 } from '../../lib/proof-utils.js';
import { fetchBsvUsdRate, formatUsd, formatInlineCurrency, formatInlineCurrencyAlt } from '../../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../../lib/storage.js';
import type { ProofDetail, ProofsResponse, ReceiptDetail, ReceiptsResponse, VerifyStatus, OnChainVerification } from '../../lib/shared-types.js';
import { pageHost, centerState, pageTitle, stateFeedback, renderLoading, renderEmpty, renderBreadcrumb, renderError } from '../../lib/shared-styles.js';
import { FetchController } from '../../controllers/fetch-controller.js';
import { PROOF_TYPE_LABELS } from '../../lib/constants.js';
import '../../components/receipt-table.js';

/** XHR-based fetch that bypasses MetaNet Client's global fetch interceptor. */
function xhrFetch(url: string, init?: RequestInit): Promise<Response> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open(init?.method || 'GET', url);
    if (init?.headers) {
      const h = init.headers as Record<string, string>;
      for (const [k, v] of Object.entries(h)) xhr.setRequestHeader(k, v);
    }
    xhr.onload = () => resolve(new Response(xhr.responseText, {
      status: xhr.status, statusText: xhr.statusText,
    }));
    xhr.onerror = () => reject(new Error('Network error'));
    xhr.send(init?.body as string ?? null);
  });
}

interface ProofsPageData {
  proofs: ProofDetail[];
  checkpoints: ProofDetail[];
  receipts: ReceiptDetail[];
  receiptsTotalPaid: number;
  receiptsTotalRefunded: number;
  usdRate: number;
}

@customElement('dm-proofs')
export class WormProofs extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property() taskId = '';
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  private ctrl = new FetchController<ProofsPageData>(this);
  @state() private copiedTxid = '';
  @state() private expandedProofs = new Set<string>();
  @state() private verifyResults = new Map<string, VerifyStatus>();
  @state() private onChainResults = new Map<string, OnChainVerification>();
  @state() private onChainLoading = false;
  @state() private decryptResults = new Map<string, unknown>();
  @state() private decryptLoading = new Set<string>();
  /** SHA-256 hashes computed from checkpoint_data for state tokens. */
  @state() private stateTokenHashes = new Map<string, string>();
  @state() private verifyingHash = '';
  @state() private verifiedHashes = new Set<string>();

  static styles = [pageHost, centerState, pageTitle, stateFeedback, css`
    .page-title { margin-bottom: 4px; }

    .back-link {
      display: inline-flex; align-items: center; gap: 4px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-decoration: none; font-size: 13px; margin-bottom: 12px;
      transition: color 0.15s ease;
    }
    .back-link:hover { color: var(--accent, #14A8C4); }
    .back-link:focus-visible { outline: 2px solid var(--accent, #14A8C4); outline-offset: 2px; }
    .subtitle { font-size: 13px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-bottom: 20px; }

    .proof-list { display: flex; flex-direction: column; gap: 12px; }

    .proof-card {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px; padding: 16px;
    }

    .proof-header { display: flex; align-items: center; gap: 8px; margin-bottom: 12px; }

    .proof-type-badge {
      display: inline-flex; align-items: center;
      padding: 2px 8px; border-radius: 4px;
      font-size: 11px; font-weight: 600;
      font-family: var(--mono, monospace);
      text-transform: uppercase; letter-spacing: 0.3px;
      color: var(--success, #00b69b);
      background: rgba(0, 182, 155, 0.1);
    }
    .proof-type-badge.decision { background: rgba(20, 168, 196, 0.15); border: 1px solid rgba(20, 168, 196, 0.4); color: #14A8C4; }
    .proof-type-badge.task_completion { background: rgba(0, 182, 155, 0.15); border: 1px solid rgba(0, 182, 155, 0.4); color: #00b69b; }
    .proof-type-badge.budget_snapshot { background: rgba(252, 190, 45, 0.15); border: 1px solid rgba(252, 190, 45, 0.4); color: #fcbe2d; }
    .proof-type-badge.checkpoint { background: rgba(167, 139, 250, 0.15); border: 1px solid rgba(167, 139, 250, 0.4); color: #a78bfa; }
    .proof-type-badge.capability_proof { background: rgba(0, 182, 155, 0.15); border: 1px solid rgba(0, 182, 155, 0.4); color: #00b69b; }
    .proof-type-badge.memory_commitment { background: rgba(167, 139, 250, 0.15); border: 1px solid rgba(167, 139, 250, 0.4); color: #a78bfa; }

    .proof-iter-badge { display: inline-flex; align-items: center; padding: 2px 8px; border-radius: 4px; font-size: 11px; font-weight: 600; font-family: var(--mono, monospace); color: var(--accent, #14A8C4); background: rgba(20, 168, 196, 0.1); }
    .proof-cost { font-size: 11px; color: var(--warning, #fcbe2d); font-family: var(--mono, monospace); }
    .proof-time { font-size: 11px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-left: auto; }

    .proof-field { margin-bottom: 10px; }
    .proof-field:last-child { margin-bottom: 0; }
    .field-label { font-size: 10px; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-bottom: 4px; }
    .field-value { font-family: var(--mono, monospace); font-size: 13px; color: var(--text, #e0e0e8); background: var(--bg, #0B1929); border: 1px solid var(--border, #1A3550); border-radius: 4px; padding: 8px 10px; word-break: break-all; line-height: 1.5; position: relative; }
    .field-value.hash { color: var(--text-bright, #fff); font-size: 14px; letter-spacing: 0.5px; }
    .field-value pre { margin: 0; white-space: pre-wrap; word-break: break-word; font-family: inherit; font-size: inherit; }

    .copy-btn { display: inline-flex; align-items: center; gap: 4px; background: none; border: 1px solid var(--border, #1A3550); color: var(--text-dim, rgba(255,255,255,0.5)); padding: 4px 8px; border-radius: 4px; cursor: pointer; font-size: 11px; font-family: var(--mono, monospace); transition: color 0.15s ease, border-color 0.15s ease; margin-top: 6px; }
    .copy-btn:hover { color: var(--text, #e0e0e8); border-color: var(--text-dim, rgba(255,255,255,0.5)); }
    .copy-btn.copied { color: var(--success, #00b69b); border-color: var(--success, #00b69b); }

    .expand-btn { display: inline-flex; align-items: center; gap: 4px; background: none; border: 1px solid var(--border, #1A3550); color: var(--text-dim, rgba(255,255,255,0.5)); padding: 4px 10px; border-radius: 4px; cursor: pointer; font-size: 11px; font-family: var(--mono, monospace); transition: color 0.15s ease, border-color 0.15s ease; margin-top: 8px; }
    .expand-btn:hover { color: var(--text, #e0e0e8); border-color: var(--text-dim, rgba(255,255,255,0.5)); }

    .verify-btn { display: inline-flex; align-items: center; gap: 4px; background: none; border: 1px solid var(--success, #00b69b); color: var(--success, #00b69b); padding: 4px 10px; border-radius: 4px; cursor: pointer; font-size: 11px; font-family: var(--mono, monospace); transition: opacity 0.15s ease; margin-top: 8px; margin-left: 6px; }
    .verify-btn:hover { opacity: 0.8; }
    .verify-btn:disabled { opacity: 0.4; cursor: not-allowed; border-color: var(--text-dim, rgba(255,255,255,0.5)); color: var(--text-dim, rgba(255,255,255,0.5)); }

    .verify-badge { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: 4px; font-size: 11px; font-weight: 600; font-family: var(--mono, monospace); margin-left: 6px; }
    .verify-badge.match { color: var(--success, #00b69b); background: rgba(0, 182, 155, 0.1); }
    .verify-badge.mismatch { color: var(--error, #fd5454); background: rgba(253, 84, 84, 0.1); }

    .hash-verified { animation: hash-glow 0.6s ease-out; color: var(--success, #00b69b) !important; }
    @keyframes hash-glow { 0% { color: var(--text-dim); } 30% { color: var(--success); text-shadow: 0 0 8px rgba(0, 182, 155, 0.4); } 100% { color: var(--success); text-shadow: none; } }
    .badge-enter { animation: badge-pop 0.3s cubic-bezier(0.34, 1.56, 0.64, 1); }
    @keyframes badge-pop { 0% { transform: scale(0.8); opacity: 0; } 100% { transform: scale(1); opacity: 1; } }
    .badge-shake { animation: shake 0.4s ease-in-out; }
    @keyframes shake { 0%, 100% { transform: translateX(0); } 20% { transform: translateX(-4px); } 40% { transform: translateX(4px); } 60% { transform: translateX(-2px); } 80% { transform: translateX(2px); } }

    .data-section { margin-top: 10px; border-top: 1px solid var(--border, #1A3550); padding-top: 10px; }

    .verify-all-btn { display: inline-flex; align-items: center; gap: 6px; background: rgba(0, 182, 155, 0.08); border: 1px solid var(--success, #00b69b); color: var(--success, #00b69b); padding: 10px 20px; border-radius: 6px; cursor: pointer; font-size: 13px; font-weight: 600; font-family: var(--mono, monospace); transition: opacity 0.15s ease, background 0.15s ease; margin-bottom: 16px; position: relative; overflow: hidden; }
    .verify-all-btn:hover { background: rgba(0, 182, 155, 0.15); }
    .verify-all-btn:disabled { opacity: 0.4; cursor: not-allowed; }
    .verify-all-btn.verifying::after { content: ''; position: absolute; left: 0; bottom: 0; height: 3px; background: var(--success, #00b69b); animation: verify-progress 2s ease-in-out; }
    @keyframes verify-progress { 0% { width: 0; } 100% { width: 100%; } }

    .onchain-status { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 8px; }
    .onchain-badge { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: 4px; font-size: 10px; font-weight: 600; font-family: var(--mono, monospace); text-transform: uppercase; }
    .onchain-badge.match { color: var(--success, #00b69b); background: rgba(0, 182, 155, 0.1); }
    .onchain-badge.mismatch { color: var(--error, #fd5454); background: rgba(253, 84, 84, 0.1); }
    .onchain-badge.not-found { color: var(--warning, #fcbe2d); background: rgba(252, 190, 45, 0.1); }
    .onchain-badge.unavailable { color: var(--text-dim, rgba(255,255,255,0.5)); background: rgba(255, 255, 255, 0.06); }

    .decrypt-btn { display: inline-flex; align-items: center; gap: 4px; background: none; border: 1px solid var(--accent, #14A8C4); color: var(--accent, #14A8C4); padding: 4px 10px; border-radius: 4px; cursor: pointer; font-size: 11px; font-family: var(--mono, monospace); transition: opacity 0.15s ease; margin-top: 8px; margin-left: 6px; }
    .decrypt-btn:hover { opacity: 0.8; }
    .decrypt-btn:disabled { opacity: 0.4; cursor: not-allowed; }

    .decrypt-result { margin-top: 10px; border-top: 1px solid var(--border, #1A3550); padding-top: 10px; }
    .decrypt-error { color: var(--error, #fd5454); font-size: 12px; font-style: italic; margin-top: 8px; }
    .no-data-hint { font-size: 11px; color: var(--text-dim, rgba(255,255,255,0.5)); font-style: italic; margin-top: 8px; }
    .hash-empty { color: var(--text-dim, rgba(255,255,255,0.3)); font-style: italic; font-size: 12px; }

    .explainer {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px; padding: 14px 16px;
      margin-bottom: 16px; font-size: 13px;
      line-height: 1.6; color: var(--text, #e0e0e8);
    }
    .explainer summary {
      cursor: pointer; color: var(--text-bright, #fff);
      font-weight: 600; font-size: 13px;
      user-select: none;
    }
    .explainer summary:hover { color: var(--accent, #14A8C4); }
    .explainer p { margin: 8px 0 0; color: var(--text-dim, rgba(255,255,255,0.5)); font-size: 12px; line-height: 1.5; }
    .explainer .proof-type-explain { margin-top: 10px; display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; font-size: 12px; }
    .explainer .pte-label { font-weight: 600; color: var(--text, #e0e0e8); }
    .explainer .pte-desc { color: var(--text-dim, rgba(255,255,255,0.5)); }

    .verify-subtitle { display: block; font-size: 9px; font-weight: 400; color: var(--text-dim, rgba(255,255,255,0.5)); margin-top: 2px; letter-spacing: 0; text-transform: none; }

    .proof-data-grid { display: grid; grid-template-columns: auto 1fr; gap: 6px 12px; font-size: 13px; margin-top: 8px; }
    .proof-data-key { color: var(--text-dim, rgba(255,255,255,0.5)); font-size: 11px; text-transform: uppercase; letter-spacing: 0.3px; align-self: center; }
    .proof-data-val { font-family: var(--mono, monospace); color: var(--text, #e0e0e8); word-break: break-all; }

    .cost-summary { font-size: 13px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-bottom: 16px; padding: 10px 14px; background: var(--bg-elevated, #142D42); border: 1px solid var(--border, #1A3550); border-radius: 8px; }
    .cost-summary .cost-value { color: var(--warning, #fcbe2d); font-family: var(--mono, monospace); font-weight: 600; }

    .woc-link { display: inline-flex; align-items: center; gap: 4px; color: var(--accent, #14A8C4); text-decoration: none; font-size: 13px; padding: 8px 0; transition: color 0.15s ease; }
    .woc-link:hover { color: var(--text-bright, #fff); text-decoration: underline; }
    .woc-link:focus-visible { outline: 2px solid var(--accent, #14A8C4); outline-offset: 2px; }

    @media (max-width: 639px) {
      .proof-card { padding: 12px; }
      .field-value { font-size: 12px; }
      .field-value.hash { font-size: 12px; }
    }
  `];

  firstUpdated() { this.loadProofs(); }

  updated(changed: Map<string, unknown>) {
    if (changed.has('taskId') && this.taskId) this.loadProofs();
  }

  private async loadProofs() {
    if (!this.taskId) return;
    await this.ctrl.fetch(async () => {
      const [proofsRes, receiptsRes, rate] = await Promise.all([
        this.fetchFn(`/task/${this.taskId}/proofs`),
        this.fetchFn(`/task/${this.taskId}/receipts`),
        fetchBsvUsdRate(),
      ]);
      if (!proofsRes.ok) throw new Error(`Failed to load proofs: ${proofsRes.status}`);
      const proofsData: ProofsResponse = await proofsRes.json();
      const proofs: ProofDetail[] = proofsData.proofs ?? [];
      const checkpoints: ProofDetail[] = proofsData.checkpoints ?? [];

      let receipts: ReceiptDetail[] = [];
      let receiptsTotalPaid = 0;
      let receiptsTotalRefunded = 0;
      if (receiptsRes.ok) {
        const receiptsData: ReceiptsResponse = await receiptsRes.json();
        receipts = receiptsData.receipts ?? [];
        receiptsTotalPaid = receiptsData.total_sats_paid ?? 0;
        receiptsTotalRefunded = receiptsData.total_sats_refunded ?? 0;
      }

      // Compute SHA-256 hashes for state tokens that have checkpoint_data
      for (const cp of checkpoints) {
        if (cp.checkpoint_data && !cp.hash) {
          const json = JSON.stringify(cp.checkpoint_data);
          sha256(json).then(hash => {
            const next = new Map(this.stateTokenHashes);
            next.set(cp.txid, hash);
            this.stateTokenHashes = next;
          });
        }
      }

      return { proofs, checkpoints, receipts, receiptsTotalPaid, receiptsTotalRefunded, usdRate: rate };
    });
  }

  private async copyToClipboard(text: string, txid: string) {
    try { await navigator.clipboard.writeText(text); this.copiedTxid = txid; setTimeout(() => { this.copiedTxid = ''; }, 2000); } catch { /* ignore */ }
  }

  private toggleExpanded(txid: string) {
    const next = new Set(this.expandedProofs);
    if (next.has(txid)) next.delete(txid); else next.add(txid);
    this.expandedProofs = next;
  }

  private async verifyHash(p: ProofDetail) {
    if (!p.proof_data || !p.proof_timestamp) return;
    this.verifyingHash = p.txid;
    const next = new Map(this.verifyResults); next.set(p.txid, 'pending'); this.verifyResults = next;
    const computed = await computeProofHash(p.proof_data, p.proof_timestamp, p.prev_hash);
    await new Promise(r => setTimeout(r, 300));
    const result: VerifyStatus = computed === p.hash ? 'match' : 'mismatch';
    const updated = new Map(this.verifyResults); updated.set(p.txid, result); this.verifyResults = updated;
    this.verifyingHash = '';
    if (result === 'match') { const vh = new Set(this.verifiedHashes); vh.add(p.txid); this.verifiedHashes = vh; }
  }

  private async verifyOnChain() {
    if (!this.taskId || this.onChainLoading) return;
    this.onChainLoading = true;
    try {
      const res = await this.fetchFn(`/task/${this.taskId}/proofs/verify`);
      if (!res.ok) throw new Error(`Verification failed: ${res.status}`);
      const data = await res.json();
      const next = new Map<string, OnChainVerification>();
      for (const v of data.verifications ?? []) next.set(v.txid, v);
      this.onChainResults = next;
    } catch (e) {
      // Use a temporary error display -- don't overwrite ctrl.error
      console.error('On-chain verification failed:', e);
    } finally {
      this.onChainLoading = false;
    }
  }

  /**
   * Extract PushDrop data hex from the 1-sat output of a transaction.
   * Tries wallet first, falls back to WhatsOnChain for spent UTXOs.
   */
  private async fetchPushDropData(txid: string, basket: string): Promise<string> {
    // Try wallet first (fast, local)
    const outRes = await fetch(`/output/${encodeURIComponent(basket)}/${txid}`);
    if (outRes.ok) {
      const output = await outRes.json();
      const fields: string[] = output.data_fields ?? [];
      if (fields.length >= 2 && fields[1]) return fields[1];
    }

    // Wallet doesn't have it (spent UTXO) — fetch raw tx from WhatsOnChain
    const wocRes = await fetch(`https://api.whatsonchain.com/v1/bsv/main/tx/${txid}/hex`);
    if (!wocRes.ok) throw new Error(`Transaction not found on-chain (${wocRes.status})`);
    const rawHex = await wocRes.text();

    // Parse the raw tx to find the 1-sat PushDrop output.
    // PushDrop outputs have data pushes followed by OP_DROP/OP_2DROP + OP_CHECKSIG.
    // The data field we want is the second push in the script.
    return this.extractPushDropFromRawTx(rawHex);
  }

  /** Extract the second data field from the PushDrop output in a raw tx hex. */
  private extractPushDropFromRawTx(rawHex: string): string {
    // Find outputs by scanning for the pattern: small value (1 sat) outputs
    // with PushDrop scripts. The 1-sat output contains our state token data.
    // PushDrop scripts end with OP_DROP(75)/OP_2DROP(6d) ... ac(OP_CHECKSIG).
    //
    // Strategy: decode the tx, find outputs, look for scripts ending in 'ac'
    // (OP_CHECKSIG) that aren't standard P2PKH (which start with 76a914).
    const bytes = new Uint8Array(rawHex.match(/.{2}/g)!.map(b => parseInt(b, 16)));

    // Simple heuristic: find the PushDrop script by looking for a non-P2PKH
    // output script that ends with OP_CHECKSIG (0xac).
    // Parse outputs: skip version(4) + input count + inputs, then read outputs.
    let pos = 4; // skip version

    // Skip inputs
    const inputCount = bytes[pos++];
    for (let i = 0; i < inputCount; i++) {
      pos += 36; // prev txid(32) + vout(4)
      const scriptLen = this.readVarInt(bytes, pos);
      pos = scriptLen.newPos + scriptLen.value;
      pos += 4; // sequence
    }

    // Read outputs
    const outputCount = bytes[pos++];
    for (let i = 0; i < outputCount; i++) {
      // value: 8 bytes LE
      const value = Number(bytes[pos]) + Number(bytes[pos + 1]) * 256;
      pos += 8;
      const scriptLen = this.readVarInt(bytes, pos);
      pos = scriptLen.newPos;
      const script = rawHex.substring(pos * 2, (pos + scriptLen.value) * 2);
      pos += scriptLen.value;

      // Skip P2PKH outputs (76a914...88ac) — we want the PushDrop output
      if (script.startsWith('76a914')) continue;

      // This is likely our PushDrop output — extract data pushes
      return this.extractSecondPush(script);
    }
    throw new Error('No PushDrop output found in transaction');
  }

  /** Read a Bitcoin varint from a byte array. */
  private readVarInt(bytes: Uint8Array, pos: number): { value: number; newPos: number } {
    const first = bytes[pos];
    if (first < 0xfd) return { value: first, newPos: pos + 1 };
    if (first === 0xfd) return { value: bytes[pos + 1] + bytes[pos + 2] * 256, newPos: pos + 3 };
    // fd, fe, ff cases — only fd is common for our use case
    return { value: bytes[pos + 1] + bytes[pos + 2] * 256, newPos: pos + 3 };
  }

  /** Extract the second data push from a PushDrop script hex string. */
  private extractSecondPush(scriptHex: string): string {
    const bytes = new Uint8Array(scriptHex.match(/.{2}/g)!.map(b => parseInt(b, 16)));
    let pos = 0;
    let pushCount = 0;

    while (pos < bytes.length) {
      const op = bytes[pos++];
      let pushLen = 0;

      if (op >= 1 && op <= 75) {
        pushLen = op;
      } else if (op === 0x4c) { // OP_PUSHDATA1
        pushLen = bytes[pos++];
      } else if (op === 0x4d) { // OP_PUSHDATA2
        pushLen = bytes[pos] + bytes[pos + 1] * 256;
        pos += 2;
      } else {
        // Not a data push — opcode (DROP, CHECKSIG, etc.)
        continue;
      }

      pushCount++;
      if (pushCount === 2) {
        // This is the second push — the encrypted/plaintext data
        return Array.from(bytes.slice(pos, pos + pushLen)).map(b => b.toString(16).padStart(2, '0')).join('');
      }
      pos += pushLen;
    }
    throw new Error('Could not find data field in PushDrop script');
  }

  private async decryptCheckpoint(txid: string, basket: string) {
    if (this.decryptLoading.has(txid)) return;
    const loading = new Set(this.decryptLoading); loading.add(txid); this.decryptLoading = loading;
    const authFetch = this.fetchFn ?? fetch;
    try {
      const dataHex = await this.fetchPushDropData(txid, basket);

      // Try to parse as plain UTF-8 JSON first (unencrypted tokens)
      try {
        const utf8 = new TextDecoder().decode(new Uint8Array(dataHex.match(/.{2}/g)!.map(b => parseInt(b, 16))));
        const parsed = JSON.parse(utf8);
        const next = new Map(this.decryptResults); next.set(txid, parsed); this.decryptResults = next;
        return;
      } catch { /* Not valid JSON — try decrypting */ }

      // Encrypted — decrypt via wallet
      const decRes = await authFetch('/decrypt', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ciphertext_hex: dataHex, protocol_id: [2, 'dolphin milk state'], key_id: 'tokens', counterparty: 'self' }),
      });
      if (!decRes.ok) { let detail = ''; try { detail = await decRes.text(); } catch { /* ignore */ } throw new Error(detail || 'Decryption failed \u2014 ensure your wallet is connected and running'); }
      const decData = await decRes.json();
      const next = new Map(this.decryptResults); next.set(txid, decData.parsed_json ?? decData.plaintext); this.decryptResults = next;
    } catch (e) {
      const next = new Map(this.decryptResults); next.set(txid, { error: e instanceof Error ? e.message : 'Decrypt failed' }); this.decryptResults = next;
    } finally {
      const done = new Set(this.decryptLoading); done.delete(txid); this.decryptLoading = done;
    }
  }

  private formatFullTimestamp(ts: number): string {
    return new Date(ts * 1000).toLocaleString('en-US', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false, timeZoneName: 'short' });
  }

  private renderOnChainStatus(p: ProofDetail) {
    const v = this.onChainResults.get(p.txid);
    if (!v) return nothing;
    const hashClass = v.hash_recompute === 'match' ? 'match' : v.hash_recompute === 'mismatch' ? 'mismatch' : 'unavailable';
    const chainClass = v.on_chain_match === 'match' ? 'match' : v.on_chain_match === 'mismatch' ? 'mismatch' : 'not-found';
    const hashLabel = v.hash_recompute === 'match' ? '\u2713 Hash recomputed' : v.hash_recompute === 'mismatch' ? '\u2717 Mismatch' : 'No data to recompute';
    const hashSubtitle = v.hash_recompute === 'match' ? 'Data integrity confirmed locally' : v.hash_recompute === 'mismatch' ? 'Data may have been tampered with' : '';
    const chainLabel = v.on_chain_match === 'match' ? '\u2713 On-chain verified' : v.on_chain_match === 'mismatch' ? '\u2717 On-chain mismatch' : 'Not found on-chain';
    const chainSubtitle = v.on_chain_match === 'match' ? 'Confirmed on BSV blockchain' : v.on_chain_match === 'mismatch' ? 'Data may have been tampered with' : 'Transaction may still be propagating';
    return html`<div class="onchain-status"><span class="onchain-badge ${hashClass}">${hashLabel}${hashSubtitle ? html`<span class="verify-subtitle">${hashSubtitle}</span>` : nothing}</span><span class="onchain-badge ${chainClass}">${chainLabel}<span class="verify-subtitle">${chainSubtitle}</span></span></div>`;
  }

  /** Extract meaningful identity/hash fields from proof/checkpoint data. */
  private extractDataHash(p: ProofDetail): string {
    const sources = [p.checkpoint_data, this.decryptResults.get(p.txid)];
    for (const data of sources) {
      if (!data || typeof data !== 'object' || (data as any).error) continue;
      const d = data as Record<string, unknown>;
      const parts: string[] = [];
      // Hash fields
      if (d.task_hash) parts.push(`task: ${d.task_hash}`);
      if (d.hmac) parts.push(`hmac: ${d.hmac}`);
      if (d.skills_hash) parts.push(`skills: ${d.skills_hash}`);
      if (d.state_root) parts.push(`state_root: ${d.state_root}`);
      // Budget/spending fields (budget_allocation tokens)
      if (d.budget_cap) parts.push(`cap: ${Number(d.budget_cap).toLocaleString()} sats`);
      if (d.task_spent != null) parts.push(`spent: ${Number(d.task_spent).toLocaleString()} sats`);
      // Iteration
      if (d.iteration != null && parts.length > 0) parts.push(`iter: ${d.iteration}`);
      if (parts.length > 0) return parts.join(' · ');
    }
    return '';
  }

  private friendlyErrorMessage(msg: string): string {
    if (msg === 'No data field in PushDrop script') return 'Checkpoint data requires wallet connection to decrypt';
    if (msg.startsWith('Decrypt failed:')) return 'Decryption failed \u2014 ensure your wallet is connected and running';
    return msg;
  }

  private renderStructuredData(proofData: string, proofType: string) {
    if (!proofData || typeof proofData !== 'string') {
      return html`<div class="field-value"><pre class="unparseable">Unable to display proof data</pre></div>`;
    }
    // Try to parse the proof_data as JSON for structured display
    try {
      const parsed = JSON.parse(proofData);
      if (typeof parsed === 'object' && parsed !== null) {
        if (proofType === 'decision') {
          return this.renderDecisionData(parsed);
        }
        return this.renderGenericStructuredData(parsed);
      }
    } catch {
      // Not JSON — fall through to raw display
    }
    // If it looks like hex (only hex chars, even length), label it as raw hex
    if (/^[0-9a-fA-F]+$/.test(proofData) && proofData.length > 64) {
      return html`<div class="field-value"><pre>Raw OP_RETURN data (hex, ${proofData.length / 2} bytes):\n${proofData}</pre></div>`;
    }
    return html`<div class="field-value"><pre>${proofData}</pre></div>`;
  }

  private renderDecisionData(data: Record<string, unknown>) {
    const fieldMap: Record<string, string> = {
      model: 'Model',
      tokens: 'Tokens',
      sats: 'Cost (sats)',
      payment_txid: 'Payment TX',
      task_id: 'Task',
      iteration: 'Iteration',
      memory_ids: 'Memory References',
    };
    const knownKeys = Object.keys(fieldMap);
    const shownKeys = knownKeys.filter(k => data[k] !== undefined && data[k] !== null);
    const remainingKeys = Object.keys(data).filter(k => !knownKeys.includes(k));

    if (shownKeys.length === 0 && remainingKeys.length === 0) {
      return html`<div class="field-value"><pre>${JSON.stringify(data, null, 2)}</pre></div>`;
    }

    return html`
      <div class="proof-data-grid">
        ${shownKeys.map(k => {
          let val = data[k];
          let display: unknown;
          if (k === 'memory_ids' && Array.isArray(val)) {
            display = val.length > 0 ? val.join(', ') : 'None';
          } else if (k === 'sats' && typeof val === 'number') {
            display = formatInlineCurrency(val, this.ctrl.data?.usdRate ?? 0, this.currencyMode);
          } else {
            display = String(val);
          }
          return html`
            <span class="proof-data-key">${fieldMap[k]}</span>
            <span class="proof-data-val">${display}</span>
          `;
        })}
      </div>
      ${remainingKeys.length > 0 ? html`
        <div class="proof-data-grid" style="margin-top: 8px;">
          ${remainingKeys.map(k => html`
            <span class="proof-data-key">${k}</span>
            <span class="proof-data-val">${typeof data[k] === 'object' ? JSON.stringify(data[k]) : String(data[k])}</span>
          `)}
        </div>
      ` : nothing}
    `;
  }

  private renderGenericStructuredData(data: Record<string, unknown>) {
    const entries = Object.entries(data);
    if (entries.length === 0) {
      return html`<div class="field-value"><pre>${JSON.stringify(data, null, 2)}</pre></div>`;
    }
    // Display key-value pairs with human-readable formatting
    const labelMap: Record<string, string> = {
      task_id: 'Task',
      session_id: 'Session',
      total_sats: 'Total Cost (sats)',
      iterations: 'Iterations',
      balance_before: 'Balance Before',
      balance_after: 'Balance After',
      model: 'Model',
      tool_name: 'Tool',
      tool_category: 'Category',
      memory_id: 'Memory ID',
      uhrp_hash: 'Content Hash',
      skills_hash: 'Skills Hash',
      capabilities: 'Capabilities',
      services: 'Services',
    };
    return html`
      <div class="proof-data-grid">
        ${entries.map(([k, v]) => {
          const label = labelMap[k] ?? k.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
          const display = typeof v === 'object' ? JSON.stringify(v) : String(v);
          return html`
            <span class="proof-data-key">${label}</span>
            <span class="proof-data-val">${display}</span>
          `;
        })}
      </div>
    `;
  }

  private renderProofCard(p: ProofDetail, usdRate: number) {
    return html`
      <div class="proof-card">
        <div class="proof-header">
          <span class="proof-type-badge ${p.proof_type}">${PROOF_TYPE_LABELS[p.proof_type] ?? p.proof_type}</span>
          ${p.iteration != null ? html`<span class="proof-iter-badge">${p.proof_type === 'task_completion' || p.proof_type === 'budget_snapshot' ? `${p.iteration} iterations` : `Iter ${p.iteration}`}</span>` : nothing}
          ${p.sats_cost ? html`<span class="proof-cost" title="${formatInlineCurrencyAlt(p.sats_cost, usdRate, this.currencyMode)}">${formatInlineCurrency(p.sats_cost, usdRate, this.currencyMode)}</span>` : nothing}
          <span class="proof-time" title=${this.formatFullTimestamp(p.timestamp)}>${formatRelativeTime(p.timestamp)}</span>
        </div>
        <div class="proof-field">
          <div class="field-label">Transaction ID</div>
          <div class="field-value">${p.txid}</div>
          <button class="copy-btn ${this.copiedTxid === p.txid ? 'copied' : ''}" @click=${() => this.copyToClipboard(p.txid, p.txid)}>${this.copiedTxid === p.txid ? '\u2713 Copied' : 'Copy'}</button>
        </div>
        <div class="proof-field">
          <div class="field-label">SHA-256 Hash</div>
          ${p.hash
            ? html`<div class="field-value hash ${this.verifiedHashes.has(p.txid) ? 'hash-verified' : ''}">${p.hash}</div>`
            : this.stateTokenHashes.has(p.txid)
              ? html`<div class="field-value hash">${this.stateTokenHashes.get(p.txid)}</div>`
              : html`<div class="field-value hash hash-empty">Encrypted on-chain</div>`
          }
          ${this.renderOnChainStatus(p)}
        </div>
        <div class="proof-field">
          <div class="field-label">Timestamp</div>
          <div class="field-value">${this.formatFullTimestamp(p.timestamp)}</div>
        </div>
        ${this.renderProofData(p)}
        <a class="woc-link" href="https://whatsonchain.com/tx/${p.txid}" target="_blank" rel="noopener noreferrer">View on WhatsOnChain \u2192</a>
      </div>
    `;
  }

  private renderProofData(p: ProofDetail) {
    const expanded = this.expandedProofs.has(p.txid);
    const hasData = !!p.proof_data;
    const hasCheckpoint = !!p.checkpoint_data;
    const verifyStatus = this.verifyResults.get(p.txid);
    const isStateToken = !!p.basket && !p.hash;
    const decryptResult = this.decryptResults.get(p.txid);
    const isDecrypting = this.decryptLoading.has(p.txid);

    if (!hasData && !hasCheckpoint) return html`<div class="no-data-hint">Data not recorded (older format)</div>`;

    return html`
      <div>
        <button class="expand-btn" @click=${() => this.toggleExpanded(p.txid)}>${expanded ? '\u25BC' : '\u25B6'} Proof Data</button>
        ${hasData ? html`<button class="verify-btn" ?disabled=${!hasData || this.verifyingHash === p.txid} @click=${() => this.verifyHash(p)}>${this.verifyingHash === p.txid ? 'Verifying...' : 'Verify Hash'}</button>` : nothing}
        ${isStateToken && !hasCheckpoint ? html`<button class="decrypt-btn" ?disabled=${isDecrypting || !!decryptResult} @click=${() => this.decryptCheckpoint(p.txid, p.basket!)}>${isDecrypting ? 'Decrypting...' : decryptResult ? 'Decrypted' : 'Decrypt On-Chain'}</button>` : nothing}
        ${isStateToken && hasCheckpoint ? html`<button class="decrypt-btn" ?disabled=${isDecrypting || !!decryptResult} @click=${() => this.decryptCheckpoint(p.txid, p.basket!)}>${isDecrypting ? 'Verifying...' : decryptResult ? 'Verified' : 'Verify On-Chain'}</button>` : nothing}
        ${verifyStatus === 'match' ? html`<span class="verify-badge match badge-enter">\u2713 Hash verified</span>` : verifyStatus === 'mismatch' ? html`<span class="verify-badge mismatch badge-shake">\u2717 Hash mismatch</span>` : nothing}
      </div>
      ${expanded ? html`
        <div class="data-section">
          ${hasData ? html`
            <div class="proof-field">
              <div class="field-label">Commitment Data</div>
              ${this.renderStructuredData(p.proof_data!, p.proof_type)}
            </div>
            <div class="proof-field"><div class="field-label">Proof Timestamp (RFC 3339)</div><div class="field-value">${p.proof_timestamp}</div></div>
          ` : nothing}
          ${hasCheckpoint ? html`<div class="proof-field"><div class="field-label">Checkpoint Data (Transcript)</div><div class="field-value"><pre>${JSON.stringify(p.checkpoint_data, null, 2)}</pre></div></div>` : nothing}
          ${decryptResult ? html`
            <div class="decrypt-result">
              ${(decryptResult as Record<string, unknown>).error
                ? html`<div class="decrypt-error">${this.friendlyErrorMessage((decryptResult as Record<string, unknown>).error as string)}</div>`
                : html`<div class="proof-field"><div class="field-label">On-Chain Data (Decrypted)</div><div class="field-value"><pre>${typeof decryptResult === 'string' ? decryptResult : JSON.stringify(decryptResult, null, 2)}</pre></div></div>`}
            </div>
          ` : nothing}
        </div>
      ` : nothing}
    `;
  }

  render() {
    if (this.ctrl.loading && !this.ctrl.hasData) return renderLoading('Loading proofs...');
    if (this.ctrl.hasError && !this.ctrl.hasData) return renderError(this.ctrl.error ?? 'Failed to load proofs', 'Check the task ID and that the agent server is running.');

    const d = this.ctrl.data;
    if (!d) return html`<div class="center-state">No proof data available.</div>`;

    const totalCost = [...d.proofs, ...d.checkpoints].reduce((sum, p) => sum + (p.sats_cost ?? 0), 0);

    const taskShort = this.taskId.length > 8 ? this.taskId.slice(0, 8) : this.taskId;

    return html`
      ${renderBreadcrumb([
        { label: 'Tasks', hash: '#tasks' },
        { label: `Task #${taskShort}`, hash: `#task/${this.taskId}` },
        { label: 'Proofs' },
      ])}
      <div class="page-title">Proof Viewer</div>
      <div class="subtitle">
        ${d.proofs.length} chain proof${d.proofs.length !== 1 ? 's' : ''} &middot; ${d.checkpoints.length} state token${d.checkpoints.length !== 1 ? 's' : ''}${totalCost > 0 ? html` &middot; <span title="${formatInlineCurrencyAlt(totalCost, d.usdRate, this.currencyMode)}">${formatInlineCurrency(totalCost, d.usdRate, this.currencyMode)}</span>` : nothing}
      </div>

      <details class="explainer">
        <summary>What are proofs and why do they matter?</summary>
        <p>Every action the agent takes is recorded on the BSV blockchain as a cryptographic proof.
        Each proof contains a SHA-256 hash of what happened, chained to the previous proof.
        This creates a tamper-evident audit trail — if anyone modifies a record, the chain breaks and verification fails.</p>
        <div class="proof-type-explain">
          <span class="pte-label">Agent Decision</span><span class="pte-desc">Records each thinking step — what model was used, how much it cost, and what was decided</span>
          <span class="pte-label">Tool Usage</span><span class="pte-desc">Records when the agent called a paid tool — which tool, what it cost, and the payment transaction</span>
          <span class="pte-label">Memory Saved</span><span class="pte-desc">Records when the agent stored a memory — content hash for integrity verification</span>
          <span class="pte-label">Task Complete</span><span class="pte-desc">Final record when a task finishes — total iterations and outcome</span>
          <span class="pte-label">Budget Record</span><span class="pte-desc">Snapshot of spending limits at task end — prevents overspending disputes</span>
          <span class="pte-label">Session Checkpoint</span><span class="pte-desc">Encrypted state snapshot stored on-chain — allows session resume after restart</span>
        </div>
      </details>

      ${totalCost > 0 ? html`
        <div class="cost-summary">
          Proof recording cost: <span class="cost-value" title="${formatInlineCurrencyAlt(totalCost, d.usdRate, this.currencyMode)}">${formatInlineCurrency(totalCost, d.usdRate, this.currencyMode)}</span>
          across ${d.proofs.length + d.checkpoints.length} on-chain records
        </div>
      ` : nothing}

      ${d.proofs.length > 0 ? html`
        <button class="verify-all-btn ${this.onChainLoading ? 'verifying' : ''}" ?disabled=${this.onChainLoading} @click=${() => this.verifyOnChain()}>
          ${this.onChainLoading ? `Verifying... ${this.onChainResults.size}/${d.proofs.length}` : this.onChainResults.size > 0 ? `Re-verify On-Chain (${d.proofs.length} proofs)` : `Verify All On-Chain (${d.proofs.length} proofs)`}
        </button>
      ` : nothing}

      ${d.proofs.length === 0 && d.checkpoints.length === 0
        ? renderEmpty('No proofs recorded', 'Proofs are created when the agent completes tasks with on-chain verification enabled.')
        : html`
            ${d.proofs.length > 0 ? html`
              <div class="proof-list">
                ${d.proofs.map((p) => this.renderProofCard(p, d.usdRate))}
              </div>
            ` : nothing}
            ${d.checkpoints.length > 0 ? html`
              <div style="margin-top: 24px; margin-bottom: 12px;">
                <span style="font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-dim, rgba(255,255,255,0.5));">State Tokens (${d.checkpoints.length})</span>
                <span style="font-size: 11px; color: var(--text-dim, rgba(255,255,255,0.5)); margin-left: 8px;">BRC-48 spendable UTXOs carrying agent state</span>
              </div>
              <div class="proof-list">
                ${d.checkpoints.map((p) => this.renderProofCard(p, d.usdRate))}
              </div>
            ` : nothing}
          `}

      <dm-receipt-table
        .receipts=${d.receipts}
        .totalPaid=${d.receiptsTotalPaid}
        .totalRefunded=${d.receiptsTotalRefunded}
        .usdRate=${d.usdRate}
        .currencyMode=${this.currencyMode}
      ></dm-receipt-table>
    `;
  }
}
