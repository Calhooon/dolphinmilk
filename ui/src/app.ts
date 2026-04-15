import { LitElement, html, css, nothing, type TemplateResult } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { authenticate, type AuthState } from './lib/wallet-auth.js';
import { truncateKey, formatSats } from './lib/util.js';
import { fetchBsvUsdRate, formatUsd, formatDualCurrency } from './lib/usd.js';
import { parseRoute, type Route } from './lib/router.js';
import { getModel, getCurrencyDisplay, setCurrencyDisplay, type CurrencyDisplay } from './lib/storage.js';
import {
  iconChat, iconDashboard, iconBudget, iconReports, iconCompliance,
  iconSearch, iconTasks, iconConversations, iconMemory, iconSchedules,
  iconCertificates, iconAgent, iconMenu, iconChevron, iconSettings, iconBell,
  iconWallet, iconArtifacts,
} from './lib/icons.js';
import './components/notification-center.js';
import './components/search-overlay.js';
import './pages/chat/chat.js';
import './pages/task-list.js';
import './pages/audit/audit.js';
import './pages/proofs/proofs.js';
import './pages/agent.js';
import './pages/conversations.js';
import './pages/conversation-detail.js';
import './pages/dashboard.js';
import './pages/automations.js';
import './pages/memory.js';
import './pages/certificates.js';
import './pages/budget/budget-panel.js';
import './pages/audit/audit-global.js';
import './pages/demo-overlay.js';
import './pages/reports.js';
import './pages/compliance.js';
import './pages/services.js';
import './pages/settings.js';
import './pages/wallet.js';
import './views/telemetry-view.js';

interface NavItem {
  label: string;
  icon: TemplateResult;
  hash: string;
  route: Route;
}

interface NavGroup {
  title: string;
  items: NavItem[];
}

@customElement('dm-app')
export class WormApp extends LitElement {
  @state() private auth: AuthState = {
    authenticated: false,
    identityKey: null,
    error: null,
    fetchFn: fetch.bind(window),
  };
  @state() private loading = true;
  @state() private balance = 0;
  @state() private usdRate = 0;
  @state() private spent = 0;
  @state() private model = getModel();
  @state() private currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @state() private connected = false;
  @state() private route: Route = 'chat';
  @state() private routeParams: Record<string, string> = {};
  @state() private sidebarOpen = false;
  @state() private searchOpen = false;
  @state() private notificationOpen = false;
  @state() private unreadCount = 0;
  @state() private collapsedSections = (() => {
    try {
      const stored = localStorage.getItem('dm-nav-collapsed');
      if (stored) return new Set<string>(JSON.parse(stored));
    } catch { /* ignore */ }
    return new Set<string>(['Governance']);
  })();

  private healthTimer: number | null = null;
  private logoClickCount = 0;
  private logoClickTimer: number | null = null;

