/**
 * Shared Lit CSS modules — extracted from 10+ files to eliminate duplication.
 * Each component imports only what it needs:
 *   static styles = [pageHost, centerState, pageTitle, css`...`];
 *
 * Also exports helper functions for consistent state rendering:
 *   renderError(), renderEmpty(), renderLoading(), renderBreadcrumb()
 */

import { css, html, type TemplateResult } from 'lit';

/** Page host — standard flex column with padding and overflow. */
export const pageHost = css`
  :host {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 24px;
  }
  @media (max-width: 639px) {
    :host { padding: 16px 12px; }
  }
`;

/** Loading/error/empty centering. */
export const centerState = css`
  .center-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    font-size: 13px;
  }
  .center-state .title {
    font-size: 14px;
    color: var(--text, rgba(255,255,255,0.8));
  }
  .error-text {
    color: var(--error, #fd5454);
  }
`;

/** Page heading. */
export const pageTitle = css`
  .page-title {
    font-size: 24px;
    font-weight: 700;
    color: var(--text-bright, #fff);
    font-family: var(--sans, sans-serif);
    margin-bottom: 24px;
  }
`;

/** Stats summary grid — 6-card grid used in dashboard and agent. */
export const summaryGrid = css`
  .summary-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 8px;
    margin-bottom: 24px;
  }
  .summary-card {
    background: var(--bg-elevated, #142D42);
    border: 1px solid var(--border, #1A3550);
    border-radius: var(--radius-md, 14px);
    padding: 16px;
    text-align: left;
  }
  .summary-value {
    font-size: 28px;
    font-weight: 700;
    font-family: var(--sans, sans-serif);
    color: var(--text-bright, #fff);
    line-height: 1.2;
  }
  .summary-value.sats { color: var(--warning, #fcbe2d); }
  .summary-value.accent { color: var(--accent, #14A8C4); }
  .summary-value.success { color: var(--success, #00b69b); }
  .summary-label {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    margin-top: 6px;
  }
  @media (max-width: 639px) {
    .summary-grid { grid-template-columns: repeat(2, 1fr); }
    .summary-value { font-size: 16px; }
  }
  @media (min-width: 640px) and (max-width: 1023px) {
    .summary-grid { grid-template-columns: repeat(3, 1fr); }
  }
`;

/** Inline stats bar — compact horizontal stats row. */
export const statsBar = css`
  .stats-bar {
    display: flex;
    gap: 16px;
    margin-bottom: 20px;
    flex-wrap: wrap;
  }
  .stat {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .stat-value {
    font-size: 14px;
    font-weight: 600;
    font-family: var(--mono, monospace);
    color: var(--text-bright, #fff);
  }
  .stat-value.sats { color: var(--warning, #fcbe2d); }
  .stat-label {
    font-size: 11px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  @media (max-width: 639px) {
    .stats-bar { gap: 12px; }
  }
`;

/** Status badges / pills. */
export const badges = css`
  .badge {
    display: inline-flex;
    align-items: center;
    padding: 4px 12px;
    border-radius: 13.5px;
    font-size: 11px;
    font-weight: 600;
    font-family: var(--mono, monospace);
    letter-spacing: 0.3px;
  }
  .badge-running {
    color: var(--accent, #14A8C4);
    background: rgba(20, 168, 196, 0.15);
    animation: pulse-badge 2s ease-in-out infinite;
  }
  .badge-complete {
    color: var(--success, #00b69b);
    background: rgba(0, 182, 155, 0.15);
  }
  .badge-error {
    color: var(--error, #fd5454);
    background: rgba(253, 84, 84, 0.15);
  }
  .badge-pending {
    color: var(--text-dim, rgba(255,255,255,0.5));
    background: rgba(255, 255, 255, 0.08);
  }
  @keyframes pulse-badge {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.6; }
  }
`;

/** Card base — elevated card with border, meta items, and time. */
export const cardBase = css`
  .card {
    background: var(--bg-elevated, #142D42);
    border: 1px solid var(--border, #1A3550);
    border-radius: var(--radius-md, 14px);
    padding: 16px 20px;
    cursor: pointer;
    transition: background 0.15s ease, border-color 0.15s ease,
                transform 0.2s ease, box-shadow 0.2s ease;
    text-decoration: none;
    color: inherit;
  }
  .card:hover {
    background: rgba(255, 255, 255, 0.03);
    border-color: var(--accent-dim, #0E8FA8);
    transform: translateY(-2px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.15);
  }
  @media (prefers-reduced-motion: reduce) {
    .card { transition: none; }
    .card:hover { transform: none; }
  }
  .card:focus-visible {
    outline: 2px solid var(--accent, #14A8C4);
    outline-offset: 2px;
  }
  .meta-item {
    font-size: 11px;
    font-family: var(--mono, monospace);
    color: var(--text-dim, rgba(255,255,255,0.5));
    white-space: nowrap;
  }
  .meta-item.sats { color: var(--warning, #fcbe2d); }
  .time {
    font-size: 11px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    white-space: nowrap;
  }
`;

