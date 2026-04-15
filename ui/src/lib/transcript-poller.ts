/**
 * Stateful transcript poller — incrementally converts transcript events
 * into ChatMessage[] for the UI. Encapsulates all transcript-to-UI-state mapping.
 *
 * The JSONL transcript is the source of truth. The poller fetches events
 * via GET /task/{id}/events?since={cursor} and builds UI state from them.
 */

import type {
  ChatMessage,
  ToolCallInfo,
  TaskEventsResponse,
  TranscriptEventDto,
} from './ui-types.js';

/** Result returned by each poll() call. */
export interface PollResult {
  messages: ChatMessage[];
  thinking: boolean;
  pendingToolCalls: ToolCallInfo[];
  active: boolean;
  cursor: number;
  sessionId?: string;
  budgetUpdate?: { balance: number; spent: number; remaining: number };
  doneInfo?: { iterations: number; sats_spent: number; result: string };
  totalTokens: number;
  iteration: number;
}

export class TranscriptPoller {
  private cursor = 0;
  private messages: ChatMessage[] = [];
  private thinking = false;
  private pendingToolCalls: ToolCallInfo[] = [];
  private pendingAssistantText = '';
  private pendingIterationCost = 0;
  private sessionId?: string;
  private lastBudgetUpdate?: { balance: number; spent: number; remaining: number };
  private lastDoneInfo?: { iterations: number; sats_spent: number; result: string };
  private active = true;
  private totalTokens = 0;
  private iteration = 0;

  constructor(
    private fetchFn: typeof fetch,
    private taskId: string,
  ) {}

  /** Fetch new events and return updated UI state. */
  async poll(): Promise<PollResult> {
    this.lastBudgetUpdate = undefined;
    this.lastDoneInfo = undefined;

    try {
      const res = await this.fetchFn(`/task/${this.taskId}/events?since=${this.cursor}`, {
        cache: 'no-store',
      });
      if (!res.ok) {
        if (res.status !== 404) {
          console.warn(`[TranscriptPoller] poll failed: ${res.status} for task ${this.taskId}`);
        }
        return this.currentResult();
      }
      const data: TaskEventsResponse = await res.json();

      if (data.session_id) {
        this.sessionId = data.session_id;
      }
      this.active = data.active;

      for (const evt of data.events) {
        this.processEvent(evt);
      }
      this.cursor = data.total;

      // If the task is no longer active, flush any pending tool calls
      // so they appear in the final message (handles cases where
      // session_end is not the last event).
      if (!this.active) {
        this.flushPending();
      }
    } catch (err) {
      console.warn('[TranscriptPoller] poll error:', err);
    }

    return this.currentResult();
  }

  private currentResult(): PollResult {
    return {
      messages: [...this.messages],
      thinking: this.thinking,
      pendingToolCalls: [...this.pendingToolCalls],
      active: this.active,
      cursor: this.cursor,
      sessionId: this.sessionId,
      budgetUpdate: this.lastBudgetUpdate,
      doneInfo: this.lastDoneInfo,
      totalTokens: this.totalTokens,
      iteration: this.iteration,
    };
  }

