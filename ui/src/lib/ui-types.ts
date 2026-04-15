/**
 * UI-specific types for chat and transcript polling.
 * Canonical domain interfaces live in shared-types.ts.
 */

/** A file attachment — either base64-encoded (new messages) or URL-based (loaded from history). */
export interface Attachment {
  /** Base64-encoded content (present for newly-sent messages). */
  data?: string;
  /** URL to fetch the image from (present for messages loaded from conversation history). */
  url?: string;
  mime_type: string;
  filename: string;
}

/** Display message in the chat UI. */
export interface ChatMessage {
  role: 'user' | 'assistant' | 'error' | 'system';
  content: string;
  toolCalls?: ToolCallInfo[];
  timestamp?: number;
  /** True while awaiting server confirmation of a user message. */
  pending?: boolean;
  /** Per-iteration LLM cost in satoshis (from think_response sats_effective). */
  satsCost?: number;
  /** File attachments (images) sent with the message. */
  attachments?: Attachment[];
}

export interface ToolCallInfo {
  callId: string;
  name: string;
  arguments: string;
  output?: string;
  success?: boolean;
  pending: boolean;
  /** Cost in sats for paid tool calls (x402). Parsed from tool output JSON. */
  satsCost?: number;
  /** Duration in ms (from think_response or tool execution). */
  durationMs?: number;
}

/** A single transcript event from GET /task/{id}/events. */
export interface TranscriptEventDto {
  index: number;
  ts: number;
  type: string;
  id: string;
  data: Record<string, any>;
}

/** Response from GET /task/{id}/events. */
export interface TaskEventsResponse {
  task_id: string;
  offset: number;
  total: number;
  events: TranscriptEventDto[];
  active: boolean;
  session_id?: string;
}