/** Tab bar — horizontal tab filter. */
export const tabBar = css`
  .tab-bar {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border, #1A3550);
    margin-bottom: 16px;
  }
  .tab-btn {
    padding: 8px 16px;
    border: none;
    background: none;
    color: var(--text-dim, rgba(255,255,255,0.5));
    cursor: pointer;
    font-size: 14px;
    font-weight: 600;
    font-family: var(--sans, sans-serif);
    border-bottom: 2px solid transparent;
    transition: color 0.15s ease;
  }
  .tab-btn:hover {
    color: var(--text, rgba(255,255,255,0.8));
  }
  .tab-btn.active {
    color: var(--accent, #14A8C4);
    border-bottom-color: var(--accent, #14A8C4);
  }
  .tab-btn:focus-visible {
    outline: 2px solid var(--accent, #14A8C4);
    outline-offset: -2px;
  }
  @media (max-width: 639px) {
    .tab-bar {
      overflow-x: auto;
      -webkit-overflow-scrolling: touch;
      white-space: nowrap;
      scrollbar-width: none;
    }
    .tab-bar::-webkit-scrollbar { display: none; }
  }
`;

/** Gauge bar — horizontal progress bar with fill, label, and value. */
export const gaugeBar = css`
  .gauge { margin-bottom: 16px; }
  .gauge-header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 4px;
  }
  .gauge-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .gauge-pct {
    font-family: var(--mono);
    font-size: 13px;
    font-weight: 700;
    color: var(--text);
  }
  .gauge-track {
    height: 10px;
    background: var(--bg-input, #1A2D42);
    border-radius: 5px;
    overflow: hidden;
  }
  .gauge-fill {
    height: 100%;
    border-radius: 4px;
    transition: width 0.6s ease, background 0.3s ease;
  }
  @media (prefers-reduced-motion: no-preference) {
    .gauge-fill {
      animation: gaugeGrow 0.5s ease-out;
    }
  }
  @keyframes gaugeGrow {
    from { width: 0 !important; }
  }
  .gauge-detail {
    display: flex;
    justify-content: space-between;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-dim);
    margin-top: 3px;
  }
  .gauge-detail .usd {
    color: var(--text-dim);
    opacity: 0.7;
  }
`;

/** Alert banner — warning/critical/exceeded states. */
export const alertBanner = css`
  .alert-banner {
    padding: 10px 16px;
    border-radius: var(--radius-sm, 8px);
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 16px;
    transition: all 0.3s ease;
  }
  .alert-banner.warning {
    background: rgba(252, 190, 45, 0.12);
    border: 1px solid rgba(252, 190, 45, 0.3);
    color: #fcbe2d;
  }
  .alert-banner.critical {
    background: rgba(253, 84, 84, 0.12);
    border: 1px solid rgba(253, 84, 84, 0.3);
    color: #fd5454;
  }
  .alert-banner.exceeded {
    background: rgba(253, 84, 84, 0.2);
    border: 1px solid rgba(253, 84, 84, 0.5);
    color: #fd5454;
  }
`;

/** Section title — used for sub-sections within a page. */
export const sectionTitle = css`
  .section-title {
    font-size: 16px;
    font-weight: 600;
    color: var(--text-bright, #fff);
    font-family: var(--sans, sans-serif);
    margin-bottom: 12px;
  }
`;

/** Buttons — base, primary, danger, small variants. */
export const buttonStyles = css`
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    border-radius: 8px;
    font-size: 13px;
    cursor: pointer;
    border: 1px solid var(--border, #1A3550);
    font-family: inherit;
    background: transparent;
    color: var(--text, #c9d1d9);
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--border-bright, #444d5a);
  }
  .btn:focus-visible {
    outline: 2px solid var(--accent, #14A8C4);
    outline-offset: 2px;
  }
  .btn-primary {
    background: rgba(20, 168, 196, 0.12);
    color: var(--accent, #14A8C4);
  }
  .btn-primary:hover:not(:disabled) {
    background: rgba(20, 168, 196, 0.2);
  }
  .btn-danger {
    background: rgba(255, 72, 72, 0.12);
    color: var(--error, #ff6b6b);
  }
  .btn-danger:hover:not(:disabled) {
    background: rgba(255, 72, 72, 0.2);
  }
  .btn-sm {
    padding: 5px 10px;
    font-size: 12px;
  }
  @media (max-width: 639px) {
    .btn { min-height: 44px; }
    .tab-btn { min-height: 44px; }
  }
`;

