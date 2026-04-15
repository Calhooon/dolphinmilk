/**
 * First-run onboarding overlay for the chat page.
 * Shows when no conversations exist and the user hasn't dismissed it.
 * Displays agent capabilities, wallet balance, and suggested prompts.
 */

import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatSats } from '../lib/util.js';
import { fetchBsvUsdRate, formatDualCurrency } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay, markOnboardingSeen } from '../lib/storage.js';
import type { AgentInfo } from '../lib/shared-types.js';

/** Suggested prompts with approximate cost. */
const SUGGESTED_PROMPTS = [
  { text: 'What is BSV and how do micropayments work?', cost: '~200 sats', icon: '\u{1F4AC}' },
  { text: 'Generate an image of a lobster on the moon', cost: '~$0.19', icon: '\u{1F5BC}' },
  { text: 'Search Twitter for the latest BSV news', cost: '~$0.06', icon: '\u{1F50D}' },
  { text: 'What services are available via x402?', cost: 'free', icon: '\u{1F310}' },
];

/** Capability pills. */
const CAPABILITIES = [
  { label: 'LLM Reasoning', desc: 'GPT-5, Claude via x402', icon: '\u{1F9E0}' },
  { label: 'Web Research', desc: 'Fetch, scrape, browse', icon: '\u{1F310}' },
  { label: 'Micropayments', desc: 'Pay-per-call via x402', icon: '\u{1F4B0}' },
  { label: 'On-chain Proofs', desc: 'BRC-18 audit trail', icon: '\u{1F512}' },
  { label: 'Persistent Memory', desc: 'Cross-session recall', icon: '\u{1F4DD}' },
  { label: 'Automations', desc: 'Scheduled recurring tasks', icon: '\u23F0' },
];