  private processEvent(evt: TranscriptEventDto): void {
    switch (evt.type) {
      case 'user': {
        const content = evt.data.content ?? '';
        this.messages.push({ role: 'user', content, timestamp: evt.ts });
        break;
      }

      case 'think_request': {
        // New iteration starting — flush any pending tool calls from
        // the previous iteration into a message
        this.flushPending();
        this.thinking = true;
        this.iteration++;
        break;
      }

      case 'think_response': {
        this.thinking = false;
        const text = evt.data.content ?? '';
        const toolCalls = evt.data.tool_calls as any[] | undefined;

        // Accumulate token usage
        const promptTokens = typeof evt.data.prompt_tokens === 'number' ? evt.data.prompt_tokens : 0;
        const completionTokens = typeof evt.data.completion_tokens === 'number' ? evt.data.completion_tokens : 0;
        this.totalTokens += promptTokens + completionTokens;

        // Extract per-iteration LLM cost (sats_effective preferred, fallback to sats_paid)
        const iterCost = typeof evt.data.sats_effective === 'number' ? evt.data.sats_effective
          : typeof evt.data.sats_paid === 'number' ? evt.data.sats_paid : 0;
        this.pendingIterationCost = iterCost;

        if (toolCalls && toolCalls.length > 0) {
          // Register tool calls as pending — they'll be resolved by
          // subsequent tool_call / tool_result events
          this.pendingAssistantText = text;
          for (const tc of toolCalls) {
            const callId = tc.id ?? '';
            const fn = tc.function ?? {};
            const name = fn.name ?? 'unknown';
            const args = fn.arguments ?? '{}';
            // Only add if not already tracked
            if (!this.pendingToolCalls.find(p => p.callId === callId)) {
              this.pendingToolCalls.push({
                callId,
                name,
                arguments: args,
                pending: true,
              });
            }
          }
        } else if (text) {
          // Pure text response — push as assistant message with cost
          this.messages.push({ role: 'assistant', content: text, timestamp: evt.ts, satsCost: iterCost || undefined });
        }
        break;
      }

      case 'tool_call': {
        const callId = evt.data.call_id ?? '';
        const name = evt.data.name ?? 'unknown';
        const args = typeof evt.data.arguments === 'string'
          ? evt.data.arguments
          : JSON.stringify(evt.data.arguments ?? {});
        // Only add if not already tracked from think_response
        if (!this.pendingToolCalls.find(p => p.callId === callId)) {
          this.pendingToolCalls.push({
            callId,
            name,
            arguments: args,
            pending: true,
          });
        }
        break;
      }

      case 'tool_result': {
        const callId = evt.data.call_id ?? '';
        const output = evt.data.content ?? '';
        const success = evt.data.success ?? false;
        const existing = this.pendingToolCalls.find(p => p.callId === callId);
        if (existing) {
          existing.output = output;
          existing.success = success;
          existing.pending = false;
          // Extract cost from paid tool output (x402 calls include sats_paid in JSON)
          existing.satsCost = TranscriptPoller.extractToolCost(output);
        }
        break;
      }

      case 'budget_check': {
        const balance = (evt.data?.balance ?? evt.data?.cached_balance ?? 0) as number;
        const spent = (evt.data?.task_sats ?? evt.data?.sats_spent ?? evt.data?.spent_session ?? evt.data?.spent ?? 0) as number;
        const taskLimit = (evt.data?.task_limit ?? 0) as number;
        this.lastBudgetUpdate = {
          balance,
          spent,
          remaining: taskLimit > 0 ? Math.max(taskLimit - spent, 0) : Math.max(balance - spent, 0),
        };
        break;
      }

      case 'error': {
        this.flushPending();
        const message = evt.data.error ?? 'Unknown error';
        this.messages.push({ role: 'error', content: message, timestamp: evt.ts });
        break;
      }

      case 'session_end': {
        this.flushPending();
        this.active = false;
        const iterations = evt.data.iterations ?? 0;
        const sats_spent = evt.data.sats_spent ?? 0;
        const result = evt.data.result ?? '';
        this.lastDoneInfo = { iterations, sats_spent, result };
        break;
      }

      // Events we don't need in the chat UI
      case 'system':
      case 'session_start':
      case 'proof_created':
      case 'receipt_stored':
      case 'checkpoint_created':
      case 'continuation_save':
      case 'continuation_resume':
      case 'loop_warning':
        break;
    }
  }

  /**
   * Flush accumulated assistant text + resolved tool calls into a
   * ChatMessage. Called before a new iteration starts or on done/error.
   */
  private flushPending(): void {
    if (this.pendingToolCalls.length === 0 && !this.pendingAssistantText) {
      this.pendingIterationCost = 0;
      return;
    }

    const toolCalls = this.pendingToolCalls.length > 0
      ? [...this.pendingToolCalls]
      : undefined;

    if (this.pendingAssistantText || toolCalls) {
      this.messages.push({
        role: 'assistant',
        content: this.pendingAssistantText,
        toolCalls,
        satsCost: this.pendingIterationCost || undefined,
      });
    }

    this.pendingToolCalls = [];
    this.pendingAssistantText = '';
    this.pendingIterationCost = 0;
  }

  /** Try to extract sats cost from a tool output string (x402 calls return JSON with sats_paid). */
  private static extractToolCost(output: string): number | undefined {
    if (!output) return undefined;
    try {
      const parsed = JSON.parse(output);
      const cost = parsed.sats_effective ?? parsed.sats_paid ?? parsed.cost_sats;
      return typeof cost === 'number' && cost > 0 ? cost : undefined;
    } catch {
      return undefined;
    }
  }
}