/** Search input — bar with text input, optional positioned icon. */
export const searchInput = css`
  .search-bar {
    display: flex;
    gap: 8px;
    margin-bottom: 16px;
  }
  .search-wrapper {
    position: relative;
    flex: 1;
    margin-bottom: 16px;
  }
  .search-icon {
    position: absolute;
    left: 12px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-dim, rgba(255,255,255,0.5));
    pointer-events: none;
    display: flex;
    align-items: center;
  }
  .search-wrapper:focus-within .search-icon {
    color: var(--accent, #14A8C4);
  }
  .search-wrapper .search-input {
    width: 100%;
    padding-left: 36px;
    box-sizing: border-box;
  }
  .search-input {
    flex: 1;
    padding: 8px 12px;
    background: var(--bg-input, #1A2D42);
    border: 1px solid var(--border, #1A3550);
    border-radius: 8px;
    color: var(--text, #c9d1d9);
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color 0.15s ease;
  }
  .search-input:focus {
    border-color: var(--accent, #14A8C4);
    box-shadow: 0 0 0 2px rgba(20, 168, 196, 0.25);
  }
  .search-input::placeholder {
    color: var(--text-dim, rgba(255,255,255,0.5));
  }
  @media (max-width: 639px) {
    .search-input { font-size: 16px; }
  }
`;

/** Status badges — consistent status pill styling. */
export const statusBadges = css`
  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 0.75rem;
    padding: 2px 8px;
    border-radius: 9999px;
    font-weight: 500;
    font-family: var(--mono, monospace);
  }
  .status-running {
    background: rgba(34, 197, 94, 0.15);
    color: #4ade80;
  }
  .status-completed {
    background: rgba(34, 197, 94, 0.15);
    color: #22c55e;
  }
  .status-failed {
    background: rgba(239, 68, 68, 0.15);
    color: #f87171;
  }
  .status-cancelled {
    background: rgba(156, 163, 175, 0.15);
    color: #9ca3af;
  }
  .status-paused {
    background: rgba(250, 204, 21, 0.15);
    color: #facc15;
  }
`;

/** Skeleton loading placeholders. */
export const skeleton = css`
  .skeleton {
    background: linear-gradient(
      90deg,
      var(--bg-elevated, #142D42) 25%,
      rgba(255,255,255,0.05) 50%,
      var(--bg-elevated, #142D42) 75%
    );
    background-size: 200% 100%;
    animation: shimmer 1.5s ease-in-out infinite;
    border-radius: var(--radius-sm, 8px);
  }
  @keyframes shimmer {
    0% { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }
  @media (prefers-reduced-motion: reduce) {
    .skeleton { animation: none; }
  }
  .skeleton-text { height: 14px; margin-bottom: 8px; }
  .skeleton-card { height: 80px; margin-bottom: 8px; }
  .skeleton-chart { height: 240px; }
  .skeleton-hero-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 16px;
    margin-bottom: 24px;
  }
  @media (max-width: 639px) {
    .skeleton-hero-grid { grid-template-columns: repeat(2, 1fr); }
  }
`;

