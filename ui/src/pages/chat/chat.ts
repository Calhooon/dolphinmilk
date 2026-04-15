/**
 * Main chat panel — transcript-driven streaming UI.
 * Delegates header to <dm-chat-header> and input to <dm-chat-input>.
 */

import { LitElement, html, css } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import type { ChatMessage, ToolCallInfo, Attachment } from '../../lib/ui-types.js';
import type { ConversationDetail } from '../../lib/shared-types.js';
import { TranscriptPoller } from '../../lib/transcript-poller.js';
import { getSessionId, setSessionId, clearSessionId, getCurrencyDisplay, hasSeenOnboarding, markOnboardingSeen, type CurrencyDisplay } from '../../lib/storage.js';
import { fetchBsvUsdRate, formatInlineCurrency } from '../../lib/usd.js';
import '../../components/message.js';
import '../../components/tool-card.js';
import '../../components/snackbar.js';
import '../../components/onboarding.js';
import './chat-header.js';
import './chat-input.js';
import '../../components/budget-strip.js';

/** Throttle interval for batching DOM updates (ms). */
const SYNC_THROTTLE_MS = 80;

/**
 * Rewrite any 64-hex txid in `content` as a markdown link to WhatsOnChain.
 * Used for system-role messages (commission payment notices) so users can
 * click through to verify the on-chain payment. Idempotent: skips txids that
 * already appear inside an existing markdown link.
 */
function linkifyTxids(content: string): string {
  return content.replace(/\b([0-9a-f]{64})\b/gi, (match, txid, offset, full) => {
    const before = full.slice(Math.max(0, offset - 2), offset);
    if (before === '](') return match;
    return `[${txid.slice(0, 12)}…](https://whatsonchain.com/tx/${txid})`;
  });
}