  static styles = css`
    :host {
      display: flex;
      height: 100vh;
      background: var(--bg, #0B1929);
    }

    /* ---- Sidebar ---- */
    .sidebar {
      width: var(--sidebar-width, 220px);
      background: var(--sidebar-bg, #0F2337);
      border-right: 1px solid var(--sidebar-border, #1A3550);
      box-shadow: 2px 0 8px rgba(0, 0, 0, 0.15);
      display: flex;
      flex-direction: column;
      flex-shrink: 0;
      overflow-y: auto;
      overflow-x: hidden;
    }

    .sidebar-brand {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 16px;
      border-bottom: 1px solid var(--sidebar-border, #1A3550);
      flex-shrink: 0;
    }

    .sidebar-brand .logo {
      font-size: 16px;
      font-weight: 700;
      color: var(--text-bright, #fff);
      font-family: var(--sans, sans-serif);
      white-space: nowrap;
    }

    .logo-mark {
      width: 22px;
      height: 22px;
      border-radius: 4px;
      flex-shrink: 0;
      overflow: hidden;
    }
    .logo-mark img {
      width: 100%;
      height: 100%;
      object-fit: contain;
    }
    .brand-accent { color: var(--accent); }

    /* Nav section */
    .sidebar-nav {
      flex: 1;
      padding: 8px 0;
    }

    .nav-item {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 10px 16px 10px 20px;
      font-size: 13px;
      font-weight: 600;
      font-family: var(--sans, sans-serif);
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-decoration: none;
      cursor: pointer;
      transition: color 0.15s ease, background 0.15s ease, border-radius 0.15s ease, opacity 0.15s ease;
      white-space: nowrap;
      position: relative;
    }

    .nav-item:hover {
      color: var(--text, #e0e0e8);
      background: rgba(255, 255, 255, 0.02);
    }

    .nav-item:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: -2px;
      border-radius: 4px;
    }

    .nav-item.active {
      color: #fff;
      background: transparent;
      border-radius: 0;
      margin: 4px 10px;
    }

    .nav-item::before {
      content: '';
      position: absolute;
      left: 12px;
      top: 50%;
      width: 6px;
      height: 6px;
      border-radius: 50%;
      background: var(--border, #1A3550);
      opacity: 0.5;
      transform: translateY(-50%) scale(0.95);
      transition: all 0.15s ease;
    }

    .nav-item:hover::before {
      opacity: 0.75;
    }

    .nav-item.active::before {
      background: var(--accent, #14A8C4);
      width: 3px;
      height: 16px;
      border-radius: 2px;
      left: 10px;
      opacity: 1;
      transform: translateY(-50%);
    }

    .nav-item .icon {
      width: 16px;
      height: 16px;
      flex-shrink: 0;
      display: flex;
      align-items: center;
      justify-content: center;
    }
    .nav-item .icon svg {
      width: 16px;
      height: 16px;
    }

    /* Nav groups */
    .nav-group-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 6px 16px;
      margin-top: 8px;
      cursor: pointer;
      user-select: none;
      border-top: 1px solid var(--border, #1A3550);
      padding-top: 12px;
    }

    .nav-group-title {
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.6px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      opacity: 0.8;
    }

    .nav-group-chevron {
      width: 14px;
      height: 14px;
      display: flex;
      align-items: center;
      justify-content: center;
      color: var(--text-dim, rgba(255,255,255,0.5));
      transition: transform 0.15s ease;
    }
    .nav-group-chevron svg {
      width: 14px;
      height: 14px;
    }

    .nav-group-chevron.collapsed {
      transform: rotate(-90deg);
    }

    .nav-group-items {
      overflow: hidden;
    }

    .nav-group-items .nav-item {
      padding-left: 28px;
      font-size: 13px;
    }

    .nav-tier-separator {
      margin-top: 4px;
    }

    .notif-badge {
      background: var(--error, #fd5454);
      color: #fff;
      font-size: 9px;
      font-weight: 700;
      min-width: 16px;
      height: 16px;
      border-radius: 8px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      padding: 0 4px;
      margin-left: auto;
    }

    /* Sidebar footer */
    .sidebar-footer {
      padding: 12px 16px;
      border-top: 1px solid var(--sidebar-border, #1A3550);
      flex-shrink: 0;
      display: flex;
      flex-direction: column;
      gap: 10px;
    }

    .footer-status {
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .conn-dot {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      background: var(--error, #fd5454);
      flex-shrink: 0;
    }

    .conn-dot.on { background: var(--success, #00b69b); }

    .footer-spacer { flex: 1; }

    .footer-model {
      font-family: var(--mono, monospace);
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      background: rgba(255, 255, 255, 0.04);
      padding: 1px 6px;
      border-radius: 3px;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      max-width: 90px;
    }

    .footer-balance {
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .balance-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
    }

    .balance-primary {
      font-family: var(--mono, monospace);
      font-size: 14px;
      font-weight: 600;
      color: var(--warning, #fcbe2d);
      letter-spacing: -0.02em;
    }

    .balance-secondary {
      font-family: var(--mono, monospace);
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.4));
    }

    .currency-toggle {
      background: none;
      border: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-family: var(--mono, monospace);
      font-size: 10px;
      font-weight: 700;
      width: 22px;
      height: 22px;
      border-radius: 4px;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 0;
      flex-shrink: 0;
      transition: color 0.15s, border-color 0.15s, background 0.15s;
    }

    .currency-toggle:hover {
      color: var(--text, #e0e0e8);
      border-color: var(--accent, #14A8C4);
      background: rgba(20, 168, 196, 0.08);
    }

    .currency-toggle:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    .footer-identity {
      font-family: var(--mono, monospace);
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.35));
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    /* ---- Main content ---- */
    .main {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    .page-content {
      flex: 1;
      min-height: 0;
      display: flex;
      flex-direction: column;
      animation: pageFadeIn 0.15s ease;
    }
    @keyframes pageFadeIn {
      from { opacity: 0; transform: translateY(4px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @media (prefers-reduced-motion: reduce) {
      .page-content { animation: none; }
    }

    /* ---- Loading/auth states ---- */
    .loading, .auth-error {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .auth-error .msg {
      color: var(--error, #fd5454);
      max-width: 400px;
      text-align: center;
    }

    .retry-btn {
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text, #e0e0e8);
      padding: 8px 16px;
      border-radius: 6px;
      cursor: pointer;
      font-size: 13px;
    }

    .retry-btn:hover {
      background: var(--border, #1A3550);
    }

    .retry-btn:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    /* ---- Mobile top bar ---- */
    .mobile-bar {
      display: none;
      align-items: center;
      gap: 12px;
      padding: 0 12px;
      height: 48px;
      background: var(--sidebar-bg, #0F2337);
      border-bottom: 1px solid var(--sidebar-border, #1A3550);
      flex-shrink: 0;
    }

    .hamburger {
      background: none;
      border: none;
      color: var(--text, #e0e0e8);
      cursor: pointer;
      padding: 4px;
      display: flex;
      align-items: center;
      justify-content: center;
      width: 28px;
      height: 28px;
    }

    .hamburger:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
      border-radius: 4px;
    }
    .hamburger svg {
      width: 20px;
      height: 20px;
    }

    .mobile-logo {
      font-size: 14px;
      font-weight: 700;
      color: var(--text-bright, #fff);
      font-family: var(--sans, sans-serif);
    }

    /* Overlay for mobile drawer */
    .overlay {
      display: none;
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      z-index: 99;
    }

    .overlay.visible {
      display: block;
    }

    /* ---- Responsive ---- */

    /* Tablet: icons only, hover expands */
    @media (min-width: 640px) and (max-width: 1023px) {
      .sidebar {
        width: var(--sidebar-collapsed, 64px);
        transition: width 0.2s ease;
      }

      .sidebar:hover {
        width: var(--sidebar-width, 220px);
      }

      .sidebar:not(:hover) .nav-item .label,
      .sidebar:not(:hover) .nav-group-header,
      .sidebar:not(:hover) .nav-group-items .nav-item .label,
      .sidebar:not(:hover) .sidebar-brand .logo,
      .sidebar:not(:hover) .footer-balance,
      .sidebar:not(:hover) .footer-identity,
      .sidebar:not(:hover) .footer-model,
      .sidebar:not(:hover) .footer-spacer,
      .sidebar:not(:hover) .status-label {
        display: none;
      }

      .sidebar:not(:hover) .nav-item {
        justify-content: center;
        padding: 10px 0;
      }

      .sidebar:not(:hover) .nav-item.active {
        margin: 2px 10px;
      }

      .sidebar:not(:hover) .nav-group-items .nav-item {
        padding-left: 0;
      }

      .sidebar:not(:hover) .sidebar-brand {
        justify-content: center;
      }

      .sidebar:not(:hover) .footer-status {
        justify-content: center;
      }
    }

    /* Safe area insets for modern iPhones */
    @supports(padding: env(safe-area-inset-bottom)) {
      .sidebar-footer {
        padding-bottom: calc(12px + env(safe-area-inset-bottom));
      }
      .mobile-bar {
        padding-top: env(safe-area-inset-top);
      }
    }

    /* Mobile: hidden sidebar, hamburger */
    @media (max-width: 639px) {
      .sidebar {
        position: fixed;
        top: 0;
        left: -260px;
        width: 260px;
        height: 100%;
        z-index: 100;
        transition: left 0.25s ease;
      }

      .sidebar.open {
        left: 0;
      }

      .mobile-bar {
        display: flex;
      }

      .main {
        width: 100%;
      }
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    window.addEventListener('hashchange', () => this.updateRoute());
    window.addEventListener('keydown', this._handleKeyDown);
    this.updateRoute();
    this.init();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    window.removeEventListener('keydown', this._handleKeyDown);
    if (this.healthTimer) {
      clearInterval(this.healthTimer);
    }
  }

  private _handleKeyDown = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      this.searchOpen = !this.searchOpen;
    }
  };

  private updateRoute() {
    const hash = window.location.hash.slice(1) || '';
    const { route, params } = parseRoute(hash);
    this.route = route;
    this.routeParams = params;
    // Close mobile drawer on navigation
    this.sidebarOpen = false;
  }

  private async init() {
    this.loading = true;
    this.auth = await authenticate();
    this.loading = false;
    if (this.auth.authenticated) {
      this.connected = true;
      this.startHealthPolling();
      this.fetchInitialBalance();
      fetchBsvUsdRate().then(r => { this.usdRate = r; });
    }
  }

  private async fetchInitialBalance() {
    try {
      const res = await this.auth.fetchFn('/agent');
      if (res.ok) {
        const data = await res.json();
        this.balance = data.balance ?? 0;
      }
    } catch {
      // Best effort — balance will update on next budget event
    }
  }

  private startHealthPolling() {
    this.fetchHealth();
    this.healthTimer = window.setInterval(() => this.fetchHealth(), 30000);
  }

  private async fetchHealth() {
    try {
      const res = await fetch('/health');
      this.connected = res.ok;
    } catch {
      this.connected = false;
    }
  }

  private handleBudgetUpdate(e: CustomEvent) {
    this.balance = e.detail.balance;
    this.spent = e.detail.spent;
  }

  private handleDone(e: CustomEvent) {
    this.spent = e.detail.sats_spent;
  }

  private handleModelChange(e: CustomEvent) {
    this.model = e.detail.model;
  }

  private toggleCurrency() {
    this.currencyMode = this.currencyMode === 'usd-first' ? 'sats-first' : 'usd-first';
    setCurrencyDisplay(this.currencyMode);
  }

  private toggleSection(name: string) {
    const next = new Set(this.collapsedSections);
    if (next.has(name)) {
      next.delete(name);
    } else {
      next.add(name);
    }
    this.collapsedSections = next;
    try {
      localStorage.setItem('dm-nav-collapsed', JSON.stringify([...next]));
    } catch { /* ignore */ }
  }

  private isActive(route: Route, hash?: string): boolean {
    if (this.route === route) return true;
    // conversation-detail highlights "Conversations" nav item
    if (route === 'conversations' && this.route === 'conversation-detail') return true;
    return false;
  }

  private isGroupActive(routes: Route[]): boolean {
    return routes.some((r) => this.route === r);
  }

  private renderNavTop() {
    const topItems: NavItem[] = [
      { label: 'Chat', icon: iconChat, hash: '', route: 'chat' },
      { label: 'Conversations', icon: iconConversations, hash: 'conversations', route: 'conversations' },
      { label: 'Wallet', icon: iconWallet, hash: 'wallet', route: 'wallet' as Route },
      { label: 'Agent', icon: iconAgent, hash: 'agent', route: 'agent' },
    ];

    return html`
      ${topItems.map((item) => html`
        <a class="nav-item ${this.isActive(item.route) ? 'active' : ''}" href="#${item.hash}" aria-label=${item.label}>
          <span class="icon">${item.icon}</span>
          <span class="label">${item.label}</span>
        </a>
      `)}
    `;
  }

  private renderNavGroups() {
    const operationsGroup: NavGroup = {
      title: 'Operations',
      items: [
        { label: 'Activity', icon: iconTasks, hash: 'activity', route: 'tasks' },
        { label: 'Artifacts', icon: iconArtifacts, hash: 'artifacts', route: 'artifacts' as Route },
        { label: 'Automations', icon: iconSchedules, hash: 'automations', route: 'automations' as Route },
        { label: 'Services', icon: iconSearch, hash: 'services', route: 'services' as Route },
        { label: 'Budget', icon: iconBudget, hash: 'budget', route: 'budget' as Route },
        { label: 'Reports', icon: iconReports, hash: 'reports', route: 'reports' as Route },
      ],
    };

    const governanceGroup: NavGroup = {
      title: 'Governance',
      items: [
        { label: 'Audit Trail', icon: iconSearch, hash: 'audit-trail', route: 'audit-global' as Route },
        { label: 'Compliance', icon: iconCompliance, hash: 'compliance', route: 'compliance' as Route },
        { label: 'Certificates', icon: iconCertificates, hash: 'certificates', route: 'certificates' },
      ],
    };

    return html`
      ${this.renderNavGroup(operationsGroup, operationsGroup.items.map(i => i.route))}
      ${this.renderNavGroup(governanceGroup, governanceGroup.items.map(i => i.route))}
    `;
  }

  private renderSidebarFooter() {
    const shortKey = this.auth.identityKey
      ? truncateKey(this.auth.identityKey)
      : 'dev mode';

    const dual = this.usdRate
      ? formatDualCurrency(this.balance, this.usdRate, this.currencyMode)
      : null;

    return html`
      <div class="sidebar-footer">
        <div class="footer-status">
          <span class="conn-dot ${this.connected ? 'on' : ''}"></span>
          <span class="status-label">${this.connected ? 'Connected' : 'Disconnected'}</span>
          <span class="footer-spacer"></span>
          <span class="footer-model">${this.model}</span>
        </div>
        <a href="#wallet" class="footer-balance" style="text-decoration:none;color:inherit">
          <div class="balance-row">
            <span class="balance-primary">${dual ? dual.primary : `${formatSats(this.balance)} sats`}</span>
            <button class="currency-toggle" @click=${(e: Event) => { e.preventDefault(); e.stopPropagation(); this.toggleCurrency(); }}
                    title="Switch between USD and sats display"
                    aria-label="Toggle currency display between USD and sats">
              ${this.currencyMode === 'usd-first' ? '$' : 'S'}
            </button>
          </div>
          ${dual ? html`<div class="balance-secondary">${dual.secondary}</div>` : ''}
        </a>
        <div class="footer-identity" title=${this.auth.identityKey ?? ''}>${shortKey}</div>
      </div>
    `;
  }

  private handleLogoClick() {
    this.logoClickCount++;
    console.log('logo click', this.logoClickCount);
    if (this.logoClickTimer !== null) clearTimeout(this.logoClickTimer);
    this.logoClickTimer = window.setTimeout(() => { this.logoClickCount = 0; }, 1500);
    if (this.logoClickCount >= 5) {
      this.logoClickCount = 0;
      if (this.route === 'demo') {
        window.location.hash = '#dashboard';
      } else {
        window.location.hash = '#demo/stats';
      }
    }
  }

  private renderSidebar() {
    return html`
      <div class="sidebar ${this.sidebarOpen ? 'open' : ''}">
        <button class="sidebar-brand" @click=${() => this.handleLogoClick()}
          style="all:unset;display:flex;align-items:center;gap:8px;padding:16px;border-bottom:1px solid var(--sidebar-border,#1A3550);flex-shrink:0;cursor:pointer;width:100%;box-sizing:border-box">
          <span class="logo-mark"><img src="${import.meta.env.BASE_URL}logo.png" alt="Dolphin Milk"></span>
          <span class="logo"><span class="brand-accent">Dolphin</span> Milk</span>
        </button>