/** State feedback — error, empty, and loading visual states. */
export const stateFeedback = css`
  .state-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 24px;
    text-align: center;
    flex: 1;
  }
  .state-error-icon {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: rgba(253, 84, 84, 0.12);
    color: var(--error, #fd5454);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    font-weight: 700;
  }
  .state-error-message {
    font-size: 13px;
    color: var(--error, #fd5454);
    max-width: 400px;
  }
  .state-error-hint {
    font-size: 12px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    max-width: 400px;
  }
  .state-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 48px 24px;
    text-align: center;
    flex: 1;
  }
  .state-empty-title {
    font-size: 15px;
    font-weight: 600;
    color: var(--text, rgba(255,255,255,0.8));
  }
  .state-empty-hint {
    font-size: 13px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    max-width: 400px;
    line-height: 1.5;
  }
  .state-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 40px 24px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    font-size: 13px;
    flex: 1;
  }
  .spinner {
    display: inline-block;
    width: 16px;
    height: 16px;
    border: 2px solid rgba(255, 255, 255, 0.15);
    border-top-color: var(--accent, #14A8C4);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Smart empty state — enhanced empty with icon, desc, action, prompts */
  .state-empty-smart {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 56px 24px;
    text-align: center;
    flex: 1;
  }
  .state-empty-smart-icon {
    font-size: 36px;
    line-height: 1;
    opacity: 0.7;
  }
  .state-empty-smart-title {
    font-size: 17px;
    font-weight: 600;
    color: var(--text, rgba(255,255,255,0.8));
  }
  .state-empty-smart-desc {
    font-size: 13px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    max-width: 420px;
    line-height: 1.6;
  }
  .state-empty-smart-prompts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  .state-empty-smart-prompt {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 16px;
    background: var(--bg-elevated, #142D42);
    border: 1px solid var(--border, #1A3550);
    border-radius: 8px;
    color: var(--text, rgba(255,255,255,0.8));
    font-size: 13px;
    text-decoration: none;
    transition: border-color 0.15s ease, background 0.15s ease;
    cursor: pointer;
  }
  .state-empty-smart-prompt:hover {
    border-color: var(--accent, #14A8C4);
    background: rgba(20, 168, 196, 0.06);
  }
  .state-empty-smart-action {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
    padding: 8px 20px;
    background: rgba(20, 168, 196, 0.12);
    border: 1px solid rgba(20, 168, 196, 0.3);
    border-radius: 8px;
    color: var(--accent, #14A8C4);
    font-size: 13px;
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
    transition: background 0.15s ease;
  }
  .state-empty-smart-action:hover {
    background: rgba(20, 168, 196, 0.2);
  }

  .breadcrumb {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-dim, rgba(255,255,255,0.5));
    margin-bottom: 12px;
  }
  .breadcrumb a {
    color: var(--text-dim, rgba(255,255,255,0.5));
    text-decoration: none;
    transition: color 0.15s ease;
  }
  .breadcrumb a:hover {
    color: var(--accent, #14A8C4);
  }
  .breadcrumb .sep {
    color: var(--border, #1A3550);
    font-size: 10px;
  }
  .breadcrumb .current {
    color: var(--text, rgba(255,255,255,0.8));
  }
`;

// ---------------------------------------------------------------------------
// Helper functions for consistent state rendering
// ---------------------------------------------------------------------------

/** Render a friendly error state with icon and optional hint. */
export function renderError(message: string, suggestion?: string): TemplateResult {
  return html`
    <div class="state-error">
      <div class="state-error-icon">!</div>
      <div class="state-error-message">${message}</div>
      ${suggestion ? html`<div class="state-error-hint">${suggestion}</div>` : ''}
    </div>
  `;
}

/** Optional action for enhanced empty states. */
export interface EmptyStateAction {
  label: string;
  href?: string;
  onClick?: () => void;
}

/** Optional suggested prompt for empty states. */
export interface EmptyStatePrompt {
  text: string;
  href?: string;
}

/** Render an empty state with title and optional subtitle. */
export function renderEmpty(title: string, subtitle?: string): TemplateResult {
  return html`
    <div class="state-empty">
      <div class="state-empty-title">${title}</div>
      ${subtitle ? html`<div class="state-empty-hint">${subtitle}</div>` : ''}
    </div>
  `;
}

/**
 * Render an enhanced empty state with icon, description, optional action button,
 * and optional suggested prompts. Designed for first-time empty pages.
 */
export function renderSmartEmpty(options: {
  icon: string;
  title: string;
  description: string;
  action?: EmptyStateAction;
  prompts?: EmptyStatePrompt[];
}): TemplateResult {
  return html`
    <div class="state-empty-smart">
      <div class="state-empty-smart-icon">${options.icon}</div>
      <div class="state-empty-smart-title">${options.title}</div>
      <div class="state-empty-smart-desc">${options.description}</div>
      ${options.prompts && options.prompts.length > 0 ? html`
        <div class="state-empty-smart-prompts">
          ${options.prompts.map(p => html`
            <a class="state-empty-smart-prompt" href=${p.href ?? '#'}>${p.text}</a>
          `)}
        </div>
      ` : ''}
      ${options.action ? html`
        <a class="state-empty-smart-action"
           href=${options.action.href ?? '#'}
           @click=${options.action.onClick ? (e: Event) => { e.preventDefault(); options.action!.onClick!(); } : undefined}
        >${options.action.label}</a>
      ` : ''}
    </div>
  `;
}

/** Render a loading spinner with text. */
export function renderLoading(text: string = 'Loading...'): TemplateResult {
  return html`
    <div class="state-loading"><span class="spinner"></span> ${text}</div>
  `;
}

/** Render a breadcrumb trail. Last item has no hash and is styled as current. */
export function renderBreadcrumb(items: Array<{ label: string; hash?: string }>): TemplateResult {
  return html`
    <nav class="breadcrumb">
      ${items.map((item, i) => {
        const isLast = i === items.length - 1;
        const sep = i < items.length - 1
          ? html`<span class="sep">&rsaquo;</span>`
          : '';
        if (isLast || !item.hash) {
          return html`<span class="current">${item.label}</span>${sep}`;
        }
        return html`<a href="${item.hash}">${item.label}</a>${sep}`;
      })}
    </nav>
  `;
}
