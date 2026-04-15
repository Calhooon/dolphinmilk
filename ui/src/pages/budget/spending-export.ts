/**
 * CSV export component for spending data.
 * "Hand it to your auditor."
 */

import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { formatUsd } from '../../lib/usd.js';
import type { BudgetDetailResponse, SpendingEntry } from '../../lib/shared-types.js';
import { friendlyName } from '../../lib/constants.js';

@customElement('dm-spending-export')
export class WormSpendingExport extends LitElement {
  @property({ type: Object }) fetchFn: typeof fetch = fetch;
  @property({ type: Number }) usdRate = 0;

  @state() private exporting = false;

  static styles = css`
    .export-btn {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 8px 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px;
      color: var(--text, #e0e0e8);
      font-size: 13px;
      cursor: pointer;
      transition: all 0.15s ease;
    }
    .export-btn:hover {
      background: var(--accent, #14A8C4);
      color: #fff;
      border-color: var(--accent, #14A8C4);
    }
    .export-btn:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }
    .export-btn svg {
      width: 14px;
      height: 14px;
    }
  `;

  render() {
    return html`
      <button class="export-btn" @click=${this.exportCsv} ?disabled=${this.exporting}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/>
          <polyline points="7 10 12 15 17 10"/>
          <line x1="12" y1="15" x2="12" y2="3"/>
        </svg>
        ${this.exporting ? 'Exporting...' : 'Export CSV'}
      </button>
    `;
  }

  private async exportCsv() {
    this.exporting = true;
    try {
      const resp = await this.fetchFn('/budget/detail');
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      const detail: BudgetDetailResponse = await resp.json();

      const rows: string[] = [];
      rows.push('Date,Service,Operation,Amount (sats),Amount (USD)');

      for (const entry of detail.entries) {
        const usd = this.usdRate > 0 ? formatUsd(entry.sats, this.usdRate) : '';
        rows.push([
          entry.timestamp,
          friendlyName(entry.service),
          entry.operation ?? '',
          entry.sats,
          usd,
        ].map(v => `"${String(v).replace(/"/g, '""')}"`).join(','));
      }

      const csv = rows.join('\n');
      const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `dolphin-milk-spending-${new Date().toISOString().slice(0, 10)}.csv`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      console.error('CSV export failed:', e);
    } finally {
      this.exporting = false;
    }
  }
}
