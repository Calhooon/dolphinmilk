/**
 * Chat input row with model picker flyout.
 * Dispatches 'send', 'cancel', and 'model-change' events.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import { getModel, setModel, type CurrencyDisplay } from '../../lib/storage.js';
import { formatTokens } from '../../lib/util.js';
import { formatInlineCurrency } from '../../lib/usd.js';
import type { Attachment } from '../../lib/ui-types.js';

const MAX_FILE_SIZE = 5 * 1024 * 1024; // 5MB
const MAX_FILES = 10;
const ACCEPTED_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];

/** Model definitions grouped by provider. */
export interface ModelDef {
  id: string;
  label: string;
  cost: string;
  provider: string;
}

export const MODELS: ModelDef[] = [
  // OpenAI
  { id: 'gpt-5-nano',        label: 'GPT-5 Nano',      cost: '$0.05 / $0.40',   provider: 'OpenAI' },
  { id: 'gpt-5-mini',        label: 'GPT-5 Mini',      cost: '$0.25 / $2',      provider: 'OpenAI' },
  { id: 'gpt-5',             label: 'GPT-5',           cost: '$1.25 / $10',     provider: 'OpenAI' },
  { id: 'gpt-5.2',           label: 'GPT-5.2',         cost: '$1.75 / $14',     provider: 'OpenAI' },
  { id: 'o4-mini',           label: 'o4 Mini',          cost: '$1.10 / $4.40',   provider: 'OpenAI' },
  { id: 'gpt-5.2-pro',       label: 'GPT-5.2 Pro',     cost: '$21 / $168',      provider: 'OpenAI' },
  // Claude
  { id: 'claude-haiku-4-5',  label: 'Haiku 4.5',       cost: '$1 / $5',         provider: 'Claude' },
  { id: 'claude-sonnet-4-6', label: 'Sonnet 4.6',      cost: '$3 / $15',        provider: 'Claude' },
  { id: 'claude-opus-4-6',   label: 'Opus 4.6',        cost: '$5 / $25',        provider: 'Claude' },
];


@customElement('dm-chat-input')
export class WormChatInput extends LitElement {
  @property({ type: Boolean }) busy = false;
  @property() currentTaskId: string | null = null;
  @property({ type: Number }) sessionSats = 0;
  @property({ type: Number }) sessionTokens = 0;
  @property() currencyMode: CurrencyDisplay = 'usd-first';
  @property({ type: Number }) usdRate = 0;

  @state() private inputValue = '';
  @state() selectedModel: string = getModel();
  @state() private modelPickerOpen = false;
  @state() private attachments: Attachment[] = [];
  @state() private attachmentError: string | null = null;

  @query('#file-input') private fileInput!: HTMLInputElement;