@customElement('dm-chat')
export class WormChat extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property({ type: String }) sessionId: string | null = null;
  @property() currencyMode: CurrencyDisplay = getCurrencyDisplay();
  @state() private messages: ChatMessage[] = [];
  @state() private thinking = false;
  @state() private busy = false;
  @state() private currentToolCalls: ToolCallInfo[] = [];
  @state() private currentTaskId: string | null = null;
  @state() private conversationTitle: string | null = null;
  @state() private snackbar: { type: 'info' | 'warning' | 'error' | 'success'; message: string } | null = null;
  @state() private sessionSats = 0;
  @state() private usdRate = 0;
  @state() private sessionTokens = 0;
  @state() private iteration = 0;
  /** True when onboarding should be shown (first run, no conversations). */
  @state() private showOnboarding = false;
  /** Shown after task completes — links to conversation detail. */
  @state() private completionCard: { iterations: number; sats: number; conversationId: string } | null = null;

  @query('.messages') private messagesEl!: HTMLElement;

  /** True when user has scrolled up away from the bottom. */
  @state() private userScrolledUp = false;

  private messageQueue: string[] = [];
  private syncTimer: number | null = null;
  private loadedSessionId: string | null = null;
  private budgetWarningShown = false;

  /** Transcript poller for the current active task. */
  private poller: TranscriptPoller | null = null;
  /** Timer ID for polling. */
  private pollTimer: number | null = null;

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      flex: 1;
      min-height: 0;
    }

    .messages {
      flex: 1;
      overflow-y: auto;
      padding: 16px;
      display: flex;
      flex-direction: column;
    }

    .thinking {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 10px 14px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: 12px;
      border-bottom-left-radius: 4px;
      max-width: 85%;
      margin-bottom: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 13px;
    }

    .thinking-dots {
      display: flex;
      gap: 4px;
      align-items: center;
    }

    .dot {
      width: 6px; height: 6px; border-radius: 50%;
      background: var(--accent, #14A8C4);
      animation: thinking-pulse 1.4s ease-in-out infinite;
    }

    .dot:nth-child(2) { animation-delay: 0.2s; }
    .dot:nth-child(3) { animation-delay: 0.4s; }

    @keyframes thinking-pulse {
      0%, 80%, 100% { opacity: 0.25; transform: scale(0.8); }
      40% { opacity: 1; transform: scale(1); }
    }

    .thinking-label {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .tools-group {
      max-width: 85%;
      margin-bottom: 8px;
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .empty-state {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      color: var(--text-dim, rgba(255,255,255,0.5));
      gap: 8px;
    }

    .empty-state .title {
      font-size: 18px;
      color: var(--text, #e0e0e8);
    }

    .completion-card {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 10px 14px;
      margin: 8px 0;
      background: rgba(0, 182, 155, 0.08);
      border: 1px solid var(--success, #00b69b);
      border-radius: 10px;
      font-size: 13px;
      color: var(--text);
    }
    .completion-check {
      color: var(--success);
      font-size: 16px;
      font-weight: 700;
    }
    .completion-link {
      margin-left: auto;
      color: var(--accent);
      text-decoration: none;
      font-weight: 500;
      white-space: nowrap;
    }
    .completion-link:hover { text-decoration: underline; }

    .scroll-bottom-btn {
      position: absolute;
      bottom: 8px;
      left: 50%;
      transform: translateX(-50%);
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      color: var(--text-dim, rgba(255,255,255,0.5));
      padding: 6px 16px;
      border-radius: var(--radius-pill, 999px);
      font-size: 12px;
      cursor: pointer;
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
      transition: background 0.15s, color 0.15s;
      z-index: 10;
    }

    .scroll-bottom-btn:hover {
      background: var(--accent, #14A8C4);
      color: #fff;
      border-color: var(--accent, #14A8C4);
    }

    .messages-wrapper {
      position: relative;
      flex: 1;
      min-height: 0;
      display: flex;
      flex-direction: column;
    }

    .scroll-bottom-btn:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    @media (max-width: 639px) {
      .messages { padding: 12px 8px; }
      .thinking { max-width: 92%; }
      .tools-group { max-width: 100%; }
    }
  `;

  private boundScrollHandler = () => {
    if (!this.messagesEl) return;
    const el = this.messagesEl;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 150;
    if (nearBottom && this.userScrolledUp) {
      this.userScrolledUp = false;
    } else if (!nearBottom && !this.userScrolledUp) {
      this.userScrolledUp = true;
    }
  };

  private scrollListenerAttached = false;

  connectedCallback() {
    super.connectedCallback();
    // Only restore from localStorage if no sessionId was set by the parent
    // (e.g., from #conversation/{id} route). Use microtask to let Lit set
    // properties from the parent template first.
    queueMicrotask(() => {
      if (!this.sessionId) {
        const saved = getSessionId();
        if (saved) this.sessionId = saved;
      }
      this.detectActiveTask();
      this.checkOnboarding();
    });
    fetchBsvUsdRate().then(r => { this.usdRate = r; });
  }

  /** Check if onboarding should be shown (first run, no conversations, no session). */
  private async checkOnboarding() {
    if (hasSeenOnboarding() || this.sessionId || this.messages.length > 0) {
      this.showOnboarding = false;
      return;
    }
    try {
      const res = await this.fetchFn('/conversations');
      if (!res.ok) return;
      const data = await res.json();
      const convs = Array.isArray(data) ? data : [];
      this.showOnboarding = convs.length === 0;
    } catch {
      // If we can't check, don't show onboarding
    }
  }

  private handleOnboardingPrompt(e: CustomEvent) {
    this.showOnboarding = false;
    this.sendMessage(e.detail.text);
  }

  private handleOnboardingDismiss() {
    this.showOnboarding = false;
  }

  protected firstUpdated() {
    if (this.messagesEl && !this.scrollListenerAttached) {
      this.messagesEl.addEventListener('scroll', this.boundScrollHandler, { passive: true });
      this.scrollListenerAttached = true;
    }
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this.messagesEl && this.scrollListenerAttached) {
      this.messagesEl.removeEventListener('scroll', this.boundScrollHandler);
      this.scrollListenerAttached = false;
    }
    this.stopPollLoop();
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('sessionId') && this.sessionId && this.sessionId !== this.loadedSessionId) {
      this.loadConversationHistory(this.sessionId);
    }
  }

  private async loadConversationHistory(sessionId: string) {
    this.loadedSessionId = sessionId;
    try {
      const res = await this.fetchFn(`/conversations/${sessionId}`);
      if (!res.ok) return;
      const data: ConversationDetail = await res.json();
      this.conversationTitle = data.conversation.title;
      let msgs = this._buildMessagesFromConversation(data.messages ?? []);

      // Fallback: if tasks ran but no assistant messages were assembled,
      // try reconstructing from the last task's transcript.
      const hasAssistant = msgs.some(m => m.role === 'assistant');
      if (!hasAssistant && data.conversation.task_ids?.length) {
        const lastTaskId = data.conversation.task_ids[data.conversation.task_ids.length - 1];
        try {
          const fallback = await this.fetchFn(`/task/${lastTaskId}/conversation`);
          if (fallback.ok) {
            const taskConv = await fallback.json();
            if (taskConv.messages?.length > 0) {
              msgs = this._buildMessagesFromOpenAI(taskConv.messages);
            }
          }
        } catch { /* fallback failed, show what we have */ }
      }

      this.messages = msgs;
      this.scrollToBottom();
    } catch {
      // Silently fail -- will start fresh
    }
  }

  /** Convert conversation messages (BRC-60 format) to ChatMessage[]. */
  private _buildMessagesFromConversation(messages: any[]): ChatMessage[] {
    const toolResults = new Map<string, { content: string; name: string }>();
    for (const m of messages) {
      if (m.role === 'tool' && m.tool_call_id) {
        toolResults.set(m.tool_call_id, { content: m.content ?? '', name: m.name ?? '' });
      }
    }
    const msgs: ChatMessage[] = [];
    for (const m of messages) {
      if (m.role === 'user') {
        // Reconstruct attachment display from persisted metadata + task file URLs
        let attachments: Attachment[] | undefined;
        if (m.attachments?.length && m.task_id) {
          attachments = m.attachments.map((a: any) => ({
            url: `/files/${m.task_id}/${a.filename}`,
            mime_type: a.mime_type ?? 'image/png',
            filename: a.filename ?? 'attachment',
          }));
        }
        msgs.push({ role: 'user', content: m.content ?? '', timestamp: (m.ts ?? 0) * 1000, attachments });
      } else if (m.role === 'assistant') {
        const toolCalls: ToolCallInfo[] | undefined = m.tool_calls?.map((tc: any) => {
          const callId = tc.id ?? '';
          const result = toolResults.get(callId);
          return {
            callId,
            name: tc.function?.name ?? tc.name ?? '',
            arguments: tc.function?.arguments ?? '',
            output: result?.content ?? '',
            pending: false,
            success: true,
          };
        });
        if (m.content || toolCalls) {
          msgs.push({ role: 'assistant', content: m.content ?? '', toolCalls, timestamp: (m.ts ?? 0) * 1000 });
        }
      } else if (m.role === 'system' && m.content) {
        msgs.push({
          role: 'system',
          content: linkifyTxids(m.content),
          timestamp: (m.ts ?? 0) * 1000,
        });
      }
    }
    return msgs;
  }

  /** Convert OpenAI-format messages (from /task/{id}/conversation) to ChatMessage[]. */
  private _buildMessagesFromOpenAI(messages: any[]): ChatMessage[] {
    const msgs: ChatMessage[] = [];
    for (const m of messages) {
      if (m.role === 'user') {
        msgs.push({ role: 'user', content: m.content ?? '' });
      } else if (m.role === 'assistant') {
        const toolCalls: ToolCallInfo[] | undefined = m.tool_calls?.map((tc: any) => ({
          callId: tc.id ?? '',
          name: tc.function?.name ?? tc.name ?? '',
          arguments: tc.function?.arguments ?? JSON.stringify(tc.arguments ?? ''),
          output: '',
          pending: false,
          success: true,
        }));
        if (m.content || toolCalls) {
          msgs.push({ role: 'assistant', content: m.content ?? '', toolCalls });
        }
      } else if (m.role === 'system' && m.content) {
        msgs.push({ role: 'system', content: linkifyTxids(m.content) });
      }
    }
    return msgs;
  }

  private newConversation() {
    // Don't cancel the running task — let it finish in the background.
    // Just detach from it so the UI can start a fresh conversation.
    this.stopPollLoop();
    this.busy = false;
    this.thinking = false;
    this.currentTaskId = null;
    this.currentToolCalls = [];
    this.poller = null;
    this.sessionId = null;
    this.conversationTitle = null;
    this.messages = [];
    this.loadedSessionId = null;
    this.sessionSats = 0;
    this.sessionTokens = 0;
    this.iteration = 0;
    clearSessionId();
    window.location.hash = '';
  }

  render() {
    return html`
      <dm-chat-header
        .sessionId=${this.sessionId}
        .conversationTitle=${this.conversationTitle}
        .taskId=${this.currentTaskId}
        .fetchFn=${this.fetchFn}
        @new-conversation=${this.newConversation}
      ></dm-chat-header>

      <dm-budget-strip
        .fetchFn=${this.fetchFn}
        ?active=${this.busy}
        .iteration=${this.iteration}
        .taskSats=${this.sessionSats}
        .currencyMode=${this.currencyMode}
        .usdRate=${this.usdRate}
      ></dm-budget-strip>

      <div class="messages-wrapper">
        ${this.messages.length === 0 && !this.thinking && this.showOnboarding
          ? html`
              <dm-onboarding
                .fetchFn=${this.fetchFn}
                .currencyMode=${this.currencyMode}
                @send-prompt=${this.handleOnboardingPrompt}
                @dismiss=${this.handleOnboardingDismiss}
              ></dm-onboarding>
            `
          : html``}
        <div class="messages" style="${this.showOnboarding && this.messages.length === 0 && !this.thinking ? 'display:none' : ''}">
          ${this.messages.length === 0 && !this.thinking && !this.showOnboarding
            ? html`<div class="empty-state"><div class="title">Dolphin Milk</div><div>Send a message to start a conversation</div></div>`
            : ''}
          ${this.messages.map(
            (msg) => html`
              ${msg.toolCalls?.length ? html`
                <div class="tools-group">
                  ${msg.toolCalls.map(
                    (tc) => html`
                      <dm-tool-card .name=${tc.name} .callId=${tc.callId} .arguments=${tc.arguments}
                        .output=${tc.output ?? ''} .success=${tc.success ?? true} .pending=${tc.pending}
                        .satsCost=${tc.satsCost ?? 0} .durationMs=${tc.durationMs ?? 0}
                        .currencyMode=${this.currencyMode} .usdRate=${this.usdRate}></dm-tool-card>
                    `
                  )}
                </div>
              ` : ''}
              ${msg.content || msg.attachments?.length
                ? html`<dm-message .role=${msg.role} .content=${msg.content} .timestamp=${msg.timestamp ?? 0} ?pending=${msg.pending}
                    .satsCost=${msg.satsCost ?? 0} .currencyMode=${this.currencyMode} .usdRate=${this.usdRate}
                    .attachments=${msg.attachments ?? []}></dm-message>`
                : ''}
            `
          )}
          ${this.thinking
            ? html`
                <div class="thinking">
                  <div class="thinking-dots"><span class="dot"></span><span class="dot"></span><span class="dot"></span></div>
                  <span class="thinking-label">Thinking</span>
                </div>
              `
            : ''}
          ${this.currentToolCalls.length > 0
            ? html`
                <div class="tools-group">
                  ${this.currentToolCalls.map(
                    (tc) => html`<dm-tool-card .name=${tc.name} .callId=${tc.callId} .arguments=${tc.arguments}
                      .output=${tc.output ?? ''} .success=${tc.success ?? true} .pending=${tc.pending}
                      .satsCost=${tc.satsCost ?? 0} .durationMs=${tc.durationMs ?? 0}
                      .currencyMode=${this.currencyMode} .usdRate=${this.usdRate}></dm-tool-card>`
                  )}
                </div>
              `
            : ''}
          ${this.completionCard ? html`
            <div class="completion-card">
              <span class="completion-check">\u2713</span>
              <span>Complete \u00B7 ${this.completionCard.iterations} iteration${this.completionCard.iterations !== 1 ? 's' : ''} \u00B7 ${formatInlineCurrency(this.completionCard.sats, this.usdRate, this.currencyMode)}</span>
              <a class="completion-link" href="#conversation/${this.completionCard.conversationId}">View details \u2192</a>
            </div>
          ` : ''}
        </div>
        ${this.userScrolledUp
          ? html`<button class="scroll-bottom-btn" @click=${this.handleScrollToBottom}>New activity below</button>`
          : ''}
      </div>

      ${this.snackbar
        ? html`<dm-snackbar .type=${this.snackbar.type} .message=${this.snackbar.message}
            @dismiss=${() => { this.snackbar = null; }}></dm-snackbar>`
        : ''}

      <dm-chat-input
        .busy=${this.busy}
        .currentTaskId=${this.currentTaskId}
        .sessionSats=${this.sessionSats}
        .sessionTokens=${this.sessionTokens}
        .currencyMode=${this.currencyMode}
        .usdRate=${this.usdRate}
        @send=${(e: CustomEvent) => this.handleSend(e.detail.text, e.detail.attachments)}
        @cancel=${this.handleCancel}
      ></dm-chat-input>
    `;
  }

  // --------------------------------------------------------------------------
  // Send / Cancel
  // --------------------------------------------------------------------------

  private handleSend(text: string, attachments?: Attachment[]) {
    this.sendMessage(text, attachments);
  }

  private async handleCancel() {
    if (!this.currentTaskId) return;
    try {
      const res = await this.fetchFn(`/task/${this.currentTaskId}/cancel`, { method: 'POST' });
      if (res.ok) {
        this.stopPollLoop();
        this.thinking = false;
        this.currentToolCalls = [];
        this.busy = false;
        this.currentTaskId = null;
        this.poller = null;
        this.messages = [
          ...this.messages,
          { role: 'error' as const, content: 'Task cancelled.' },
        ];
        this.scheduleSync(true);
      }
    } catch {
      // Best effort
    }
  }

  // --------------------------------------------------------------------------
  // Send message — POST /chat, then poll transcript for results
  // --------------------------------------------------------------------------

  async sendMessage(text: string, attachments?: Attachment[]) {
    if (this.busy) {
      this.messageQueue.push(text);
      return;
    }

    // Check balance before sending — warn if wallet is empty
    try {
      // Use plain fetch (no auth) — /agent is publicly accessible
      const agentRes = await fetch('/agent');
      if (agentRes.ok) {
        const agentData = await agentRes.json();
        if (typeof agentData.balance === 'number' && agentData.balance === 0) {
          this.snackbar = {
            type: 'warning',
            message: 'Wallet is empty. Run "dolphin-milk receive" to get a funding address, then "dolphin-milk fund <TXID>" after sending BSV.',
          };
          return;
        }
      }
    } catch { /* best effort — proceed if check fails */ }

    // Dismiss onboarding on first message
    if (this.showOnboarding) {
      this.showOnboarding = false;
      markOnboardingSeen();
    }

    this.busy = true;
    this.completionCard = null;
    this.userScrolledUp = false;
    this.budgetWarningShown = false;
    const pendingMsg: ChatMessage = {
      role: 'user',
      content: text,
      timestamp: Date.now(),
      pending: true,
      attachments: attachments?.length ? attachments : undefined,
    };
    this.messages = [
      ...this.messages,
      pendingMsg,
    ];
    this.scrollToBottom();

    try {
      const { taskId, sessionId } = await this.submitViaRest(text, attachments);
      // Confirm the pending user message
      this.messages = this.messages.map(m =>
        m === pendingMsg ? { ...m, pending: false } : m
      );
      this.currentTaskId = taskId;
      this.thinking = true;
      if (sessionId) {
        this.sessionId = sessionId;
        this.loadedSessionId = sessionId;
        setSessionId(sessionId);
      }
      this.poller = new TranscriptPoller(this.fetchFn, taskId);
      await this.runPollLoop();
    } catch (e) {
      const errMsg = e instanceof Error ? e.message : 'Connection failed';
      // Mark the pending message as failed and add error message
      this.messages = [
        ...this.messages.map(m =>
          m.pending ? { ...m, pending: false } : m
        ),
        { role: 'error', content: errMsg },
      ];
      this.snackbar = { type: 'error', message: errMsg };
    }

    this.thinking = false;
    this.busy = false;
    this.currentTaskId = null;
    this.currentToolCalls = [];
    this.poller = null;
    if (this.sessionId && !this.conversationTitle) {
      const firstUser = this.messages.find(m => m.role === 'user');
      if (firstUser) {
        this.conversationTitle = firstUser.content.length > 60
          ? firstUser.content.slice(0, 60).trimEnd()
          : firstUser.content;
      }
    }
    this.flushQueue();
  }

  private async submitViaRest(text: string, attachments?: Attachment[]): Promise<{ taskId: string; sessionId: string }> {
    const body: Record<string, any> = {
      message: text,
      client_command_id: this.generateClientCommandId(),
    };
    if (this.sessionId) body.session_id = this.sessionId;
    if (attachments?.length) {
      // Only send data/mime_type/filename to the API (not url, which is for display only)
      body.attachments = attachments
        .filter(a => a.data)
        .map(a => ({ data: a.data, mime_type: a.mime_type, filename: a.filename }));
    }

    // Read selected model from the input component
    const inputEl = this.renderRoot.querySelector('dm-chat-input') as any;
    const model = inputEl?.selectedModel;
    if (model) body.model = model;

    const res = await this.fetchFn('/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`Chat submission failed: ${res.status}`);
    const data = await res.json();
    const taskId = data.task_id ?? '';
    const sessionId = data.session_id ?? '';
    if (!taskId) throw new Error('Failed to get task ID from server');
    return { taskId, sessionId };
  }

  private generateClientCommandId(): string {
    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
    return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }

  // --------------------------------------------------------------------------
  // Poll loop
  // --------------------------------------------------------------------------

  private runPollLoop(): Promise<void> {
    return new Promise<void>((resolve) => {
      this.preTaskMessageCount = Math.max(0, this.messages.length - 1);

      const doPoll = async () => {
        if (!this.poller) { resolve(); return; }
        try {
          const result = await this.poller.poll();
          if (result.messages.length > 0) {
            const formatted = result.messages.map(m =>
              m.role === 'error' ? { ...m, content: this._formatBudgetError(m.content) } : m
            );
            // Carry forward attachments from the optimistic user message to the
            // transcript-reconstructed user message (transcript has text only).
            const preTask = this.getPreTaskMessages();
            const lastPreTask = this.messages[this.preTaskMessageCount];
            if (lastPreTask?.role === 'user' && lastPreTask.attachments?.length) {
              const firstUser = formatted.find(m => m.role === 'user');
              if (firstUser && !firstUser.attachments?.length) {
                firstUser.attachments = lastPreTask.attachments;
              }
            }
            this.messages = [...preTask, ...formatted];
          }
          this.thinking = result.thinking;
          this.currentToolCalls = result.pendingToolCalls;
          this.sessionTokens = result.totalTokens;
          this.iteration = result.iteration;

          if (result.sessionId && !this.sessionId) {
            this.sessionId = result.sessionId;
            this.loadedSessionId = result.sessionId;
            setSessionId(result.sessionId);
          }

          if (result.budgetUpdate) {
            this.sessionSats = result.budgetUpdate.spent;
            this.dispatchEvent(new CustomEvent('budget', { detail: { balance: result.budgetUpdate.balance, spent: result.budgetUpdate.spent } }));
            if (result.budgetUpdate.balance < 50000 && !this.budgetWarningShown) {
              this.budgetWarningShown = true;
              this.snackbar = { type: 'warning', message: `Low balance: ${formatInlineCurrency(result.budgetUpdate.balance, this.usdRate, this.currencyMode)} remaining` };
            }
          }

          if (result.doneInfo) {
            if (result.doneInfo.sats_spent > 0) this.sessionSats = result.doneInfo.sats_spent;
            this.dispatchEvent(new CustomEvent('done', { detail: { iterations: result.doneInfo.iterations, sats_spent: result.doneInfo.sats_spent } }));
            // Show completion card with link to conversation detail
            const sid = this.sessionId || result.sessionId;
            if (sid) {
              this.completionCard = {
                iterations: result.doneInfo.iterations ?? 0,
                sats: result.doneInfo.sats_spent ?? 0,
                conversationId: sid,
              };
            }
          }

          this.scheduleSync();

          if (!result.active) { this.stopPollTimer(); resolve(); return; }
          this.pollTimer = window.setTimeout(doPoll, 500);
        } catch {
          this.pollTimer = window.setTimeout(doPoll, 1000);
        }
      };

      doPoll();
    });
  }

  private preTaskMessageCount = 0;

  private getPreTaskMessages(): ChatMessage[] {
    return this.messages.slice(0, this.preTaskMessageCount);
  }

  /** Reformat budget error messages to show currency-aware amounts. */
  private _formatBudgetError(msg: string): string {
    // Match "spent X of Y sats limit" pattern from check_limit()
    const m = msg.match(/spent (\d+) of (\d+) sats limit/);
    if (!m) return msg;
    const spent = parseInt(m[1], 10);
    const limit = parseInt(m[2], 10);
    const fmtSpent = formatInlineCurrency(spent, this.usdRate, this.currencyMode);
    const fmtLimit = formatInlineCurrency(limit, this.usdRate, this.currencyMode);
    return msg.replace(`spent ${m[1]} of ${m[2]} sats limit`, `spent ${fmtSpent} of ${fmtLimit} limit`);
  }

  private stopPollTimer() {
    if (this.pollTimer !== null) { clearTimeout(this.pollTimer); this.pollTimer = null; }
  }

  private stopPollLoop() {
    this.stopPollTimer();
    this.poller = null;
  }

  // --------------------------------------------------------------------------
  // Active task detection (page refresh recovery)
  // --------------------------------------------------------------------------

  private async detectActiveTask() {
    try {
      const res = await this.fetchFn('/status');
      if (!res.ok) return;
      const data = await res.json();
      const tasks = (data.tasks ?? []).filter((t: any) => t.status === 'running');
      if (tasks.length === 0) return;

      let running = tasks[0];
      if (this.sessionId) {
        const candidates = await Promise.all(tasks.map(async (t: any) => {
          try {
            const eventsRes = await this.fetchFn(`/task/${t.id}/events?since=0`);
            if (!eventsRes.ok) return null;
            const eventsData = await eventsRes.json();
            return eventsData.session_id === this.sessionId ? t : null;
          } catch { return null; }
        }));
        const matched = candidates.find(Boolean);
        if (!matched) return;
        running = matched;
      }

      if (!running) return;

      this.busy = true;
      this.thinking = true;
      this.currentTaskId = running.id;
      this.poller = new TranscriptPoller(this.fetchFn, running.id);
      this.preTaskMessageCount = this.messages.length;

      this.runPollLoop().then(() => {
        this.thinking = false;
        this.busy = false;
        this.currentTaskId = null;
        this.currentToolCalls = [];
        this.poller = null;
        this.scheduleSync(true);
      });
    } catch {
      // Best effort
    }
  }

  // --------------------------------------------------------------------------
  // DOM scheduling
  // --------------------------------------------------------------------------

  private scheduleSync(force = false) {
    if (force) { this.flush(); return; }
    if (this.syncTimer !== null) return;
    this.syncTimer = window.setTimeout(() => { this.flush(); this.syncTimer = null; }, SYNC_THROTTLE_MS);
  }

  private flush() {
    if (this.syncTimer !== null) { clearTimeout(this.syncTimer); this.syncTimer = null; }
    this.requestUpdate();
    if (!this.userScrolledUp) this.scrollToBottom();
  }

  private scrollToBottom() {
    requestAnimationFrame(() => {
      if (this.messagesEl) this.messagesEl.scrollTop = this.messagesEl.scrollHeight;
    });
  }

  private handleScrollToBottom() {
    this.userScrolledUp = false;
    this.scrollToBottom();
  }

  private flushQueue() {
    if (this.busy || this.messageQueue.length === 0) return;
    const next = this.messageQueue.shift()!;
    this.sendMessage(next);
  }
}