        <div class="sidebar-nav">
          ${this.renderNavTop()}
          <div class="nav-tier-separator">
            ${this.renderNavGroups()}
          </div>
          <a class="nav-item ${this.isActive('settings' as Route) ? 'active' : ''}"
             href="#settings" aria-label="Settings" style="margin-top: 8px;">
            <span class="icon">${iconSettings}</span>
            <span class="label">Settings</span>
          </a>
          <a class="nav-item" href="javascript:void(0)" aria-label="Notifications"
             @click=${(e: Event) => { e.preventDefault(); this.notificationOpen = !this.notificationOpen; }}>
            <span class="icon">${iconBell}</span>
            <span class="label">Notifications</span>
            ${this.unreadCount > 0 ? html`<span class="notif-badge">${this.unreadCount}</span>` : nothing}
          </a>
        </div>

        ${this.renderSidebarFooter()}
      </div>
    `;
  }

  private renderNavGroup(group: NavGroup, routes: Route[]) {
    const collapsed = this.collapsedSections.has(group.title);
    const groupActive = this.isGroupActive(routes);

    return html`
      <div class="nav-group-header" @click=${() => this.toggleSection(group.title)}>
        <span class="nav-group-title" style=${groupActive ? 'color: var(--accent)' : ''}>${group.title}</span>
        <span class="nav-group-chevron ${collapsed ? 'collapsed' : ''}">${iconChevron}</span>
      </div>
      ${collapsed ? nothing : html`
        <div class="nav-group-items">
          ${group.items.map((item) => html`
            <a class="nav-item ${this.isActive(item.route, item.hash) ? 'active' : ''}" href="#${item.hash}" aria-label=${item.label}>
              <span class="icon">${item.icon}</span>
              <span class="label">${item.label}</span>
            </a>
          `)}
        </div>
      `}
    `;
  }

  private renderPage() {
    const cm = this.currencyMode;
    switch (this.route) {
      case 'conversations':
        return html`<dm-conversations .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-conversations>`;
      case 'conversation-detail':
        return html`<dm-conversation-detail
          .fetchFn=${this.auth.fetchFn}
          .currencyMode=${cm}
          .conversationId=${this.routeParams.id ?? ''}
          .initialTab=${this.routeParams.tab ?? ''}
        ></dm-conversation-detail>`;
      case 'tasks':
        return html`<dm-task-list .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-task-list>`;
      case 'audit':
        return html`<dm-audit .fetchFn=${this.auth.fetchFn} .taskId=${this.routeParams.id ?? ''} .currencyMode=${cm}></dm-audit>`;
      case 'proofs':
        return html`<dm-proofs .fetchFn=${this.auth.fetchFn} .taskId=${this.routeParams.id ?? ''} .currencyMode=${cm}></dm-proofs>`;
      case 'replay':
        return html`<dm-replay-view .fetchFn=${this.auth.fetchFn} .taskId=${this.routeParams.id ?? ''} .currencyMode=${cm}></dm-replay-view>`;
      case 'agent':
        return html`<dm-agent .fetchFn=${this.auth.fetchFn} .currencyMode=${cm} .activeTab=${this.routeParams.tab || 'identity'}></dm-agent>`;
      case 'automations':
        return html`<dm-automations .fetchFn=${this.auth.fetchFn}></dm-automations>`;
      case 'dashboard':
        return html`<dm-dashboard .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-dashboard>`;
      case 'budget':
        return html`<dm-budget-panel .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-budget-panel>`;
      case 'reports':
        return html`<dm-reports .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-reports>`;
      case 'compliance':
        return html`<dm-compliance .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-compliance>`;
      case 'audit-global':
        return html`<dm-audit-global .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-audit-global>`;
      case 'certificates':
        return html`<dm-certificates .fetchFn=${this.auth.fetchFn} .currencyMode=${cm} .usdRate=${this.usdRate}></dm-certificates>`;
      case 'services':
        return html`<dm-services .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-services>`;
      case 'settings':
        return html`<dm-settings .fetchFn=${this.auth.fetchFn} .currencyMode=${cm} .identityKey=${this.auth.identityKey}></dm-settings>`;
      case 'wallet':
        return html`<dm-wallet .fetchFn=${this.auth.fetchFn} .currencyMode=${cm}></dm-wallet>`;
      case 'artifacts':
        return html`<dm-artifacts .fetchFn=${this.auth.fetchFn}></dm-artifacts>`;
      case 'demo':
        return html`<dm-demo-overlay pillar=${this.routeParams.pillar ?? 'stats'}></dm-demo-overlay>`;
      default:
        return html`
          <dm-chat
            .fetchFn=${this.auth.fetchFn}
            .sessionId=${this.routeParams.sessionId ?? null}
            .currencyMode=${cm}
            @budget=${this.handleBudgetUpdate}
            @done=${this.handleDone}
            @model-change=${this.handleModelChange}
          ></dm-chat>
        `;
    }
  }

  render() {
    if (this.loading) {
      return html`
        <div class="main">
          <div class="loading">Connecting...</div>
        </div>
      `;
    }

    if (!this.auth.authenticated) {
      return html`
        <div class="main">
          <div class="auth-error">
            <div class="msg">${this.auth.error ?? 'Authentication required'}</div>
            <button class="retry-btn" @click=${this.init}>Retry</button>
          </div>
        </div>
      `;
    }

    const cm = this.currencyMode;

    return html`
      ${this.renderSidebar()}
      <div class="overlay ${this.sidebarOpen ? 'visible' : ''}"
        @click=${() => { this.sidebarOpen = false; }}
      ></div>
      <div class="main">
        <div class="mobile-bar">
          <button class="hamburger" aria-label="Toggle navigation menu" @click=${() => { this.sidebarOpen = !this.sidebarOpen; }}>${iconMenu}</button>
          <span class="mobile-logo">Dolphin Milk</span>
        </div>
        <div class="page-content">
          ${this.renderPage()}
        </div>
      </div>
      <dm-search-overlay
        .open=${this.searchOpen}
        .fetchFn=${this.auth.fetchFn}
        @close=${() => { this.searchOpen = false; }}
        @navigate=${(e: CustomEvent) => { this.searchOpen = false; window.location.hash = e.detail.hash; }}
      ></dm-search-overlay>
      <dm-notification-center
        .open=${this.notificationOpen}
        .fetchFn=${this.auth.fetchFn}
        .currencyMode=${cm}
        @close=${() => { this.notificationOpen = false; }}
        @badge-update=${(e: CustomEvent) => { this.unreadCount = e.detail.count; }}
      ></dm-notification-center>
    `;
  }
}