@customElement('dm-onboarding')
export class WormOnboarding extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  @state() private agent: AgentInfo | null = null;
  @state() private usdRate = 0;
  @state() private visible = true;
  @state() private fundingAddress = '';
  @state() private copied = false;
  @state() private checking = false;

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      flex: 1;
      padding: 24px;
      overflow-y: auto;
    }

    .onboarding {
      max-width: 560px;
      width: 100%;
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 28px;
      animation: onboard-in 0.4s ease;
    }

    @keyframes onboard-in {
      from { opacity: 0; transform: translateY(12px); }
      to { opacity: 1; transform: translateY(0); }
    }

    @media (prefers-reduced-motion: reduce) {
      .onboarding { animation: none; }
    }

    /* Hero */
    .hero {
      text-align: center;
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 12px;
    }

    .logo-mark {
      width: 48px;
      height: 48px;
      border-radius: 12px;
      overflow: hidden;
    }

    .logo-mark img {
      width: 100%;
      height: 100%;
      object-fit: contain;
    }

    .hero-title {
      font-size: 24px;
      font-weight: 700;
      color: var(--text-bright, #fff);
      font-family: var(--sans, sans-serif);
    }

    .hero-title .accent {
      color: var(--accent, #14A8C4);
    }

    .hero-subtitle {
      font-size: 14px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      line-height: 1.6;
      max-width: 440px;
    }

    /* Wallet status card */
    .wallet-card {
      display: flex;
      align-items: center;
      gap: 16px;
      padding: 14px 20px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 12px;
      width: 100%;
    }

    .wallet-icon {
      width: 40px;
      height: 40px;
      border-radius: 10px;
      background: rgba(252, 190, 45, 0.12);
      color: var(--warning, #fcbe2d);
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 20px;
      flex-shrink: 0;
    }

    .wallet-info {
      flex: 1;
      min-width: 0;
    }

    .wallet-balance {
      font-size: 18px;
      font-weight: 700;
      font-family: var(--mono, monospace);
      color: var(--warning, #fcbe2d);
    }

    .wallet-detail {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      display: flex;
      gap: 12px;
      margin-top: 2px;
    }

    .wallet-detail span {
      display: inline-flex;
      align-items: center;
      gap: 4px;
    }

    /* Empty wallet state */
    .wallet-card.empty {
      border-color: rgba(252, 190, 45, 0.3);
      background: rgba(252, 190, 45, 0.06);
    }

    .wallet-card.empty .wallet-icon {
      background: rgba(252, 190, 45, 0.15);
      color: var(--warning, #fcbe2d);
    }

    .empty-balance {
      color: var(--text-dim, rgba(255,255,255,0.5)) !important;
    }

    /* Funding guide */
    .funding-guide {
      width: 100%;
      padding: 20px 24px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 12px;
      display: flex;
      flex-direction: column;
      gap: 16px;
    }

    .funding-title {
      font-size: 15px;
      font-weight: 700;
      color: var(--text-bright, #F5F8FA);
    }

    .funding-steps {
      display: flex;
      flex-direction: column;
      gap: 10px;
    }

    .funding-step {
      display: flex;
      align-items: flex-start;
      gap: 12px;
      font-size: 13px;
      color: var(--text, rgba(255,255,255,0.8));
      line-height: 1.5;
    }

    .step-num {
      width: 22px;
      height: 22px;
      border-radius: 50%;
      background: var(--accent, #14A8C4);
      color: #fff;
      font-size: 12px;
      font-weight: 700;
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;
      margin-top: 1px;
    }

    .funding-step code {
      font-family: var(--mono, monospace);
      font-size: 12px;
      background: rgba(255, 255, 255, 0.06);
      padding: 2px 6px;
      border-radius: 4px;
      color: var(--accent, #14A8C4);
    }

    .funding-cost {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      padding-top: 4px;
      border-top: 1px solid var(--border, #1A3550);
    }

    /* Address card */
    .address-card {
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      padding: 14px 16px;
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .address-label {
      font-size: 12px;
      font-weight: 600;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }

    .address-row {
      display: flex;
      align-items: center;
      gap: 8px;
    }

    .address-value {
      flex: 1;
      font-family: var(--mono, monospace);
      font-size: 14px;
      color: var(--accent, #14A8C4);
      word-break: break-all;
      background: none;
      padding: 0;
    }

    .copy-btn {
      padding: 6px 14px;
      background: var(--accent, #14A8C4);
      border: none;
      border-radius: 6px;
      color: #fff;
      font-size: 12px;
      font-weight: 600;
      font-family: inherit;
      cursor: pointer;
      flex-shrink: 0;
      transition: opacity 0.15s ease;
    }

    .copy-btn:hover { opacity: 0.85; }

    /* Funding actions */
    .funding-actions {
      display: flex;
      gap: 8px;
    }

    .refresh-btn {
      padding: 8px 16px;
      background: rgba(255, 255, 255, 0.04);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 13px;
      font-weight: 600;
      font-family: inherit;
      cursor: pointer;
      transition: background 0.15s ease, border-color 0.15s ease;
    }

    .refresh-btn:hover {
      border-color: var(--accent, #14A8C4);
      color: var(--text, rgba(255,255,255,0.8));
    }

    .refresh-btn.primary {
      background: rgba(20, 168, 196, 0.12);
      border-color: rgba(20, 168, 196, 0.3);
      color: var(--accent, #14A8C4);
    }

    .refresh-btn.primary:hover {
      background: rgba(20, 168, 196, 0.2);
    }

    .refresh-btn:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }

    /* Capabilities grid */
    .capabilities {
      width: 100%;
    }

    .cap-label {
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.6px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-bottom: 10px;
    }

    .cap-grid {
      display: grid;
      grid-template-columns: repeat(3, 1fr);
      gap: 8px;
    }

    .cap-pill {
      display: flex;
      flex-direction: column;
      gap: 2px;
      padding: 10px 12px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      transition: border-color 0.15s ease;
    }

    .cap-pill:hover {
      border-color: rgba(255, 255, 255, 0.1);
    }

    .cap-pill-name {
      font-size: 12px;
      font-weight: 600;
      color: var(--text, rgba(255,255,255,0.8));
      display: flex;
      align-items: center;
      gap: 6px;
    }

    .cap-pill-icon {
      font-size: 14px;
    }

    .cap-pill-desc {
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    /* Suggested prompts */
    .prompts {
      width: 100%;
    }

    .prompts-label {
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.6px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-bottom: 10px;
    }

    .prompt-list {
      display: flex;
      flex-direction: column;
      gap: 6px;
    }

    .prompt-item {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 10px 14px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 10px;
      cursor: pointer;
      transition: border-color 0.15s ease, background 0.15s ease, transform 0.1s ease;
      text-decoration: none;
      color: inherit;
    }

    .prompt-item:hover {
      border-color: var(--accent, #14A8C4);
      background: rgba(20, 168, 196, 0.04);
    }

    .prompt-item:active {
      transform: scale(0.99);
    }

    .prompt-icon {
      font-size: 16px;
      flex-shrink: 0;
    }

    .prompt-text {
      flex: 1;
      font-size: 13px;
      color: var(--text, rgba(255,255,255,0.8));
      min-width: 0;
    }

    .prompt-cost {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.5));
      flex-shrink: 0;
      padding: 2px 8px;
      background: rgba(255, 255, 255, 0.04);
      border-radius: 4px;
    }

    /* Footer links */
    .footer-links {
      display: flex;
      gap: 16px;
    }

    .footer-link {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-decoration: none;
      transition: color 0.15s ease;
    }

    .footer-link:hover {
      color: var(--accent, #14A8C4);
    }

    /* Responsive */
    @media (max-width: 639px) {
      :host { padding: 16px 12px; }
      .hero-title { font-size: 20px; }
      .cap-grid { grid-template-columns: repeat(2, 1fr); }
      .wallet-card { flex-direction: column; text-align: center; gap: 10px; }
      .wallet-detail { justify-content: center; }
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    this.loadAgentData();
  }

  private async loadAgentData() {
    try {
      const [agentRes, rate] = await Promise.all([
        this.fetchFn('/agent'),
        fetchBsvUsdRate(),
      ]);
      this.usdRate = rate;
      if (agentRes.ok) {
        this.agent = await agentRes.json();
      }
      // Fetch funding address if balance is 0
      if ((this.agent?.balance ?? 0) === 0) {
        try {
          const addrRes = await fetch('/wallet/address');
          if (addrRes.ok) {
            const data = await addrRes.json();
            this.fundingAddress = data.address ?? '';
          }
        } catch { /* best effort */ }
      }
    } catch {
      // Best effort — onboarding still renders without data
    }
  }

  private async copyAddress() {
    if (!this.fundingAddress) return;
    try {
      await navigator.clipboard.writeText(this.fundingAddress);
      this.copied = true;
      setTimeout(() => { this.copied = false; }, 2000);
    } catch { /* fallback: select text */ }
  }

  private async checkForFunding() {
    this.checking = true;
    try {
      const res = await fetch('/wallet/check-funding', { method: 'POST' });
      if (res.ok) {
        const data = await res.json();
        if (data.balance > 0) {
          // Reload agent data to reflect new balance
          await this.loadAgentData();
        }
      }
    } catch { /* best effort */ }
    this.checking = false;
  }

  private handlePromptClick(text: string) {
    markOnboardingSeen();
    this.dispatchEvent(new CustomEvent('send-prompt', {
      detail: { text },
      bubbles: true,
      composed: true,
    }));
  }

  private handleDismiss() {
    markOnboardingSeen();
    this.visible = false;
    this.dispatchEvent(new CustomEvent('dismiss', {
      bubbles: true,
      composed: true,
    }));
  }

  render() {
    if (!this.visible) return html``;

    const balance = this.agent?.balance ?? 0;
    const toolCount = this.agent?.tools?.length ?? 0;
    const funded = balance > 0;
    const dual = this.usdRate
      ? formatDualCurrency(balance, this.usdRate, this.currencyMode)
      : null;

    return html`
      <div class="onboarding">
        <!-- Hero -->
        <div class="hero">
          <div class="logo-mark">
            <img src="${import.meta.env.BASE_URL}logo.png" alt="Dolphin Milk">
          </div>
          <div class="hero-title">Welcome to <span class="accent">Dolphin Milk</span></div>
          <div class="hero-subtitle">
            ${funded
              ? 'Your agent has a wallet, tools, and services. Every thought costs real money. Every action creates a verifiable on-chain proof.'
              : 'An AI agent that pays for its own inference. Fund the wallet to get started.'}
          </div>
        </div>

        <!-- Wallet status -->
        <div class="wallet-card ${funded ? '' : 'empty'}">
          <div class="wallet-icon">${funded ? '\u26BF' : '\u26A0'}</div>
          <div class="wallet-info">
            ${funded ? html`
              <div class="wallet-balance">
                ${dual ? dual.primary : `${formatSats(balance)} sats`}
              </div>
              <div class="wallet-detail">
                ${dual ? html`<span>${dual.secondary}</span>` : ''}
                <span>${toolCount} tools ready</span>
              </div>
            ` : html`
              <div class="wallet-balance empty-balance">0 sats</div>
              <div class="wallet-detail">
                <span>Wallet is empty \u2014 fund it to use the agent</span>
              </div>
            `}
          </div>
        </div>

        ${!funded ? html`
          <!-- Funding guide (shown when balance is 0) -->
          <div class="funding-guide">
            <div class="funding-title">Fund your agent</div>

            ${this.fundingAddress ? html`
              <div class="address-card">
                <div class="address-label">Send BSV to this address:</div>
                <div class="address-row">
                  <code class="address-value">${this.fundingAddress}</code>
                  <button class="copy-btn" @click=${() => this.copyAddress()}>
                    ${this.copied ? 'Copied!' : 'Copy'}
                  </button>
                </div>
              </div>

              <div class="funding-steps">
                <div class="funding-step">
                  <span class="step-num">1</span>
                  <span>Send BSV to the address above from any wallet or exchange</span>
                </div>
                <div class="funding-step">
                  <span class="step-num">2</span>
                  <span>Click <strong>Check for payment</strong> below (may take a few minutes to confirm)</span>
                </div>
              </div>
            ` : html`
              <div class="funding-steps">
                <div class="funding-step">
                  <span class="step-num">1</span>
                  <span>Run <code>dolphin-milk receive</code> to get a BSV address</span>
                </div>
                <div class="funding-step">
                  <span class="step-num">2</span>
                  <span>Send BSV to that address from any wallet or exchange</span>
                </div>
                <div class="funding-step">
                  <span class="step-num">3</span>
                  <span>Run <code>dolphin-milk fund &lt;TXID&gt;</code> to internalize it</span>
                </div>
              </div>
            `}

            <div class="funding-cost">
              ~200 sats per LLM call. A few dollars gets you thousands of calls.
            </div>
            <div class="funding-actions">
              <button class="refresh-btn primary" @click=${() => this.checkForFunding()} ?disabled=${this.checking}>
                ${this.checking ? 'Checking...' : 'Check for payment'}
              </button>
              <button class="refresh-btn" @click=${() => this.loadAgentData()}>
                Refresh balance
              </button>
            </div>
          </div>
        ` : ''}

        <!-- Capabilities -->
        <div class="capabilities">
          <div class="cap-label">What your agent can do</div>
          <div class="cap-grid">
            ${CAPABILITIES.map(cap => html`
              <div class="cap-pill">
                <span class="cap-pill-name">
                  <span class="cap-pill-icon">${cap.icon}</span>
                  ${cap.label}
                </span>
                <span class="cap-pill-desc">${cap.desc}</span>
              </div>
            `)}
          </div>
        </div>

        ${funded ? html`
          <!-- Suggested prompts (only when funded) -->
          <div class="prompts">
            <div class="prompts-label">Try something</div>
            <div class="prompt-list">
              ${SUGGESTED_PROMPTS.map(p => html`
                <div class="prompt-item" @click=${() => this.handlePromptClick(p.text)}>
                  <span class="prompt-icon">${p.icon}</span>
                  <span class="prompt-text">${p.text}</span>
                  <span class="prompt-cost">${p.cost}</span>
                </div>
              `)}
            </div>
          </div>
        ` : ''}

        <!-- Footer links -->
        <div class="footer-links">
          <a class="footer-link" href="#agent">Agent Capabilities</a>
          <a class="footer-link" href="#agent/tools">Browse Tools</a>
          <a class="footer-link" href="#budget">View Budget</a>
        </div>
      </div>
    `;
  }
}