  static styles = css`
    :host { display: contents; }

    .input-row {
      display: flex;
      gap: 8px;
      padding: 12px 16px;
      background: var(--bg-surface, #0F2337);
      border-top: 1px solid var(--border, #1A3550);
    }

    .input-wrapper {
      flex: 1;
      position: relative;
      display: flex;
      align-items: center;
    }

    .input-icon {
      position: absolute;
      left: 12px;
      display: flex;
      align-items: center;
      color: var(--text-dim, rgba(255,255,255,0.5));
      pointer-events: none;
      transition: color 0.15s;
    }

    .input-wrapper:focus-within .input-icon {
      color: var(--accent, #14A8C4);
    }

    input {
      flex: 1;
      background: var(--bg-input, #1A2D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text, #e0e0e8);
      padding: 10px 14px 10px 36px;
      border-radius: var(--radius-md, 14px);
      font-size: 14px;
      font-family: var(--sans, sans-serif);
      outline: none;
      transition: border-color 0.15s, box-shadow 0.15s;
    }

    input:focus {
      border-color: var(--accent, #14A8C4);
      box-shadow: 0 0 0 2px rgba(20, 168, 196, 0.25);
    }

    input::placeholder { color: var(--text-dim, rgba(255,255,255,0.5)); }
    input:disabled { opacity: 0.5; }

    button {
      background: var(--accent, #14A8C4);
      color: #fff;
      border: none;
      padding: 10px 20px;
      border-radius: 8px;
      font-size: 14px;
      font-weight: 500;
      cursor: pointer;
      transition: background 0.15s;
    }

    button:hover:not(:disabled) { background: var(--accent-dim, #0E8FA8); }
    button:disabled { opacity: 0.5; cursor: not-allowed; }
    button:focus-visible { outline: 2px solid var(--accent, #14A8C4); outline-offset: 2px; }

    button.cancel {
      background: var(--error, #fd5454);
      padding: 10px 16px;
    }

    button.cancel:hover:not(:disabled) { background: #dc2626; }

    /* ---- Model picker ---- */

    .model-picker-anchor { position: relative; }

    .model-trigger {
      display: flex;
      align-items: center;
      gap: 6px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text, #e0e0e8);
      padding: 8px 12px;
      border-radius: 8px;
      font-size: 12px;
      font-family: var(--mono, monospace);
      cursor: pointer;
      transition: border-color 0.15s, background 0.15s;
      white-space: nowrap;
      min-width: 0;
    }

    .model-trigger:hover {
      border-color: var(--accent, #14A8C4);
      background: rgba(20, 168, 196, 0.05);
    }

    .model-trigger:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    .model-trigger .provider-dot {
      width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0;
    }

    .model-trigger .provider-dot.openai { background: #10a37f; }
    .model-trigger .provider-dot.claude { background: #d97706; }

    .model-trigger .chevron {
      font-size: 10px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      transition: transform 0.15s;
    }

    .model-trigger .chevron.open { transform: rotate(180deg); }

    .model-flyout {
      position: absolute;
      bottom: calc(100% + 6px);
      left: 0;
      min-width: 240px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      box-shadow: 0 -8px 32px rgba(0, 0, 0, 0.5);
      overflow: hidden;
      z-index: 100;
      animation: flyout-in 0.12s ease-out;
    }

    @keyframes flyout-in {
      from { opacity: 0; transform: translateY(6px); }
      to   { opacity: 1; transform: translateY(0); }
    }

    .model-flyout .group-label {
      padding: 8px 14px 4px;
      font-size: 10px; font-weight: 600;
      text-transform: uppercase; letter-spacing: 0.06em;
      color: var(--text-dim, rgba(255,255,255,0.5));
      display: flex; align-items: center; gap: 6px;
    }

    .model-flyout .group-label .gdot {
      width: 5px; height: 5px; border-radius: 50%;
    }

    .model-flyout .group-label .gdot.openai { background: #10a37f; }
    .model-flyout .group-label .gdot.claude { background: #d97706; }

    .model-option {
      display: flex; align-items: center; gap: 8px;
      padding: 7px 14px; cursor: pointer;
      transition: background 0.1s;
    }

    .model-option:hover { background: rgba(20, 168, 196, 0.08); }
    .model-option.active { background: rgba(20, 168, 196, 0.12); }

    .model-option .check {
      width: 14px; text-align: center; font-size: 11px;
      color: var(--accent, #14A8C4); flex-shrink: 0;
    }

    .model-option .model-name {
      flex: 1; font-size: 13px; color: var(--text, #e0e0e8);
    }

    .model-option .model-cost {
      font-size: 11px; color: var(--text-dim, rgba(255,255,255,0.5));
      font-family: var(--mono, monospace);
    }

    .model-flyout .separator {
      height: 1px; background: var(--border, #1A3550); margin: 4px 0;
    }

    .model-flyout-scrim {
      position: fixed; inset: 0; z-index: 99;
    }

    .sats-pill {
      display: flex; align-items: center; gap: 4px;
      font-family: var(--mono, monospace); font-size: 11px;
      color: var(--warning, #fcbe2d);
      padding: 6px 10px;
      background: rgba(252, 190, 45, 0.08);
      border: 1px solid rgba(252, 190, 45, 0.2);
      border-radius: 6px;
      white-space: nowrap; flex-shrink: 0;
    }

    .tokens-pill {
      display: flex; align-items: center; gap: 4px;
      font-family: var(--mono, monospace); font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      padding: 6px 10px;
      background: rgba(255, 255, 255, 0.04);
      border: 1px solid rgba(255, 255, 255, 0.1);
      border-radius: 6px;
      white-space: nowrap; flex-shrink: 0;
    }

    /* ---- Attachment button ---- */

    .attach-btn {
      background: none;
      border: none;
      padding: 8px;
      cursor: pointer;
      color: var(--text-dim, rgba(255,255,255,0.5));
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 6px;
      transition: color 0.15s, background 0.15s;
      flex-shrink: 0;
    }

    .attach-btn:hover {
      color: var(--text, #e0e0e8);
      background: rgba(255,255,255,0.06);
    }

    .attach-btn:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    /* ---- Attachment previews ---- */

    .attachment-preview-row {
      display: flex;
      gap: 8px;
      padding: 8px 16px 0;
      background: var(--bg-surface, #0F2337);
      flex-wrap: wrap;
      align-items: flex-start;
    }

    .attachment-thumb {
      position: relative;
      height: 48px;
      border-radius: var(--radius-sm, 8px);
      border: 1px solid var(--border, #1A3550);
      overflow: visible;
    }

    .attachment-thumb img {
      height: 48px;
      border-radius: var(--radius-sm, 8px);
      display: block;
      object-fit: cover;
    }

    .attachment-thumb .remove-btn {
      position: absolute;
      top: -6px;
      right: -6px;
      width: 18px;
      height: 18px;
      border-radius: 50%;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 11px;
      line-height: 1;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 0;
      transition: color 0.15s, background 0.15s;
    }

    .attachment-thumb .remove-btn:hover {
      color: var(--error, #fd5454);
      background: var(--bg-surface, #0F2337);
    }

    .attachment-error {
      font-size: 12px;
      color: var(--error, #fd5454);
      padding: 4px 16px 0;
      background: var(--bg-surface, #0F2337);
    }

    .drop-active input {
      border-color: var(--accent, #14A8C4);
      box-shadow: 0 0 0 2px rgba(20, 168, 196, 0.25);
    }

    @media (max-width: 639px) {
      .input-row { flex-wrap: wrap; padding: 8px 12px; gap: 6px; }
      .input-wrapper { order: -1; width: 100%; }
      input { font-size: 16px; }
      .model-picker-anchor { width: auto; }
      button, button.cancel { flex-shrink: 0; min-height: 44px; }
      .model-trigger { min-height: 44px; }
      .model-trigger .model-label {
        max-width: 80px; overflow: hidden; text-overflow: ellipsis;
      }
      .model-flyout {
        min-width: 200px;
        left: 0;
        right: auto;
        max-width: calc(100vw - 24px);
      }
      .attachment-preview-row { padding: 6px 12px 0; }
    }
  `;

