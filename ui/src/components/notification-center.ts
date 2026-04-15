/**
 * Notification center with bell icon dropdown panel.
 * Derives notifications from task and budget state changes,
 * persists history in localStorage (max 50 items).
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatRelativeTime } from '../lib/util.js';
import { formatInlineCurrency } from '../lib/usd.js';
import { fetchBsvUsdRate } from '../lib/usd.js';
import type { CurrencyDisplay } from '../lib/storage.js';
import type { TaskSummary } from '../lib/shared-types.js';

// --- Types ---

interface Notification {
  id: string;
  type: 'task-complete' | 'task-failed' | 'budget-warning' | 'automation-complete' | 'automation-failed';
  title: string;
  body: string;
  timestamp: number;
  read: boolean;
  link?: string;
}

interface NotificationState {
  lastTaskIds: string[];
  lastTaskStatuses: Record<string, string>;
  budgetThresholdsCrossed: number[];
}

// --- Constants ---

const STORAGE_KEY = 'dm-notifications';
const STATE_KEY = 'dm-notifications-state';
const MAX_NOTIFICATIONS = 50;
const BUDGET_THRESHOLDS = [60, 80, 95];

// --- Helpers ---

function loadNotifications(): Notification[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Notification[];
  } catch { /* ignore */ }
  return [];
}

function saveNotifications(items: Notification[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(items.slice(0, MAX_NOTIFICATIONS)));
  } catch { /* ignore */ }
}

function loadState(): NotificationState {
  try {
    const raw = localStorage.getItem(STATE_KEY);
    if (raw) return JSON.parse(raw) as NotificationState;
  } catch { /* ignore */ }
  return { lastTaskIds: [], lastTaskStatuses: {}, budgetThresholdsCrossed: [] };
}

function saveState(s: NotificationState): void {
  try {
    localStorage.setItem(STATE_KEY, JSON.stringify(s));
  } catch { /* ignore */ }
}

