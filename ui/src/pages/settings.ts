/**
 * Settings page — surfaces common agent configuration and client-side preferences.
 * Phase 1: Read-only display of server config + editable currency toggle.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { pageHost, centerState, pageTitle, sectionTitle, stateFeedback, buttonStyles, renderLoading, renderError } from '../lib/shared-styles.js';
import { getModel, getCurrencyDisplay, setCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import { fetchBsvUsdRate, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { truncateKey, formatSats } from '../lib/util.js';
import type { AgentInfo, BudgetReport } from '../lib/shared-types.js';

interface HealthInfo {
  version: string;
  uptime_secs: number;
  status: string;
}

@customElement('dm-settings')
export class WormSettings extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @property() identityKey: string | null = null;

  @state() private loading = true;
  @state() private error = '';
  @state() private agent: AgentInfo | null = null;
  @state() private budget: BudgetReport | null = null;
  @state() private health: HealthInfo | null = null;
  @state() private usdRate = 0;
  @state() private connected = false;
  @state() private copyFeedback = false;

  static styles = [
    pageHost, centerState, pageTitle, sectionTitle, stateFeedback, buttonStyles,
    css`
      .settings-container {
        max-width: 720px;
      }

      section {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: var(--radius-md, 14px);
        margin-bottom: 16px;
        overflow: hidden;
      }

      .section-header {
        padding: 16px 20px 0;
      }

      .setting-row {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 12px 20px;
        border-bottom: 1px solid var(--border, #1A3550);
        gap: 16px;
      }

      .setting-row:last-child {
        border-bottom: none;
      }

      .setting-label {
        color: var(--text, rgba(255,255,255,0.8));
        font-size: 13px;
        font-weight: 600;
        white-space: nowrap;
        flex-shrink: 0;
      }

      .setting-value {
        color: var(--text-dim, rgba(255,255,255,0.5));
        font-size: 13px;
        text-align: right;
        word-break: break-all;
        min-width: 0;
      }

      .setting-value.mono {
        font-family: var(--mono, monospace);
      }

      .setting-value.bright {
        color: var(--text-bright, #fff);
      }

      .setting-value.sats {
        color: var(--warning, #fcbe2d);
      }

      .setting-value.success {
        color: var(--success, #00b69b);
      }

      .setting-value.error {
        color: var(--error, #fd5454);
      }

      .identity-row {
        display: flex;
        align-items: center;
        gap: 8px;
        min-width: 0;
        text-align: right;
        justify-content: flex-end;
      }

      .identity-key {
        font-size: 12px;
        font-family: var(--mono, monospace);
        color: var(--text-bright, #fff);
        word-break: break-all;
        min-width: 0;
      }

      .copy-btn {
        background: none;
        border: 1px solid var(--border, #1A3550);
        border-radius: 6px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        cursor: pointer;
        padding: 4px 8px;
        font-size: 11px;
        font-family: var(--mono, monospace);
        transition: color 0.15s ease, border-color 0.15s ease;
        flex-shrink: 0;
        white-space: nowrap;
      }

      .copy-btn:hover {
        color: var(--accent, #14A8C4);
        border-color: var(--accent, #14A8C4);
      }

      .copy-btn.copied {
        color: var(--success, #00b69b);
        border-color: var(--success, #00b69b);
      }

      .conn-dot {
        display: inline-block;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: var(--error, #fd5454);
        flex-shrink: 0;
      }

      .conn-dot.on {
        background: var(--success, #00b69b);
      }

      .toggle-group {
        display: flex;
        gap: 0;
        border: 1px solid var(--border, #1A3550);
        border-radius: 6px;
        overflow: hidden;
      }

      .toggle-option {
        padding: 5px 14px;
        font-size: 12px;
        font-weight: 600;
        font-family: var(--mono, monospace);
        background: transparent;
        border: none;
        color: var(--text-dim, rgba(255,255,255,0.5));
        cursor: pointer;
        transition: background 0.15s ease, color 0.15s ease;
      }

      .toggle-option:not(:last-child) {
        border-right: 1px solid var(--border, #1A3550);
      }

      .toggle-option.active {
        background: rgba(20, 168, 196, 0.12);
        color: var(--accent, #14A8C4);
      }

      .toggle-option:hover:not(.active) {
        background: rgba(255, 255, 255, 0.03);
        color: var(--text, rgba(255,255,255,0.8));
      }

      .config-note {
        color: var(--text-dim, rgba(255,255,255,0.4));
        font-size: 12px;
        font-style: italic;
      }

      .footer-note {
        padding: 16px 0;
        text-align: center;
        color: var(--text-dim, rgba(255,255,255,0.4));
        font-size: 12px;
        line-height: 1.6;
      }

      .footer-note .version {
        font-family: var(--mono, monospace);
        color: var(--text-dim, rgba(255,255,255,0.5));
      }

      @media (max-width: 639px) {
        .setting-row {
          flex-direction: column;
          align-items: flex-start;
          gap: 6px;
        }

        .setting-value {
          text-align: left;
        }

        .identity-row {
          justify-content: flex-start;
        }
      }
    `,
  ];

  async connectedCallback() {
    super.connectedCallback();
    await this.loadData();
  }

  private async loadData() {
    this.loading = true;
    this.error = '';

    try {
      // Fetch all data sources in parallel
      const [agentRes, budgetRes, healthRes, rate] = await Promise.all([
        this.fetchFn('/agent').catch(() => null),
        this.fetchFn('/budget').catch(() => null),
        fetch('/health').catch(() => null),
        fetchBsvUsdRate(),
      ]);

      this.usdRate = rate;

      if (agentRes?.ok) {
        this.agent = await agentRes.json();
      }

      if (budgetRes?.ok) {
        this.budget = await budgetRes.json();
      }

      if (healthRes?.ok) {
        this.health = await healthRes.json();
        this.connected = true;
      } else {
        this.connected = false;
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : 'Failed to load settings data';
    } finally {
      this.loading = false;
    }
  }

  private copyIdentityKey() {
    const key = this.identityKey ?? this.agent?.identity_key;
    if (!key) return;
    navigator.clipboard.writeText(key).then(() => {
      this.copyFeedback = true;
      setTimeout(() => { this.copyFeedback = false; }, 1500);
    }).catch(() => {});
  }

  private handleCurrencyToggle(mode: CurrencyDisplay) {
    this.currencyMode = mode;
    setCurrencyDisplay(mode);
    this.dispatchEvent(new CustomEvent('currency-change', {
      detail: { mode },
      bubbles: true,
      composed: true,
    }));
  }

  private renderAgentSection() {
    const model = getModel();
    return html`
      <section>
        <div class="section-header">
          <h2 class="section-title">Agent</h2>
        </div>
        <div class="setting-row">
          <span class="setting-label">Default Model</span>
          <span class="setting-value mono">${model}</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Max Iterations</span>
          <span class="setting-value mono">${this.agent ? '50' : html`<span class="config-note">Configured in dolphin-milk.toml</span>`}</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Context Window</span>
          <span class="setting-value mono">${this.agent ? '128,000 tokens' : html`<span class="config-note">Configured in dolphin-milk.toml</span>`}</span>
        </div>
      </section>
    `;
  }

  private renderBudgetSection() {
    if (!this.budget) {
      return html`
        <section>
          <div class="section-header">
            <h2 class="section-title">Budget Limits</h2>
          </div>
          <div class="setting-row">
            <span class="setting-label">Status</span>
            <span class="config-note">Budget data unavailable</span>
          </div>
        </section>
      `;
    }

    const limits = this.budget.limits;
    const rate = this.usdRate;
    const cm = this.currencyMode;

    return html`
      <section>
        <div class="section-header">
          <h2 class="section-title">Budget Limits</h2>
        </div>
        <div class="setting-row">
          <span class="setting-label">Per Task</span>
          <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(limits.max_per_task, rate, cm)}">
            ${formatInlineCurrency(limits.max_per_task, rate, cm)}
          </span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Per Hour</span>
          <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(limits.max_per_hour, rate, cm)}">
            ${formatInlineCurrency(limits.max_per_hour, rate, cm)}
          </span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Per Day</span>
          <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(limits.max_per_day, rate, cm)}">
            ${formatInlineCurrency(limits.max_per_day, rate, cm)}
          </span>
        </div>
        ${limits.max_per_week > 0 ? html`
          <div class="setting-row">
            <span class="setting-label">Per Week</span>
            <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(limits.max_per_week, rate, cm)}">
              ${formatInlineCurrency(limits.max_per_week, rate, cm)}
            </span>
          </div>
        ` : nothing}
        ${limits.max_per_month > 0 ? html`
          <div class="setting-row">
            <span class="setting-label">Per Month</span>
            <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(limits.max_per_month, rate, cm)}">
              ${formatInlineCurrency(limits.max_per_month, rate, cm)}
            </span>
          </div>
        ` : nothing}
        ${limits.max_lifetime > 0 ? html`
          <div class="setting-row">
            <span class="setting-label">Lifetime</span>
            <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(limits.max_lifetime, rate, cm)}">
              ${formatInlineCurrency(limits.max_lifetime, rate, cm)}
            </span>
          </div>
        ` : nothing}
        <div class="setting-row">
          <span class="setting-label">Enforcement</span>
          <span class="setting-value mono">${limits.enforcement}</span>
        </div>
      </section>
    `;
  }

  private renderDisplaySection() {
    return html`
      <section>
        <div class="section-header">
          <h2 class="section-title">Display</h2>
        </div>
        <div class="setting-row">
          <span class="setting-label">Currency Display</span>
          <div class="toggle-group">
            <button class="toggle-option ${this.currencyMode === 'usd-first' ? 'active' : ''}"
                    @click=${() => this.handleCurrencyToggle('usd-first')}>USD First</button>
            <button class="toggle-option ${this.currencyMode === 'sats-first' ? 'active' : ''}"
                    @click=${() => this.handleCurrencyToggle('sats-first')}>Sats First</button>
          </div>
        </div>
      </section>
    `;
  }

  private renderWalletSection() {
    const key = this.identityKey ?? this.agent?.identity_key;
    const balance = this.agent?.balance ?? 0;
    const rate = this.usdRate;
    const cm = this.currencyMode;

    return html`
      <section>
        <div class="section-header">
          <h2 class="section-title">Wallet</h2>
        </div>
        <div class="setting-row">
          <span class="setting-label">Connection</span>
          <span class="setting-value">
            <span class="conn-dot ${this.connected ? 'on' : ''}" style="margin-right: 6px; vertical-align: middle;"></span>
            ${this.connected ? 'Connected' : 'Disconnected'}
          </span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Wallet URL</span>
          <span class="setting-value mono">localhost:3322</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Balance</span>
          <span class="setting-value sats mono" title="${formatInlineCurrencyAlt(balance, rate, cm)}">
            ${formatInlineCurrency(balance, rate, cm)}
          </span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Identity Key</span>
          ${key ? html`
            <div class="identity-row">
              <span class="identity-key">${truncateKey(key)}</span>
              <button class="copy-btn ${this.copyFeedback ? 'copied' : ''}"
                      @click=${this.copyIdentityKey}
                      title="Copy full public key">
                ${this.copyFeedback ? 'Copied' : 'Copy'}
              </button>
            </div>
          ` : html`<span class="config-note">Not available</span>`}
        </div>
      </section>
    `;
  }

  private renderMemorySection() {
    return html`
      <section>
        <div class="section-header">
          <h2 class="section-title">Memory</h2>
        </div>
        <div class="setting-row">
          <span class="setting-label">Auto-recall</span>
          <span class="config-note">Configured in dolphin-milk.toml</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Encryption</span>
          <span class="config-note">Configured in dolphin-milk.toml</span>
        </div>
      </section>
    `;
  }

  private renderAdvancedSection() {
    return html`
      <section>
        <div class="section-header">
          <h2 class="section-title">Advanced</h2>
        </div>
        <div class="setting-row">
          <span class="setting-label">Log Level</span>
          <span class="config-note">Configured in dolphin-milk.toml</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Heartbeat</span>
          <span class="config-note">Configured in dolphin-milk.toml</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Config File</span>
          <span class="setting-value mono">dolphin-milk.toml</span>
        </div>
      </section>
    `;
  }

  render() {
    if (this.loading) {
      return renderLoading('Loading settings...');
    }

    if (this.error && !this.agent && !this.health) {
      return renderError(this.error, 'Check that the agent server is running.');
    }

    const version = this.health?.version ?? this.agent?.version;

    return html`
      <div class="page-title">Settings</div>
      <div class="settings-container">
        ${this.renderAgentSection()}
        ${this.renderBudgetSection()}
        ${this.renderDisplaySection()}
        ${this.renderWalletSection()}
        ${this.renderMemorySection()}
        ${this.renderAdvancedSection()}

        <div class="footer-note">
          Settings are configured in dolphin-milk.toml. Server-side editing coming soon.
          ${version ? html`<br><span class="version">v${version}</span>` : nothing}
        </div>
      </div>
    `;
  }
}
