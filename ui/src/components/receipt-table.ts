/**
 * Receipt cost breakdown table.
 * Displays BEEF payment receipts with totals.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { formatUsd, formatInlineCurrency, formatInlineCurrencyAlt } from '../lib/usd.js';
import { formatSats } from '../lib/util.js';
import { getCurrencyDisplay, type CurrencyDisplay } from '../lib/storage.js';
import type { ReceiptDetail } from '../lib/shared-types.js';

@customElement('dm-receipt-table')
export class WormReceiptTable extends LitElement {
  @property({ attribute: false }) receipts: ReceiptDetail[] = [];
  @property({ type: Number }) totalPaid = 0;
  @property({ type: Number }) totalRefunded = 0;
  @property({ type: Number }) usdRate = 0;
  @property({ attribute: false }) currencyMode: CurrencyDisplay = getCurrencyDisplay();

  static styles = css`
    :host { display: block; }

    .receipt-section {
      margin-top: 24px;
      padding-top: 20px;
      border-top: 1px solid var(--border, #1A3550);
    }

    .receipt-title {
      font-size: 16px; font-weight: 600;
      color: var(--text-bright, #fff);
      font-family: var(--sans, sans-serif);
      margin-bottom: 4px;
    }

    .receipt-subtitle {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      margin-bottom: 12px;
    }

    .receipt-table {
      width: 100%; border-collapse: collapse;
      font-size: 12px; font-family: var(--mono, monospace);
    }

    .receipt-table th {
      text-align: left; padding: 6px 10px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 10px; text-transform: uppercase;
      letter-spacing: 0.5px;
      border-bottom: 1px solid var(--border, #1A3550);
      font-weight: 600;
    }

    .receipt-table td {
      padding: 6px 10px;
      color: var(--text, #e0e0e8);
      border-bottom: 1px solid rgba(49, 61, 79, 0.5);
    }

    .receipt-table tr:hover td { background: rgba(255, 255, 255, 0.02); }
    .receipt-table .sats { color: var(--warning, #fcbe2d); }
    .receipt-table .refund { color: var(--success, #00b69b); }

    .receipt-totals {
      display: flex; gap: 20px; margin-top: 10px;
      padding: 8px 10px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 6px;
      font-size: 12px; font-family: var(--mono, monospace);
    }

    .receipt-totals .label { color: var(--text-dim, rgba(255,255,255,0.5)); }
    .receipt-totals .value { color: var(--text-bright, #fff); font-weight: 600; }
    .receipt-totals .value.sats { color: var(--warning, #fcbe2d); }
    .receipt-totals .value.refund { color: var(--success, #00b69b); }
  `;

  render() {
    if (this.receipts.length === 0) return nothing;

    const netSats = this.totalPaid - this.totalRefunded;

    return html`
      <div class="receipt-section">
        <div class="receipt-title">Cost Breakdown</div>
        <div class="receipt-subtitle">
          ${this.receipts.length} BEEF receipt${this.receipts.length !== 1 ? 's' : ''} recorded
        </div>
        <table class="receipt-table">
          <thead>
            <tr>
              <th>Iter</th>
              <th>Model</th>
              <th>Tokens</th>
              <th>Paid</th>
              <th>Refunded</th>
              <th>Effective</th>
              <th>USD</th>
            </tr>
          </thead>
          <tbody>
            ${this.receipts.map((r) => html`
              <tr>
                <td>${r.iteration}</td>
                <td>${r.model}</td>
                <td>${r.tokens.toLocaleString()}</td>
                <td class="sats" title="${formatInlineCurrencyAlt(r.sats_paid, this.usdRate, this.currencyMode)}">${formatInlineCurrency(r.sats_paid, this.usdRate, this.currencyMode)}</td>
                <td class="refund" title="${r.sats_refunded > 0 ? formatInlineCurrencyAlt(r.sats_refunded, this.usdRate, this.currencyMode) : ''}">${r.sats_refunded > 0 ? formatInlineCurrency(r.sats_refunded, this.usdRate, this.currencyMode) : '-'}</td>
                <td class="sats" title="${formatInlineCurrencyAlt(r.sats_effective, this.usdRate, this.currencyMode)}">${formatInlineCurrency(r.sats_effective, this.usdRate, this.currencyMode)}</td>
                <td>${this.usdRate ? formatUsd(r.sats_effective, this.usdRate) : '-'}</td>
              </tr>
            `)}
          </tbody>
        </table>
        <div class="receipt-totals">
          <span>
            <span class="label">Total Paid: </span>
            <span class="value sats" title="${formatInlineCurrencyAlt(this.totalPaid, this.usdRate, this.currencyMode)}">${formatInlineCurrency(this.totalPaid, this.usdRate, this.currencyMode)}</span>
          </span>
          <span>
            <span class="label">Refunded: </span>
            <span class="value refund" title="${formatInlineCurrencyAlt(this.totalRefunded, this.usdRate, this.currencyMode)}">${formatInlineCurrency(this.totalRefunded, this.usdRate, this.currencyMode)}</span>
          </span>
          <span>
            <span class="label">Net: </span>
            <span class="value sats" title="${formatInlineCurrencyAlt(netSats, this.usdRate, this.currencyMode)}">${formatInlineCurrency(netSats, this.usdRate, this.currencyMode)}</span>
          </span>
        </div>
      </div>
    `;
  }
}
