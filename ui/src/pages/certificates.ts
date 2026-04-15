import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { truncateKey, formatSats } from '../lib/util.js';
import type { CertificateStatus } from '../lib/shared-types.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import { centerState, buttonStyles, pageHost, pageTitle, stateFeedback, renderLoading, renderError } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';
import { usdToSats, formatUsd } from '../lib/usd.js';

interface CapabilityDef {
  id: string;
  label: string;
  desc: string;
  icon: string;
  color: string;
}

const CAPABILITIES: CapabilityDef[] = [
  { id: 'llm',       label: 'LLM Inference',  desc: 'Paid AI model calls via x402',           icon: '\u2726', color: '#14A8C4' },
  { id: 'tools',     label: 'Sandbox Tools',   desc: 'File I/O, bash, web fetch, browser',     icon: '\u2692', color: '#38bdf8' },
  { id: 'messaging', label: 'Messaging',       desc: 'Cross-wallet BRC-33 agent messages',     icon: '\u2709', color: '#34d399' },
  { id: 'x402',      label: 'x402 Services',   desc: 'Paid external API calls & payments',     icon: '\u26A1', color: '#fcbe2d' },
  { id: 'wallet',    label: 'Wallet',          desc: 'Balance, keys, encrypt, decrypt',        icon: '\u26BF', color: '#a78bfa' },
  { id: 'memory',    label: 'Memory',          desc: 'Persistent knowledge store & search',    icon: '\u2B9A', color: '#f472b6' },
  { id: 'schedule',  label: 'Scheduling',      desc: 'Create and manage recurring tasks',      icon: '\u29D6', color: '#fb923c' },
];

const DEFAULT_CAPS = new Set(['llm', 'tools', 'messaging', 'x402']);

