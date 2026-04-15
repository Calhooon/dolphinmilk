/**
 * Date range picker with preset periods and custom range.
 * Dispatches 'range-change' event when selection changes.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { getDateRange, setDateRange, type StoredDateRange } from '../lib/storage.js';

export type DatePreset = 'today' | '7d' | '30d' | 'mtd' | 'qtd' | 'ytd' | 'custom';

export interface DateRange {
  from: Date;
  to: Date;
  preset: DatePreset;
}

const PRESET_LABELS: Record<DatePreset, string> = {
  today: 'Today',
  '7d': '7 Days',
  '30d': '30 Days',
  mtd: 'MTD',
  qtd: 'QTD',
  ytd: 'YTD',
  custom: 'Custom',
};

/** Compute a DateRange from a preset. */
export function defaultRange(preset: DatePreset, customFrom?: string, customTo?: string): DateRange {
  const now = new Date();
  const to = now;
  let from: Date;

  switch (preset) {
    case 'today': {
      from = new Date(now.getFullYear(), now.getMonth(), now.getDate());
      break;
    }
    case '7d': {
      from = new Date(now.getTime() - 7 * 86_400_000);
      break;
    }
    case '30d': {
      from = new Date(now.getTime() - 30 * 86_400_000);
      break;
    }
    case 'mtd': {
      from = new Date(now.getFullYear(), now.getMonth(), 1);
      break;
    }
    case 'qtd': {
      const qMonth = Math.floor(now.getMonth() / 3) * 3;
      from = new Date(now.getFullYear(), qMonth, 1);
      break;
    }
    case 'ytd': {
      from = new Date(now.getFullYear(), 0, 1);
      break;
    }
    case 'custom': {
      from = customFrom ? new Date(customFrom) : new Date(now.getTime() - 30 * 86_400_000);
      if (customTo) {
        const end = new Date(customTo);
        end.setHours(23, 59, 59, 999);
        return { from, to: end, preset };
      }
      break;
    }
    default:
      from = new Date(now.getTime() - 30 * 86_400_000);
  }

  return { from, to, preset };
}

function toDateInputValue(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

@customElement('dm-date-range')
export class WormDateRange extends LitElement {
  @property({ attribute: false }) value: DateRange = this._restoreRange();

  @state() private customFrom = '';
  @state() private customTo = '';

  static styles = css`
    :host { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }

    .preset-btn {
      padding: 4px 10px;
      font-size: 11px;
      font-family: var(--sans, sans-serif);
      font-weight: 500;
      color: var(--text-dim, rgba(255,255,255,0.5));
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 16px;
      cursor: pointer;
      transition: all 0.15s ease;
      white-space: nowrap;
    }

    .preset-btn:hover {
      color: var(--text, #e0e0e8);
      border-color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .preset-btn.active {
      color: var(--accent, #14A8C4);
      border-color: var(--accent, #14A8C4);
      background: rgba(20, 168, 196, 0.1);
      font-weight: 600;
    }

    .custom-inputs {
      display: flex;
      align-items: center;
      gap: 4px;
    }

    .date-input {
      padding: 3px 6px;
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text, #e0e0e8);
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 4px;
      outline: none;
      width: 110px;
    }

    .date-input:focus {
      border-color: var(--accent, #14A8C4);
    }

    /* Style the date input color scheme for dark mode */
    .date-input::-webkit-calendar-picker-indicator {
      filter: invert(0.7);
      cursor: pointer;
    }

    .dash {
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 11px;
    }
  `;

  private _restoreRange(): DateRange {
    const stored = getDateRange();
    return defaultRange(
      stored.preset as DatePreset,
      stored.from,
      stored.to,
    );
  }

  private _selectPreset(preset: DatePreset) {
    if (preset === 'custom') {
      // Initialize custom inputs from current range
      this.customFrom = toDateInputValue(this.value.from);
      this.customTo = toDateInputValue(this.value.to);
      this.value = { ...this.value, preset: 'custom' };
      this._persist();
      this._dispatch();
      return;
    }
    this.value = defaultRange(preset);
    this._persist();
    this._dispatch();
  }

  private _onCustomChange() {
    if (this.customFrom && this.customTo) {
      this.value = defaultRange('custom', this.customFrom, this.customTo);
      this._persist();
      this._dispatch();
    }
  }

  private _persist() {
    const stored: StoredDateRange = { preset: this.value.preset };
    if (this.value.preset === 'custom') {
      stored.from = toDateInputValue(this.value.from);
      stored.to = toDateInputValue(this.value.to);
    }
    setDateRange(stored);
  }

  private _dispatch() {
    this.dispatchEvent(new CustomEvent('range-change', {
      detail: { from: this.value.from, to: this.value.to, preset: this.value.preset },
      bubbles: true,
      composed: true,
    }));
  }

  render() {
    const presets: DatePreset[] = ['today', '7d', '30d', 'mtd', 'qtd', 'ytd', 'custom'];

    return html`
      ${presets.map(p => html`
        <button
          class="preset-btn ${this.value.preset === p ? 'active' : ''}"
          @click=${() => this._selectPreset(p)}
        >${PRESET_LABELS[p]}</button>
      `)}
      ${this.value.preset === 'custom' ? html`
        <div class="custom-inputs">
          <input
            type="date"
            class="date-input"
            .value=${this.customFrom}
            @change=${(e: Event) => { this.customFrom = (e.target as HTMLInputElement).value; this._onCustomChange(); }}
          />
          <span class="dash">\u2013</span>
          <input
            type="date"
            class="date-input"
            .value=${this.customTo}
            @change=${(e: Event) => { this.customTo = (e.target as HTMLInputElement).value; this._onCustomChange(); }}
          />
        </div>
      ` : nothing}
    `;
  }
}
