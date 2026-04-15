/**
 * Conversation header bar for the chat view.
 * Shows title, "All Chats" link, "New Chat" button, and share/export menu.
 */

import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import '../../components/share-menu.js';

@customElement('dm-chat-header')
export class WormChatHeader extends LitElement {
  @property() sessionId: string | null = null;
  @property() conversationTitle: string | null = null;
  @property() taskId: string | null = null;
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);

  @state() private shareOpen = false;

  static styles = css`
    :host { display: contents; }

    .conv-header {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 16px;
      background: var(--bg-surface, #0F2337);
      border-bottom: 1px solid var(--border, #1A3550);
      flex-shrink: 0;
    }

    .conv-title {
      flex: 1;
      font-size: 13px;
      font-weight: 500;
      color: var(--text, #e0e0e8);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .conv-header a, .conv-header button.link-btn {
      font-size: 12px;
      color: var(--accent, #14A8C4);
      text-decoration: none;
      cursor: pointer;
      background: none;
      border: none;
      padding: 4px 8px;
      border-radius: 4px;
      font-family: var(--sans, sans-serif);
    }

    .conv-header a:hover, .conv-header button.link-btn:hover {
      background: rgba(20, 168, 196, 0.08);
    }

    .share-wrap {
      position: relative;
    }
  `;

  render() {
    if (!this.sessionId) return '';
    return html`
      <div class="conv-header">
        <span class="conv-title">${this.conversationTitle ?? this.sessionId}</span>
        <div class="share-wrap">
          <button class="link-btn"
                  @click=${() => { this.shareOpen = !this.shareOpen; }}
                  aria-label="Share conversation">Share</button>
          <dm-share-menu
            .open=${this.shareOpen}
            .taskId=${this.taskId ?? ''}
            .sessionId=${this.sessionId ?? ''}
            .fetchFn=${this.fetchFn}
            @close=${() => { this.shareOpen = false; }}
          ></dm-share-menu>
        </div>
        <a href="#conversations">All Chats</a>
        <button class="link-btn" @click=${this._handleNew}>New Chat</button>
      </div>
    `;
  }

  private _handleNew() {
    this.dispatchEvent(new CustomEvent('new-conversation', { bubbles: true, composed: true }));
  }
}