@customElement('dm-certificates')
export class WormCertificates extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = 'sats-first';
  @property({ type: Number }) usdRate = 0;
  @state() private actionLoading = false;
  @state() private actionMessage = '';
  @state() private actionTxid = '';
  @state() private showRawCert = false;
  @state() private showRevokeConfirm = false;
  @state() private agentName = '';
  @state() private selectedCaps = new Set(DEFAULT_CAPS);
  @state() private budgetPerTask = '';
  @state() private budgetPerHour = '';
  @state() private budgetPerDay = '';
  @state() private showBudgetInputs = false;

  private ctrl = new FetchController<CertificateStatus>(this);

  static styles = [pageHost, centerState, pageTitle, buttonStyles, stateFeedback, css`
    /* ── Status card ── */
    .status-card {
      background: var(--bg-elevated, #142D42);
      border: 2px solid var(--border, #1A3550);
      border-radius: 14px;
      padding: 20px;
      margin-bottom: 20px;
    }
    .status-card.parent-signed { border-color: var(--success, #00b69b); }
    .status-card.self-signed   { border-color: var(--warning, #fcbe2d); }
    .status-card.none          { border-color: var(--error, #fd5454); }

    .status-header {
      display: flex;
      align-items: center;
      gap: 10px;
      margin-bottom: 14px;
    }
    .status-icon {
      width: 28px; height: 28px;
      border-radius: 50%;
      display: flex; align-items: center; justify-content: center;
      font-size: 14px; flex-shrink: 0;
    }
    .status-icon.parent-signed { background: rgba(0,182,155,0.15); color: var(--success); }
    .status-icon.self-signed   { background: rgba(252,190,45,0.15); color: var(--warning); }
    .status-icon.none          { background: rgba(253,84,84,0.15);  color: var(--error); }

    .status-label  { font-size: 16px; font-weight: 600; color: var(--text-bright, #fff); }
    .status-sublabel { font-size: 12px; color: var(--text-dim); }

    .cert-fields {
      display: grid;
      grid-template-columns: auto 1fr;
      gap: 6px 12px;
      margin-top: 12px;
    }
    .field-label {
      font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px;
      color: var(--text-dim); white-space: nowrap;
    }
    .field-value {
      font-size: 13px; font-family: var(--mono, monospace);
      color: var(--text); word-break: break-all;
    }

    .txid-link {
      color: var(--accent, #14A8C4);
      text-decoration: none;
    }
    .txid-link:hover {
      text-decoration: underline;
    }

    .toggle-raw {
      background: none; border: none;
      color: var(--text-dim); font-size: 12px;
      cursor: pointer; padding: 4px 0; text-decoration: underline;
    }
    .toggle-raw:hover { color: var(--text); }

    .raw-cert {
      background: var(--bg); border: 1px solid var(--border);
      border-radius: 6px; padding: 12px; margin-top: 16px; overflow-x: auto;
    }
    .raw-cert pre {
      margin: 0; font-size: 11px; font-family: var(--mono, monospace);
      color: var(--text-dim); white-space: pre-wrap; word-break: break-all;
    }

    .revocation-badge {
      display: inline-flex; align-items: center; gap: 4px;
      padding: 3px 10px; border-radius: 999px;
      font-size: 11px; font-weight: 600; font-family: var(--mono, monospace);
      text-transform: uppercase; letter-spacing: 0.3px;
    }
    .revocation-badge.active  { color: var(--success); background: rgba(0,182,155,0.12); }
    .revocation-badge.revoked { color: var(--error);   background: rgba(253,84,84,0.12); }

    .revoke-confirm {
      width: 100%; padding: 16px;
      background: rgba(253,84,84,0.06);
      border: 1px solid rgba(253,84,84,0.2);
      border-radius: 8px; margin-top: 8px;
    }
    .revoke-warning {
      display: block; font-size: 13px; color: var(--error);
      margin-bottom: 12px; line-height: 1.5;
    }
    .revoke-actions { display: flex; gap: 8px; }

    /* ── Issue form ── */
    .issue-form {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 14px;
      padding: 24px;
      margin-bottom: 20px;
    }
    .form-title {
      font-size: 15px; font-weight: 600;
      color: var(--text-bright, #fff);
      margin: 0 0 20px;
    }
    .form-section {
      margin-bottom: 16px;
    }
    .form-label-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      margin-bottom: 8px;
    }
    .form-label {
      font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px;
      color: var(--text-dim); font-weight: 600;
    }
    .form-hint {
      font-size: 11px; color: var(--text-dim);
      margin-top: 4px; font-style: italic;
      opacity: 0.7;
    }
    .form-input {
      width: 100%; box-sizing: border-box;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      padding: 8px 12px;
      color: var(--text, #e0e0e8);
      font-size: 13px; font-family: var(--sans);
      transition: border-color 0.15s ease;
    }
    .form-input:focus {
      outline: none;
      border-color: var(--accent, #14A8C4);
    }
    .form-input::placeholder {
      color: var(--text-dim);
      opacity: 0.6;
    }

    /* ── Capability grid ── */
    .select-toggle {
      background: none; border: none;
      color: var(--accent, #14A8C4);
      font-size: 11px; cursor: pointer;
      text-decoration: underline;
      padding: 0;
    }
    .select-toggle:hover { opacity: 0.8; }

    .cap-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
      gap: 8px;
    }
    .cap-card {
      display: flex; align-items: center; gap: 10px;
      padding: 10px 14px;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      cursor: pointer;
      transition: all 0.15s ease;
      user-select: none;
    }
    .cap-card:hover {
      background: rgba(255,255,255,0.03);
      border-color: rgba(255,255,255,0.12);
    }
    .cap-card.selected {
      background: rgba(72,128,255,0.06);
      border-color: rgba(72,128,255,0.3);
    }
    .cap-card.selected:hover {
      background: rgba(72,128,255,0.1);
    }
    .cap-icon {
      width: 32px; height: 32px;
      border-radius: 8px;
      display: flex; align-items: center; justify-content: center;
      font-size: 16px; flex-shrink: 0;
      transition: opacity 0.15s ease;
    }
    .cap-card:not(.selected) .cap-icon {
      opacity: 0.4;
    }
    .cap-info {
      flex: 1; min-width: 0;
    }
    .cap-name {
      font-size: 13px; font-weight: 600;
      color: var(--text-dim);
      transition: color 0.15s ease;
    }
    .cap-card.selected .cap-name {
      color: var(--text-bright, #fff);
    }
    .cap-desc {
      font-size: 11px;
      color: var(--text-dim);
      opacity: 0.7;
      line-height: 1.3;
      margin-top: 1px;
    }
    .cap-check {
      width: 18px; height: 18px;
      border-radius: 4px;
      border: 1.5px solid var(--border, #1A3550);
      display: flex; align-items: center; justify-content: center;
      font-size: 12px; flex-shrink: 0;
      transition: all 0.15s ease;
      color: transparent;
    }
    .cap-card.selected .cap-check {
      border-color: var(--accent, #14A8C4);
      background: var(--accent, #14A8C4);
      color: #fff;
    }

    /* ── Budget section ── */
    .budget-toggle {
      display: flex; align-items: center; gap: 6px;
      background: none; border: none;
      color: var(--text-dim);
      font-size: 12px; cursor: pointer;
      padding: 0; margin-bottom: 8px;
    }
    .budget-toggle:hover { color: var(--text); }
    .budget-chevron {
      display: inline-block;
      transition: transform 0.2s ease;
      font-size: 10px;
    }
    .budget-chevron.open { transform: rotate(90deg); }

    .budget-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 12px;
      padding: 12px;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
    }
    .budget-field label {
      display: block;
      font-size: 11px; color: var(--text-dim);
      margin-bottom: 4px;
    }
    .budget-field .input-wrap {
      position: relative;
      display: flex;
      align-items: center;
    }
    .budget-field .input-prefix {
      position: absolute;
      left: 8px;
      font-size: 13px;
      font-weight: 600;
      color: var(--text-dim);
      pointer-events: none;
      z-index: 1;
    }
    .budget-field input {
      width: 100%; box-sizing: border-box;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px;
      padding: 6px 8px;
      color: var(--text); font-size: 13px;
      font-family: var(--mono, monospace);
    }
    .budget-field input.has-prefix {
      padding-left: 22px;
    }
    .budget-field input:focus {
      outline: none;
      border-color: var(--accent, #14A8C4);
    }
    .budget-hint {
      font-size: 10px;
      color: var(--text-dim);
      opacity: 0.7;
      margin-top: 3px;
      font-family: var(--mono, monospace);
    }

    /* ── Actions ── */
    .actions {
      display: flex; gap: 8px;
      margin-bottom: 20px;
      flex-wrap: wrap;
    }
    .btn-primary { border-color: rgba(72,128,255,0.3); }
    .btn-secondary {
      background: var(--bg-elevated, #142D42);
      color: var(--text-dim);
    }
    .btn-secondary:hover:not(:disabled) {
      background: var(--border, #1A3550);
      color: var(--text);
    }

    .action-msg { font-size: 12px; color: var(--text-dim); margin-bottom: 12px; }
    .action-msg.error   { color: var(--error, #fd5454); }
    .action-msg.success { color: var(--success, #00b69b); }

    .action-txid {
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 12px;
      color: var(--text-dim);
      margin-bottom: 12px;
      font-family: var(--mono, monospace);
    }
    .action-txid .txid-label {
      color: var(--text-dim);
      font-size: 10px;
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }
    .action-txid a {
      color: var(--accent, #14A8C4);
      text-decoration: none;
    }
    .action-txid a:hover {
      text-decoration: underline;
    }

    /* ── Responsive ── */
    @media (max-width: 639px) {
      :host { padding: 16px 12px; }
      .cert-fields { grid-template-columns: 1fr; gap: 2px 0; }
      .field-label { margin-top: 8px; }
      .issue-form { padding: 16px; }
      .cap-grid { grid-template-columns: 1fr; }
      .budget-grid { grid-template-columns: 1fr; }
    }
  `];

  firstUpdated() {
    this.loadStatus();
  }

  private loadStatus() {
    this.ctrl.fetch(async () => {
      const res = await this.fetchFn('/certificates');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return res.json();
    });
  }

  private toggleCap(id: string) {
    const next = new Set(this.selectedCaps);
    if (next.has(id)) next.delete(id); else next.add(id);
    this.selectedCaps = next;
  }

  private toggleAllCaps() {
    if (this.selectedCaps.size === CAPABILITIES.length) {
      this.selectedCaps = new Set();
    } else {
      this.selectedCaps = new Set(CAPABILITIES.map(c => c.id));
    }
  }

  private get _isUsd(): boolean {
    return this.currencyMode === 'usd-first' && this.usdRate > 0;
  }

  private _toSats(value: string): number {
    if (!value) return 0;
    const n = parseFloat(value);
    if (isNaN(n) || n <= 0) return 0;
    return this._isUsd ? usdToSats(n, this.usdRate) : Math.round(n);
  }

  private _budgetConversion(value: string): string {
    if (!value || !this.usdRate || this.usdRate <= 0) return '';
    const n = parseFloat(value);
    if (isNaN(n) || n <= 0) return '';
    if (this._isUsd) {
      const sats = usdToSats(n, this.usdRate);
      return `\u2248 ${formatSats(sats)} sats`;
    }
    return `\u2248 ${formatUsd(Math.round(n), this.usdRate)}`;
  }

  private _renderBudgetField(label: string, value: string, onChange: (v: string) => void) {
    const usd = this._isUsd;
    const placeholder = usd
      ? (label === 'Per-Task' ? '0.10' : label === 'Per-Hour' ? '1.00' : '4.00')
      : (label === 'Per-Task' ? '50000' : label === 'Per-Hour' ? '5000000' : '20000000');
    const hint = this._budgetConversion(value);
    return html`
      <div class="budget-field">
        <label>${label} (${usd ? 'USD' : 'sats'})</label>
        <div class="input-wrap">
          ${usd ? html`<span class="input-prefix">$</span>` : nothing}
          <input type="number" placeholder=${placeholder}
            min="0" step=${usd ? '0.01' : '1'}
            class=${usd ? 'has-prefix' : ''}
            .value=${value}
            @input=${(e: Event) => { onChange((e.target as HTMLInputElement).value); this.requestUpdate(); }} />
        </div>
        ${hint ? html`<div class="budget-hint">${hint}</div>` : nothing}
      </div>
    `;
  }

  private _fmtCertBudget(satsStr: string): string {
    const sats = parseInt(satsStr || '0');
    if (!sats) return satsStr;
    if (this._isUsd) {
      const usd = formatUsd(sats, this.usdRate);
      return `${usd} (${formatSats(sats)} sats)`;
    }
    const alt = this.usdRate > 0 ? ` (${formatUsd(sats, this.usdRate)})` : '';
    return `${formatSats(sats)} sats${alt}`;
  }

  private async issueCert() {
    this.actionLoading = true;
    this.actionMessage = '';
    this.actionTxid = '';
    try {
      const body: Record<string, unknown> = {};
      if (this.agentName.trim()) body['name'] = this.agentName.trim();
      body['capabilities'] = this.selectedCaps.size > 0
        ? Array.from(this.selectedCaps).join(',')
        : 'llm,tools,messaging,x402';
      const taskBudget = this._toSats(this.budgetPerTask);
      const hourBudget = this._toSats(this.budgetPerHour);
      const dayBudget = this._toSats(this.budgetPerDay);
      if (taskBudget > 0) body['budget_per_task'] = taskBudget;
      if (hourBudget > 0) body['budget_per_hour'] = hourBudget;
      if (dayBudget > 0) body['budget_per_day'] = dayBudget;

      const res = await this.fetchFn('/certificates/issue', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      const data = await res.json();
      if (data.error) {
        this.actionMessage = `error:${data.error}`;
      } else {
        if (data.txid) this.actionTxid = data.txid;
        this.actionMessage = 'success:Parent certificate issued';
        this.agentName = '';
        this.selectedCaps = new Set(DEFAULT_CAPS);
        this.budgetPerTask = '';
        this.budgetPerHour = '';
        this.budgetPerDay = '';
        this.showBudgetInputs = false;
        await this.loadStatus();
      }
    } catch (e) {
      this.actionMessage = `error:${e instanceof Error ? e.message : 'Failed'}`;
    } finally {
      this.actionLoading = false;
    }
  }

  private async relinquishCert() {
    this.actionLoading = true;
    this.actionMessage = '';
    this.actionTxid = '';
    try {
      const res = await this.fetchFn('/certificates/relinquish', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{}',
      });
      const data = await res.json();
      if (data.error) {
        this.actionMessage = `error:${data.error}`;
      } else {
        this.actionMessage = 'success:Certificate relinquished';
        await this.loadStatus();
      }
    } catch (e) {
      this.actionMessage = `error:${e instanceof Error ? e.message : 'Failed'}`;
    } finally {
      this.actionLoading = false;
    }
  }

  private async revokeCert() {
    this.actionLoading = true;
    this.actionMessage = '';
    this.actionTxid = '';
    this.showRevokeConfirm = false;
    try {
      const res = await this.fetchFn('/certificates/revoke', { method: 'POST' });
      const data = await res.json();
      if (data.error) {
        this.actionMessage = `error:${data.error}`;
      } else {
        if (data.txid) this.actionTxid = data.txid;
        this.actionMessage = 'success:Certificate revoked';
        await this.loadStatus();
      }
    } catch (e) {
      this.actionMessage = `error:${e instanceof Error ? e.message : 'Revocation failed'}`;
    } finally {
      this.actionLoading = false;
    }
  }

  private renderStatusCard() {
    if (!this.ctrl.data) return nothing;

    const s = this.ctrl.data;
    const statusClass = s.status;
    const cert = s.certificate;

    const labels: Record<string, { label: string; sub: string; icon: string }> = {
      'parent-signed': { label: 'Authorized by Parent', sub: 'Trust chain established', icon: '\u2713' },
      'self-signed': { label: 'Bootstrap (Self-Signed)', sub: 'No parent verification', icon: '!' },
      'none': { label: 'No Certificate', sub: 'Agent is uncertified', icon: '\u2717' },
    };

    const info = labels[s.status] ?? labels['none'];
    const isRevoked = s.is_revoked === true;
    const showRevocationBadge = s.status !== 'none';

    return html`
      <div class="status-card ${statusClass}">
        <div class="status-header">
          <div class="status-icon ${statusClass}">${info.icon}</div>
          <div>
            <div class="status-label">${info.label}</div>
            <div class="status-sublabel">${info.sub}</div>
          </div>
          ${showRevocationBadge
            ? html`<span class="revocation-badge ${isRevoked ? 'revoked' : 'active'}">
                ${isRevoked ? '\u2717 Revoked' : '\u2713 Active'}
              </span>`
            : nothing}
        </div>

        ${cert ? html`
          <div class="cert-fields">
            ${cert.certifier ? html`
              <span class="field-label">Certifier</span>
              <span class="field-value" title=${String(cert.certifier)}>${truncateKey(String(cert.certifier))}</span>
            ` : nothing}
            ${cert.subject ? html`
              <span class="field-label">Subject</span>
              <span class="field-value" title=${String(cert.subject)}>${truncateKey(String(cert.subject))}</span>
            ` : nothing}
            ${((cert.fields ?? {}) as Record<string, string>)?.name ? html`
              <span class="field-label">Agent Name</span>
              <span class="field-value">${((cert.fields ?? {}) as Record<string, string>).name}</span>
            ` : nothing}
            ${((cert.fields ?? {}) as Record<string, string>)?.capabilities ? html`
              <span class="field-label">Capabilities</span>
              <span class="field-value">${((cert.fields ?? {}) as Record<string, string>).capabilities}</span>
            ` : nothing}
            ${((cert.fields ?? {}) as Record<string, string>)?.budget_per_task ? html`
              <span class="field-label">Budget / Task</span>
              <span class="field-value">${this._fmtCertBudget(((cert.fields ?? {}) as Record<string, string>).budget_per_task)}</span>
            ` : nothing}
            ${((cert.fields ?? {}) as Record<string, string>)?.budget_per_hour ? html`
              <span class="field-label">Budget / Hour</span>
              <span class="field-value">${this._fmtCertBudget(((cert.fields ?? {}) as Record<string, string>).budget_per_hour)}</span>
            ` : nothing}
            ${((cert.fields ?? {}) as Record<string, string>)?.budget_per_day ? html`
              <span class="field-label">Budget / Day</span>
              <span class="field-value">${this._fmtCertBudget(((cert.fields ?? {}) as Record<string, string>).budget_per_day)}</span>
            ` : nothing}
            ${((cert.fields ?? {}) as Record<string, string>)?.deployed_at ? html`
              <span class="field-label">Deployed</span>
              <span class="field-value">${((cert.fields ?? {}) as Record<string, string>).deployed_at}</span>
            ` : nothing}
            ${cert.serialNumber ? html`
              <span class="field-label">Serial</span>
              <span class="field-value">${cert.serialNumber}</span>
            ` : nothing}
            ${(cert as any).revocationOutpoint && String((cert as any).revocationOutpoint).length >= 64 && String((cert as any).revocationOutpoint) !== '0'.repeat(72) ? (() => {
              const op = String((cert as any).revocationOutpoint);
              const txid = op.slice(0, 64);
              return html`
                <span class="field-label">TX ID</span>
                <span class="field-value"><a class="txid-link" href="https://whatsonchain.com/tx/${txid}" target="_blank" rel="noopener" title=${txid}>${txid.slice(0, 12)}\u2026${txid.slice(-8)}</a></span>
              `;
            })() : nothing}
          </div>

          <button class="toggle-raw" @click=${() => { this.showRawCert = !this.showRawCert; }}>
            ${this.showRawCert ? 'Hide' : 'Show'} raw certificate
          </button>
          ${this.showRawCert ? html`
            <div class="raw-cert">
              <pre>${JSON.stringify(cert, null, 2)}</pre>
            </div>
          ` : nothing}
        ` : nothing}
      </div>
    `;
  }

  private renderIssueForm() {
    return html`
      <div class="issue-form">
        <h3 class="form-title">Issue New Certificate</h3>

        <!-- Agent Name -->
        <div class="form-section">
          <div class="form-label-row">
            <span class="form-label">Agent Name</span>
          </div>
          <input class="form-input" type="text"
            placeholder="lobster-agent"
            .value=${this.agentName}
            @input=${(e: Event) => { this.agentName = (e.target as HTMLInputElement).value; }} />
          <div class="form-hint">Leave blank to use the server default</div>
        </div>

        <!-- Capabilities -->
        <div class="form-section">
          <div class="form-label-row">
            <span class="form-label">Capabilities</span>
            <button class="select-toggle" @click=${this.toggleAllCaps}>
              ${this.selectedCaps.size === CAPABILITIES.length ? 'Deselect All' : 'Select All'}
            </button>
          </div>
          <div class="cap-grid">
            ${CAPABILITIES.map(cap => html`
              <div class="cap-card ${this.selectedCaps.has(cap.id) ? 'selected' : ''}"
                   tabindex="0"
                   @click=${() => this.toggleCap(cap.id)}
                   @keydown=${(e: KeyboardEvent) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); this.toggleCap(cap.id); } }}>
                <div class="cap-icon" style="background:${cap.color}20;color:${cap.color}">
                  ${cap.icon}
                </div>
                <div class="cap-info">
                  <div class="cap-name">${cap.label}</div>
                  <div class="cap-desc">${cap.desc}</div>
                </div>
                <div class="cap-check">\u2713</div>
              </div>
            `)}
          </div>
        </div>

        <!-- Budget limits -->
        <div class="form-section">
          <button class="budget-toggle" @click=${() => { this.showBudgetInputs = !this.showBudgetInputs; this.requestUpdate(); }}>
            <span class="budget-chevron ${this.showBudgetInputs ? 'open' : ''}">\u25B6</span>
            Budget Limits (optional)
          </button>
          ${this.showBudgetInputs ? html`
            <div class="budget-grid">
              ${this._renderBudgetField('Per-Task', this.budgetPerTask, (v: string) => { this.budgetPerTask = v; })}
              ${this._renderBudgetField('Per-Hour', this.budgetPerHour, (v: string) => { this.budgetPerHour = v; })}
              ${this._renderBudgetField('Per-Day', this.budgetPerDay, (v: string) => { this.budgetPerDay = v; })}
            </div>
          ` : nothing}
        </div>
      </div>
    `;
  }

  render() {
    if (this.ctrl.loading) {
      return renderLoading('Loading certificate status...');
    }

    if (this.ctrl.hasError) {
      return renderError(this.ctrl.error ?? 'Failed to load certificate status', 'Check that the agent server is running.');
    }

    const s = this.ctrl.data;
    const hasCert = s && s.status !== 'none';
    const isRevoked = s?.is_revoked === true;
    const showIssue = !s || s.status === 'self-signed' || s.status === 'none' || isRevoked;

    // Parse action message
    let msgClass = '';
    let msgText = '';
    if (this.actionMessage) {
      const [type, ...rest] = this.actionMessage.split(':');
      msgClass = type;
      msgText = rest.join(':');
    }

    return html`
      <div class="page-title">Certificates</div>

      ${this.renderStatusCard()}

      ${msgText ? html`<div class="action-msg ${msgClass}">${msgText}</div>` : nothing}
      ${this.actionTxid ? html`
        <div class="action-txid">
          <span class="txid-label">TX</span>
          <a href="https://whatsonchain.com/tx/${this.actionTxid}"
             target="_blank" rel="noopener"
             title=${this.actionTxid}>${this.actionTxid.slice(0, 12)}\u2026${this.actionTxid.slice(-8)}</a>
        </div>
      ` : nothing}

      ${showIssue ? this.renderIssueForm() : nothing}

      <div class="actions">
        ${showIssue ? html`
          <button
            class="btn btn-primary"
            ?disabled=${this.actionLoading || this.selectedCaps.size === 0}
            @click=${this.issueCert}
          >${this.actionLoading ? 'Issuing...' : 'Issue Parent Certificate'}</button>
        ` : nothing}

        ${hasCert ? html`
          <button
            class="btn btn-danger"
            ?disabled=${this.actionLoading}
            @click=${this.relinquishCert}
          >${this.actionLoading ? 'Relinquishing...' : 'Relinquish'}</button>
        ` : nothing}

        ${s?.status === 'parent-signed' && !isRevoked ? html`
          ${this.showRevokeConfirm ? html`
            <div class="revoke-confirm">
              <span class="revoke-warning">This will permanently revoke the agent's certificate. The agent will lose all capabilities until a new certificate is issued.</span>
              <div class="revoke-actions">
                <button class="btn btn-danger" ?disabled=${this.actionLoading} @click=${this.revokeCert}>
                  ${this.actionLoading ? 'Revoking...' : 'Confirm Revoke'}
                </button>
                <button class="btn btn-secondary" @click=${() => { this.showRevokeConfirm = false; }}>Cancel</button>
              </div>
            </div>
          ` : html`
            <button class="btn btn-danger" ?disabled=${this.actionLoading} @click=${() => { this.showRevokeConfirm = true; }}>
              Revoke
            </button>
          `}
        ` : nothing}

        <button
          class="btn btn-secondary"
          ?disabled=${this.actionLoading}
          @click=${this.loadStatus}
        >Refresh</button>
      </div>
    `;
  }
}
