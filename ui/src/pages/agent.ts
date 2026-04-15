import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import type { AgentInfo, CertificateStatus } from '../lib/shared-types.js';
import { pageHost, centerState, pageTitle, summaryGrid, statsBar, tabBar, stateFeedback, renderLoading, renderError, renderEmpty } from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';
import { formatUptime, truncateKey } from '../lib/util.js';
import { fetchBsvUsdRate, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import './memory.js';
import '../views/telemetry-view.js';

type AgentTab = 'identity' | 'tools' | 'memory' | 'telemetry';

/** Map tool names to categories. */
function categorize(toolName: string): string {
  if (['execute_bash', 'file_read', 'file_write', 'file_search', 'web_fetch'].includes(toolName)) return 'sandbox';
  if (['continue_task'].includes(toolName)) return 'system';
  if (toolName.startsWith('wallet_') || toolName === 'get_balance') return 'wallet';
  if (toolName.startsWith('memory_')) return 'memory';
  if (['send_message', 'check_inbox', 'list_messages', 'inbox_count'].includes(toolName)) return 'messagebox';
  if (['generate_image', 'x_search', 'x_profile', 'transcribe_audio', 'inscribe_onchain'].includes(toolName)) return 'x402';
  return 'other';
}

const CATEGORY_STYLES: Record<string, { label: string; color: string }> = {
  sandbox: { label: 'Sandbox', color: 'var(--accent, #14A8C4)' },
  system: { label: 'System', color: 'var(--text-dim, rgba(255,255,255,0.5))' },
  wallet: { label: 'Wallet', color: 'var(--warning, #fcbe2d)' },
  memory: { label: 'Memory', color: '#a78bfa' },
  messagebox: { label: 'MessageBox', color: '#38bdf8' },
  x402: { label: 'x402', color: 'var(--success, #00b69b)' },
  other: { label: 'Other', color: 'var(--text-dim, rgba(255,255,255,0.5))' },
};

const CATEGORY_ORDER = ['sandbox', 'system', 'wallet', 'memory', 'messagebox', 'x402', 'other'];

@customElement('dm-agent')
export class WormAgent extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @property() activeTab: AgentTab = 'identity';
  @state() private usdRate = 0;

  private ctrl = new FetchController<AgentInfo>(this);
  @state() private certStatus: CertificateStatus | null = null;

  static styles = [
    pageHost, centerState, pageTitle, summaryGrid, statsBar, tabBar, stateFeedback,
    css`
      .identity-card {
        background: var(--bg-elevated, #142D42);
        border: 1px solid var(--border, #1A3550);
        border-radius: 14px;
        padding: 20px;
        margin-bottom: 24px;
      }

      .identity-row {
        display: flex;
        align-items: center;
        gap: 12px;
        margin-bottom: 12px;
        flex-wrap: wrap;
      }

      .identity-label {
        font-size: 11px;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        color: var(--text-dim, rgba(255,255,255,0.5));
        min-width: 80px;
      }

      .identity-value {
        font-size: 13px;
        font-family: var(--mono, monospace);
        color: var(--text-bright, #fff);
        word-break: break-all;
      }

      .identity-key-row {
        display: flex;
        align-items: center;
        gap: 8px;
        flex: 1;
        min-width: 0;
      }

      .identity-key {
        font-size: 12px;
        font-family: var(--mono, monospace);
        color: var(--text-bright, #fff);
        word-break: break-all;
        flex: 1;
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
      }

      .copy-btn:hover {
        color: var(--accent, #14A8C4);
        border-color: var(--accent, #14A8C4);
      }

      .cert-badge {
        display: inline-flex;
        align-items: center;
        padding: 3px 10px;
        border-radius: 12px;
        font-size: 11px;
        font-weight: 600;
        font-family: var(--mono, monospace);
        letter-spacing: 0.3px;
      }

      .cert-badge.parent-signed {
        background: rgba(0, 182, 155, 0.12);
        color: var(--success, #00b69b);
      }

      .cert-badge.self-signed {
        background: rgba(252, 190, 45, 0.12);
        color: var(--warning, #fcbe2d);
      }

      .cert-badge.none {
        background: rgba(253, 84, 84, 0.12);
        color: var(--error, #fd5454);
      }

      .cert-badge.revoked {
        background: rgba(253, 84, 84, 0.12);
        color: var(--error, #fd5454);
      }

      .cert-txid {
        font-size: 11px;
        font-family: var(--mono, monospace);
        color: var(--text-dim, rgba(255,255,255,0.5));
      }
      .cert-txid a {
        color: var(--accent, #14A8C4);
        text-decoration: none;
      }
      .cert-txid a:hover {
        text-decoration: underline;
      }

      .section-heading {
        font-size: 16px;
        font-weight: 600;
        color: var(--text-bright, #fff);
        margin-bottom: 16px;
      }

      .tool-category {
        margin-bottom: 16px;
      }

      .category-label {
        font-size: 10px;
        text-transform: uppercase;
        letter-spacing: 0.5px;
        margin-bottom: 8px;
        display: flex;
        align-items: center;
        gap: 6px;
      }

      .category-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        flex-shrink: 0;
      }

      .tool-grid {
        display: flex;
        flex-wrap: wrap;
        gap: 6px;
      }

      .tool-pill {
        display: inline-flex;
        align-items: center;
        padding: 4px 10px;
        border-radius: 4px;
        font-size: 12px;
        font-family: var(--mono, monospace);
        background: var(--bg, #0B1929);
        border: 1px solid var(--border, #1A3550);
        color: var(--text, #e0e0e8);
      }

      .tab-content {
        flex: 1;
        min-height: 0;
        display: flex;
        flex-direction: column;
      }

      .tab-content > * {
        flex: 1;
      }
    `,
  ];

  firstUpdated() {
    fetchBsvUsdRate().then(r => { this.usdRate = r; });
    this.ctrl.fetch(async () => {
      const res = await this.fetchFn('/agent');
      if (!res.ok) throw new Error(`Failed to load agent info: ${res.status}`);
      return res.json();
    });
    // Fetch certificate status separately — /certificates has reliable is_revoked
    this.fetchFn('/certificates').then(r => r.ok ? r.json() : null)
      .then(d => { if (d) { this.certStatus = d; } })
      .catch(() => {});
  }

  private groupToolsByCategory(tools: string[]): Map<string, string[]> {
    const groups = new Map<string, string[]>();
    for (const tool of tools) {
      const cat = categorize(tool);
      if (!groups.has(cat)) groups.set(cat, []);
      groups.get(cat)!.push(tool);
    }
    return groups;
  }

  private copyIdentityKey(key: string) {
    navigator.clipboard.writeText(key).catch(() => {});
  }

  private renderCertBadge() {
    // Prefer certStatus from /certificates (has reliable is_revoked)
    const cs = this.certStatus;
    const agentStatus = this.ctrl.data?.certificate_status;
    const status = cs?.status ?? agentStatus ?? 'none';
    const isRevoked = cs?.is_revoked === true || this.ctrl.data?.certificate_revoked === true;

    const s = status.toLowerCase();
    if (s === 'parent-signed' && isRevoked) {
      return html`
        <span class="cert-badge revoked">\u2717 Revoked</span>
      `;
    }
    if (s === 'parent-signed') {
      return html`
        <span class="cert-badge parent-signed">\u2713 Parent-Signed</span>
      `;
    }
    if (s === 'self-signed') {
      return html`<span class="cert-badge self-signed">Self-Signed</span>`;
    }
    return html`<span class="cert-badge none">None</span>`;
  }

  private renderCertTxid() {
    const outpoint = (this.certStatus?.certificate as Record<string, unknown>)?.revocationOutpoint as string | undefined;
    if (!outpoint || outpoint.length < 64) return nothing;
    const txid = outpoint.slice(0, 64);
    if (txid === '0'.repeat(64)) return nothing; // null outpoint (self-signed)
    return html`
      <span class="cert-txid">
        <a href="https://whatsonchain.com/tx/${txid}" target="_blank" rel="noopener" title=${txid}>${txid.slice(0, 10)}\u2026${txid.slice(-6)}</a>
      </span>
    `;
  }

  private setTab(tab: AgentTab) {
    this.activeTab = tab;
    // Update URL hash to reflect tab (without triggering full navigation)
    const hash = tab === 'identity' ? 'agent' : `agent/${tab}`;
    if (window.location.hash !== `#${hash}`) {
      history.replaceState(null, '', `#${hash}`);
    }
  }

  private renderIdentity() {
    const a = this.ctrl.data!;
    return html`
      <div class="identity-card">
        <div class="identity-row">
          <span class="identity-label">Public Key</span>
          <div class="identity-key-row">
            <span class="identity-key" title="${a.identity_key}">${a.identity_key}</span>
            <button class="copy-btn" @click=${() => this.copyIdentityKey(a.identity_key)} title="Copy full public key">Copy</button>
          </div>
        </div>
        <div class="identity-row">
          <span class="identity-label">Version</span>
          <span class="identity-value">v${a.version}</span>
        </div>
        <div class="identity-row">
          <span class="identity-label">Certificate</span>
          ${this.renderCertBadge()}
          ${this.renderCertTxid()}
        </div>
        <div class="identity-row">
          <span class="identity-label">Uptime</span>
          <span class="identity-value">${formatUptime(a.uptime_secs)}</span>
        </div>
        <div class="identity-row">
          <span class="identity-label">Balance</span>
          <span class="identity-value" title="${formatInlineCurrencyAlt(a.balance, this.usdRate, this.currencyMode)}">${formatInlineCurrency(a.balance, this.usdRate, this.currencyMode)}</span>
        </div>
      </div>
    `;
  }

  private renderTools() {
    const a = this.ctrl.data!;
    const toolGroups = this.groupToolsByCategory(a.tools);

    if (a.tools.length === 0) {
      return renderEmpty('No tools registered', 'The agent has no tools loaded.');
    }

    return html`
      ${CATEGORY_ORDER
        .filter((cat) => toolGroups.has(cat))
        .map((cat) => {
          const style = CATEGORY_STYLES[cat] ?? CATEGORY_STYLES.other;
          const tools = toolGroups.get(cat)!;
          return html`
            <div class="tool-category">
              <div class="category-label" style="color: ${style.color}">
                <span class="category-dot" style="background: ${style.color}"></span>
                ${style.label}
              </div>
              <div class="tool-grid">
                ${tools.map((t) => html`<span class="tool-pill">${t}</span>`)}
              </div>
            </div>
          `;
        })}
    `;
  }

  private renderTabContent() {
    switch (this.activeTab) {
      case 'identity':
        return this.renderIdentity();
      case 'tools':
        return this.renderTools();
      case 'memory':
        return html`<dm-memory .fetchFn=${this.fetchFn}></dm-memory>`;
      case 'telemetry':
        return html`<dm-telemetry .fetchFn=${this.fetchFn}></dm-telemetry>`;
      default:
        return this.renderIdentity();
    }
  }

  render() {
    if (this.ctrl.loading) {
      return renderLoading('Loading agent info...');
    }

    if (this.ctrl.hasError) {
      return renderError(this.ctrl.error ?? 'Failed to load agent info', 'Check that the agent server is running.');
    }

    if (!this.ctrl.data) {
      return renderEmpty('No agent data available', 'The agent server may not be running.');
    }

    const a = this.ctrl.data;

    return html`
      <div class="page-title">Agent</div>

      <!-- Hero Section -->
      <div class="identity-card">
        <div class="identity-row">
          <span class="identity-label">Identity</span>
          <div class="identity-key-row">
            <span class="identity-key" title="${a.identity_key}">${truncateKey(a.identity_key)}</span>
            <button class="copy-btn" @click=${() => this.copyIdentityKey(a.identity_key)} title="Copy identity key">Copy</button>
          </div>
        </div>
        <div class="identity-row">
          <span class="identity-label">Version</span>
          <span class="identity-value">v${a.version}</span>
          <span style="margin-left: 12px; font-size: 12px; color: var(--text-dim)">Up ${formatUptime(a.uptime_secs)}</span>
        </div>
        <div class="identity-row">
          <span class="identity-label">Certificate</span>
          ${this.renderCertBadge()}
          ${this.renderCertTxid()}
        </div>
      </div>

      <!-- Stats Row -->
      <div class="summary-grid">
        <div class="summary-card">
          <div class="summary-value sats" title="${formatInlineCurrencyAlt(a.balance, this.usdRate, this.currencyMode)}">${formatInlineCurrency(a.balance, this.usdRate, this.currencyMode)}</div>
          <div class="summary-label">Balance</div>
        </div>
        <div class="summary-card">
          <div class="summary-value accent">${a.total_tasks}</div>
          <div class="summary-label">Tasks</div>
        </div>
        <div class="summary-card">
          <div class="summary-value sats" title="${formatInlineCurrencyAlt(a.total_sats_spent, this.usdRate, this.currencyMode)}">${formatInlineCurrency(a.total_sats_spent, this.usdRate, this.currencyMode)}</div>
          <div class="summary-label">Spent</div>
        </div>
        <div class="summary-card">
          <div class="summary-value">${a.tools.length}</div>
          <div class="summary-label">Tools</div>
        </div>
      </div>

      <!-- Tab Bar -->
      <div class="tab-bar">
        <button class="tab-btn ${this.activeTab === 'identity' ? 'active' : ''}"
                @click=${() => this.setTab('identity')}>Identity</button>
        <button class="tab-btn ${this.activeTab === 'tools' ? 'active' : ''}"
                @click=${() => this.setTab('tools')}>Tools</button>
        <button class="tab-btn ${this.activeTab === 'memory' ? 'active' : ''}"
                @click=${() => this.setTab('memory')}>Memory</button>
        <button class="tab-btn ${this.activeTab === 'telemetry' ? 'active' : ''}"
                @click=${() => this.setTab('telemetry')}>Telemetry</button>
      </div>

      <!-- Tab Content -->
      <div class="tab-content">
        ${this.renderTabContent()}
      </div>
    `;
  }
}