function generateId(): string {
  return `n-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

@customElement('dm-notification-center')
export class WormNotificationCenter extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ type: Boolean }) open = false;
  @property({ attribute: false }) currencyMode: CurrencyDisplay = 'usd-first';

  @state() private notifications: Notification[] = [];
  @state() private usdRate = 0;

  private _boundClickOutside: ((e: MouseEvent) => void) | null = null;

  static styles = css`
    :host {
      display: block;
      position: fixed;
      z-index: 150;
    }

    .panel {
      position: fixed;
      bottom: 60px;
      left: 16px;
      width: 360px;
      max-height: 480px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-sm, 8px);
      box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
      display: flex;
      flex-direction: column;
      overflow: hidden;
      animation: panel-in 0.15s ease;
    }

    @keyframes panel-in {
      from { opacity: 0; transform: translateY(8px); }
      to { opacity: 1; transform: translateY(0); }
    }

    @media (prefers-reduced-motion: reduce) {
      .panel { animation: none; }
    }

    .panel-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 12px 16px;
      border-bottom: 1px solid var(--border, #1A3550);
      flex-shrink: 0;
    }

    .panel-title {
      font-size: 14px;
      font-weight: 700;
      color: var(--text-bright, #fff);
      font-family: var(--sans, sans-serif);
    }

    .panel-actions {
      display: flex;
      gap: 8px;
    }

    .panel-action {
      background: none;
      border: none;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 11px;
      font-family: var(--sans, sans-serif);
      cursor: pointer;
      padding: 2px 6px;
      border-radius: 4px;
      transition: color 0.15s, background 0.15s;
    }

    .panel-action:hover {
      color: var(--accent, #14A8C4);
      background: rgba(20, 168, 196, 0.08);
    }

    .panel-action:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 1px;
    }

    .panel-body {
      flex: 1;
      overflow-y: auto;
      min-height: 0;
    }

    .day-divider {
      font-size: 10px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      padding: 10px 16px 4px;
    }

    .notif-item {
      display: flex;
      gap: 10px;
      padding: 10px 16px;
      cursor: pointer;
      transition: background 0.1s;
      border-bottom: 1px solid rgba(255,255,255,0.03);
    }

    .notif-item:hover {
      background: rgba(255, 255, 255, 0.03);
    }

    .notif-item.unread {
      background: rgba(72, 128, 255, 0.04);
    }

    .notif-icon {
      width: 28px;
      height: 28px;
      border-radius: 6px;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 14px;
      flex-shrink: 0;
    }

    .notif-icon.complete { background: rgba(0, 182, 155, 0.12); }
    .notif-icon.failed { background: rgba(253, 84, 84, 0.12); }
    .notif-icon.warning { background: rgba(252, 190, 45, 0.12); }

    .notif-body {
      flex: 1;
      min-width: 0;
    }

    .notif-title {
      font-size: 13px;
      font-weight: 600;
      color: var(--text, #e0e0e8);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .notif-desc {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-top: 2px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .notif-link {
      font-size: 11px;
      color: var(--accent, #14A8C4);
      text-decoration: none;
      margin-top: 3px;
      display: inline-block;
    }

    .notif-link:hover {
      text-decoration: underline;
    }

    .notif-time {
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      flex-shrink: 0;
      white-space: nowrap;
    }

    .empty {
      padding: 40px 16px;
      text-align: center;
      color: var(--text-dim, rgba(255,255,255,0.4));
      font-size: 13px;
    }

    .unread-dot {
      width: 6px;
      height: 6px;
      border-radius: 50%;
      background: var(--accent, #14A8C4);
      flex-shrink: 0;
      margin-top: 6px;
    }

    @media (max-width: 639px) {
      .panel {
        left: 8px;
        right: 8px;
        width: auto;
        bottom: 56px;
      }
    }
  `;

  connectedCallback() {
    super.connectedCallback();
    this.notifications = loadNotifications();
    this._emitBadgeUpdate();
    fetchBsvUsdRate().then(r => { this.usdRate = r; });
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this._removeClickOutside();
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('open')) {
      if (this.open) {
        this._checkForNewNotifications();
        this._markAllRead();
        // Defer adding click-outside listener to avoid catching the opening click
        requestAnimationFrame(() => {
          this._addClickOutside();
        });
      } else {
        this._removeClickOutside();
      }
    }
  }

  private _addClickOutside() {
    this._removeClickOutside();
    this._boundClickOutside = (e: MouseEvent) => {
      const path = e.composedPath();
      if (!path.includes(this)) {
        this.dispatchEvent(new CustomEvent('close'));
      }
    };
    document.addEventListener('click', this._boundClickOutside, true);
  }

  private _removeClickOutside() {
    if (this._boundClickOutside) {
      document.removeEventListener('click', this._boundClickOutside, true);
      this._boundClickOutside = null;
    }
  }

  private async _checkForNewNotifications() {
    const prevState = loadState();
    const newNotifs: Notification[] = [];

    try {
      // Fetch tasks
      const tasksRes = await this.fetchFn('/tasks');
      if (tasksRes.ok) {
        const tasks: TaskSummary[] = await tasksRes.json();
        for (const task of tasks) {
          const prevStatus = prevState.lastTaskStatuses[task.id];
          const isNew = prevStatus && prevStatus !== task.status;

          if (isNew && (task.status === 'complete' || task.status === 'completed' || task.status === 'done')) {
            const costStr = this.usdRate > 0
              ? formatInlineCurrency(task.sats_spent, this.usdRate, this.currencyMode)
              : `${task.sats_spent.toLocaleString()} sats`;
            newNotifs.push({
              id: generateId(),
              type: 'task-complete',
              title: `Task "${this._truncateText(task.task, 40)}" completed`,
              body: `${task.iterations} iters, ${costStr}`,
              timestamp: Date.now(),
              read: false,
              link: `#task/${task.id}`,
            });
          } else if (isNew && (task.status === 'error' || task.status === 'failed')) {
            newNotifs.push({
              id: generateId(),
              type: 'task-failed',
              title: `Task "${this._truncateText(task.task, 40)}" failed`,
              body: task.error ?? 'Unknown error',
              timestamp: Date.now(),
              read: false,
              link: `#task/${task.id}`,
            });
          }

          prevState.lastTaskStatuses[task.id] = task.status;
        }
        prevState.lastTaskIds = tasks.map(t => t.id);
      }
    } catch {
      // Best effort
    }

    try {
      // Fetch budget
      const budgetRes = await this.fetchFn('/budget');
      if (budgetRes.ok) {
        const budget = await budgetRes.json();
        const limits = budget.limits;
        if (limits) {
          // Check daily budget utilization
          const dailySpent = budget.daily_sats ?? 0;
          const dailyLimit = limits.max_per_day ?? 0;
          if (dailyLimit > 0) {
            const pct = Math.round((dailySpent / dailyLimit) * 100);
            for (const threshold of BUDGET_THRESHOLDS) {
              if (pct >= threshold && !prevState.budgetThresholdsCrossed.includes(threshold)) {
                newNotifs.push({
                  id: generateId(),
                  type: 'budget-warning',
                  title: `Budget at ${pct}%`,
                  body: `Daily spending has reached ${pct}% of the limit`,
                  timestamp: Date.now(),
                  read: false,
                  link: '#budget',
                });
                prevState.budgetThresholdsCrossed.push(threshold);
              }
            }
            // Reset thresholds if spending dropped below
            if (pct < 60) {
              prevState.budgetThresholdsCrossed = [];
            }
          }
        }
      }
    } catch {
      // Best effort
    }

    if (newNotifs.length > 0) {
      this.notifications = [...newNotifs, ...this.notifications].slice(0, MAX_NOTIFICATIONS);
      saveNotifications(this.notifications);
      this._emitBadgeUpdate();
    }

    saveState(prevState);
  }

  private _markAllRead() {
    let changed = false;
    for (const n of this.notifications) {
      if (!n.read) {
        n.read = true;
        changed = true;
      }
    }
    if (changed) {
      this.notifications = [...this.notifications];
      saveNotifications(this.notifications);
      this._emitBadgeUpdate();
    }
  }

  private _clearAll() {
    this.notifications = [];
    saveNotifications([]);
    this._emitBadgeUpdate();
  }

  private _emitBadgeUpdate() {
    const count = this.notifications.filter(n => !n.read).length;
    this.dispatchEvent(new CustomEvent('badge-update', {
      detail: { count },
      bubbles: true,
      composed: true,
    }));
  }

  private _handleNotifClick(n: Notification) {
    if (n.link) {
      window.location.hash = n.link;
      this.dispatchEvent(new CustomEvent('close'));
    }
  }

  private _truncateText(text: string, max: number): string {
    if (text.length <= max) return text;
    return text.slice(0, max) + '...';
  }

  private _getIcon(type: Notification['type']): string {
    switch (type) {
      case 'task-complete':
      case 'automation-complete':
        return '\u2705';
      case 'task-failed':
      case 'automation-failed':
        return '\u274C';
      case 'budget-warning':
        return '\u26A0\uFE0F';
    }
  }

  private _getIconClass(type: Notification['type']): string {
    switch (type) {
      case 'task-complete':
      case 'automation-complete':
        return 'complete';
      case 'task-failed':
      case 'automation-failed':
        return 'failed';
      case 'budget-warning':
        return 'warning';
    }
  }

  private _groupByDay(items: Notification[]): Map<string, Notification[]> {
    const groups = new Map<string, Notification[]>();
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    const yesterday = today - 86400000;

    for (const item of items) {
      const d = new Date(item.timestamp);
      const dayStart = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
      let label: string;
      if (dayStart >= today) {
        label = 'Today';
      } else if (dayStart >= yesterday) {
        label = 'Yesterday';
      } else {
        label = d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
      }
      const existing = groups.get(label);
      if (existing) {
        existing.push(item);
      } else {
        groups.set(label, [item]);
      }
    }
    return groups;
  }

  render() {
    if (!this.open) return nothing;

    const groups = this._groupByDay(this.notifications);

    return html`
      <div class="panel" role="dialog" aria-label="Notifications">
        <div class="panel-header">
          <span class="panel-title">Notifications</span>
          <div class="panel-actions">
            <button class="panel-action" @click=${this._markAllRead}>Mark all read</button>
            <button class="panel-action" @click=${this._clearAll}>Clear all</button>
          </div>
        </div>
        <div class="panel-body">
          ${this.notifications.length === 0
            ? html`<div class="empty">No notifications yet</div>`
            : html`${Array.from(groups.entries()).map(([label, items]) => html`
                <div class="day-divider">${label}</div>
                ${items.map(n => html`
                  <div class="notif-item ${n.read ? '' : 'unread'}"
                       @click=${() => this._handleNotifClick(n)}
                       tabindex="0"
                       role="button"
                       aria-label="${n.title}">
                    <div class="notif-icon ${this._getIconClass(n.type)}">${this._getIcon(n.type)}</div>
                    <div class="notif-body">
                      <div class="notif-title">${n.title}</div>
                      <div class="notif-desc">${n.body}</div>
                      ${n.link ? html`<a class="notif-link" href="${n.link}" @click=${(e: Event) => e.stopPropagation()}>View</a>` : nothing}
                    </div>
                    ${!n.read ? html`<div class="unread-dot"></div>` : nothing}
                    <span class="notif-time">${formatRelativeTime(Math.floor(n.timestamp / 1000))}</span>
                  </div>
                `)}
              `)}`
          }
        </div>
      </div>
    `;
  }
}