  private boundEscHandler = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && this.modelPickerOpen) {
      this.modelPickerOpen = false;
    }
  };

  connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this.boundEscHandler);
    // Dispatch initial model so status bar picks it up
    requestAnimationFrame(() => {
      this.dispatchEvent(
        new CustomEvent('model-change', {
          detail: { model: this.selectedModel },
          bubbles: true, composed: true,
        })
      );
    });
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this.boundEscHandler);
  }

  render() {
    const hasAttachments = this.attachments.length > 0;
    return html`
      ${this.modelPickerOpen
        ? html`<div class="model-flyout-scrim" @click=${this._closeModelPicker}></div>`
        : ''}
      ${this.attachmentError ? html`<div class="attachment-error">${this.attachmentError}</div>` : nothing}
      ${hasAttachments ? html`
        <div class="attachment-preview-row">
          ${this.attachments.map((att, i) => html`
            <div class="attachment-thumb">
              <img src="data:${att.mime_type};base64,${att.data}" alt="${att.filename}" />
              <button class="remove-btn" @click=${() => this._removeAttachment(i)} title="Remove">&times;</button>
            </div>
          `)}
        </div>
      ` : nothing}
      <div class="input-row">
        <div class="model-picker-anchor">
          ${this._renderModelTrigger()}
          ${this.modelPickerOpen ? this._renderModelFlyout() : ''}
        </div>
        <div class="input-wrapper ${this._dragOver ? 'drop-active' : ''}"
          @dragover=${this._handleDragOver}
          @dragleave=${this._handleDragLeave}
          @drop=${this._handleDrop}
        >
          <span class="input-icon">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
            </svg>
          </span>
          <input
            type="text"
            placeholder="Message Dolphin Milk..."
            .value=${this.inputValue}
            @input=${(e: Event) => (this.inputValue = (e.target as HTMLInputElement).value)}
            @keydown=${(e: KeyboardEvent) => e.key === 'Enter' && this._handleSend()}
            @paste=${this._handlePaste}
            ?disabled=${this.busy}
          />
        </div>
        <input id="file-input" type="file" accept="image/png,image/jpeg,image/gif,image/webp" multiple
          style="display:none" @change=${this._handleFileSelect} />
        <button class="attach-btn" @click=${this._openFilePicker} title="Attach images" aria-label="Attach images">
          <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21.44 11.05l-9.19 9.19a6 6 0 01-8.49-8.49l9.19-9.19a4 4 0 015.66 5.66l-9.2 9.19a2 2 0 01-2.83-2.83l8.49-8.48"/>
          </svg>
        </button>
        ${this.sessionTokens > 0
          ? html`<span class="tokens-pill">${formatTokens(this.sessionTokens)} tok</span>`
          : ''}
        ${this.sessionSats > 0
          ? html`<span class="sats-pill">\u26A1 ${formatInlineCurrency(this.sessionSats, this.usdRate, this.currencyMode)}</span>`
          : ''}
        ${this.busy && this.currentTaskId
          ? html`<button class="cancel" @click=${this._handleCancel}>Stop</button>`
          : html`<button @click=${this._handleSend} ?disabled=${this.busy || (!this.inputValue.trim() && this.attachments.length === 0)}>Send</button>`
        }
      </div>
    `;
  }

  // --------------------------------------------------------------------------
  // Model picker
  // --------------------------------------------------------------------------

  private _getSelectedModelDef(): ModelDef | undefined {
    return MODELS.find(m => m.id === this.selectedModel);
  }

  private _closeModelPicker() { this.modelPickerOpen = false; }

  private _selectModel(model: ModelDef) {
    this.selectedModel = model.id;
    this.modelPickerOpen = false;
    setModel(model.id);
    this.dispatchEvent(
      new CustomEvent('model-change', { detail: { model: model.id }, bubbles: true, composed: true })
    );
  }

  private _renderModelTrigger() {
    const def = this._getSelectedModelDef();
    const provider = (def?.provider ?? 'OpenAI').toLowerCase();
    return html`
      <button
        class="model-trigger"
        @click=${() => { this.modelPickerOpen = !this.modelPickerOpen; }}
        title="Switch model"
        aria-label="Switch AI model"
        aria-haspopup="listbox"
        aria-expanded=${this.modelPickerOpen}
      >
        <span class="provider-dot ${provider}"></span>
        <span class="model-label">${def?.label ?? this.selectedModel}</span>
        <span class="chevron ${this.modelPickerOpen ? 'open' : ''}">▾</span>
      </button>
    `;
  }

  private _renderModelFlyout() {
    const providers = ['OpenAI', 'Claude'];
    return html`
      <div class="model-flyout" role="listbox" aria-label="Select model">
        ${providers.map((provider, i) => {
          const group = MODELS.filter(m => m.provider === provider);
          return html`
            ${i > 0 ? html`<div class="separator"></div>` : ''}
            <div class="group-label">
              <span class="gdot ${provider.toLowerCase()}"></span>
              ${provider}
            </div>
            ${group.map(m => html`
              <div
                class="model-option ${m.id === this.selectedModel ? 'active' : ''}"
                role="option"
                aria-selected=${m.id === this.selectedModel}
                @click=${() => this._selectModel(m)}
              >
                <span class="check">${m.id === this.selectedModel ? '\u25CF' : ''}</span>
                <span class="model-name">${m.label}</span>
                <span class="model-cost">${m.cost}</span>
              </div>
            `)}
          `;
        })}
      </div>
    `;
  }

  // --------------------------------------------------------------------------
  // Event dispatchers
  // --------------------------------------------------------------------------

  private _handleSend() {
    const text = this.inputValue.trim();
    if (!text && this.attachments.length === 0) return;
    const attachments = this.attachments.length > 0 ? [...this.attachments] : undefined;
    this.inputValue = '';
    this.attachments = [];
    this.attachmentError = null;
    this.dispatchEvent(
      new CustomEvent('send', { detail: { text, attachments }, bubbles: true, composed: true })
    );
  }

  private _handleCancel() {
    this.dispatchEvent(
      new CustomEvent('cancel', { bubbles: true, composed: true })
    );
  }

  // --------------------------------------------------------------------------
  // File attachment handling
  // --------------------------------------------------------------------------

  @state() private _dragOver = false;

  private _openFilePicker() {
    this.fileInput?.click();
  }

  private _handleFileSelect(e: Event) {
    const input = e.target as HTMLInputElement;
    if (input.files) {
      this._addFiles(Array.from(input.files));
    }
    // Reset so the same file can be re-selected
    input.value = '';
  }

  private _handleDragOver(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    this._dragOver = true;
  }

  private _handleDragLeave(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    this._dragOver = false;
  }

  private _handleDrop(e: DragEvent) {
    e.preventDefault();
    e.stopPropagation();
    this._dragOver = false;
    if (e.dataTransfer?.files) {
      this._addFiles(Array.from(e.dataTransfer.files));
    }
  }

  private _handlePaste(e: ClipboardEvent) {
    const items = e.clipboardData?.items;
    if (!items) return;
    const imageFiles: File[] = [];
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.kind === 'file' && ACCEPTED_TYPES.includes(item.type)) {
        const file = item.getAsFile();
        if (file) imageFiles.push(file);
      }
    }
    if (imageFiles.length > 0) {
      e.preventDefault();
      this._addFiles(imageFiles);
    }
  }

  private _removeAttachment(index: number) {
    this.attachments = this.attachments.filter((_, i) => i !== index);
    this.attachmentError = null;
  }

  private async _addFiles(files: File[]) {
    this.attachmentError = null;

    // Filter to accepted image types
    const imageFiles = files.filter(f => ACCEPTED_TYPES.includes(f.type));
    if (imageFiles.length === 0) {
      this.attachmentError = 'Only PNG, JPEG, GIF, and WebP images are supported.';
      return;
    }

    // Check total count
    if (this.attachments.length + imageFiles.length > MAX_FILES) {
      this.attachmentError = `Maximum ${MAX_FILES} files allowed.`;
      return;
    }

    // Check individual sizes
    for (const file of imageFiles) {
      if (file.size > MAX_FILE_SIZE) {
        this.attachmentError = `"${file.name}" exceeds the 5MB size limit.`;
        return;
      }
    }

    // Read files as base64
    const newAttachments: Attachment[] = [];
    for (const file of imageFiles) {
      try {
        const data = await this._readFileAsBase64(file);
        newAttachments.push({
          data,
          mime_type: file.type,
          filename: file.name,
        });
      } catch {
        this.attachmentError = `Failed to read "${file.name}".`;
        return;
      }
    }

    this.attachments = [...this.attachments, ...newAttachments];
  }

  private _readFileAsBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        // Strip the data:...;base64, prefix
        const base64 = result.split(',')[1];
        if (base64) {
          resolve(base64);
        } else {
          reject(new Error('Failed to encode file'));
        }
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
  }
}
