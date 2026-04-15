/**
 * Tooltip and help icon components for contextual help.
 */

import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';

@customElement('dm-tooltip')
export class WormTooltip extends LitElement {
  @property() text = '';
  @property() position: 'top' | 'bottom' = 'top';
  @state() private visible = false;
  @state() private tipTop = 0;
  @state() private tipLeft = 0;

  static styles = css`
    :host {
      position: relative;
      display: inline-flex;
      align-items: center;
    }
    .trigger {
      display: inline-flex;
      align-items: center;
      cursor: help;
    }
    .tip {
      position: fixed;
      z-index: 10000;
      background: var(--bg, #0B1929);
      border: 1px solid var(--border, #1A3550);
      border-radius: 8px;
      padding: 8px 12px;
      font-size: 12px;
      line-height: 1.5;
      color: var(--text, rgba(255,255,255,0.8));
      max-width: 280px;
      width: max-content;
      pointer-events: none;
      opacity: 0;
      transition: opacity 0.15s ease;
      box-shadow: 0 4px 12px rgba(0,0,0,0.3);
      font-family: var(--sans, sans-serif);
      font-weight: 400;
    }
    .tip.show { opacity: 1; pointer-events: auto; }
  `;

  private show() {
    const trigger = this.renderRoot.querySelector('.trigger');
    const tip = this.renderRoot.querySelector('.tip') as HTMLElement;
    if (!trigger || !tip) return;

    const tr = trigger.getBoundingClientRect();
    const tipW = tip.offsetWidth;
    const tipH = tip.offsetHeight;

    let top: number;
    let left = tr.left + tr.width / 2 - tipW / 2;

    if (this.position === 'top') {
      top = tr.top - tipH - 6;
      if (top < 8) top = tr.bottom + 6;
    } else {
      top = tr.bottom + 6;
      if (top + tipH > window.innerHeight - 8) top = tr.top - tipH - 6;
    }

    if (left < 8) left = 8;
    if (left + tipW > window.innerWidth - 8) left = window.innerWidth - tipW - 8;

    this.tipTop = top;
    this.tipLeft = left;
    this.visible = true;
  }

  private hide() {
    this.visible = false;
  }

  render() {
    return html`
      <span class="trigger" tabindex="0"
        @mouseenter=${this.show}
        @mouseleave=${this.hide}
        @focusin=${this.show}
        @focusout=${this.hide}
      ><slot></slot></span>
      ${this.text ? html`<span class="tip ${this.visible ? 'show' : ''}"
        style="top:${this.tipTop}px;left:${this.tipLeft}px"
      >${this.text}</span>` : ''}
    `;
  }
}

@customElement('dm-help')
export class WormHelp extends LitElement {
  @property() text = '';

  static styles = css`
    :host { display: inline-flex; align-items: center; margin-left: 6px; }
    .help-icon {
      width: 16px; height: 16px; border-radius: 50%;
      background: rgba(255,255,255,0.08);
      color: var(--text-dim, rgba(255,255,255,0.4));
      font-size: 10px; font-weight: 700;
      display: inline-flex; align-items: center; justify-content: center;
      cursor: help;
      transition: background 0.15s ease, color 0.15s ease;
    }
    .help-icon:hover {
      background: rgba(20, 168, 196, 0.15);
      color: var(--accent, #14A8C4);
    }
  `;

  render() {
    return html`
      <dm-tooltip text=${this.text}>
        <span class="help-icon">?</span>
      </dm-tooltip>
    `;
  }
}
