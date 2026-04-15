# Chat Module
> Poll-based streaming chat UI with model picker, transcript-driven message rendering, currency-aware display, session persistence, file attachments, and onboarding.

## Overview

The chat module is the primary user-facing interaction surface. It composes three Lit components — a header bar (with share menu), a message area with thinking/tool indicators, and an input row with model selection and file attachments — plus an embedded `<dm-budget-strip>` showing iteration progress and budget usage during active tasks. Messages are not pushed via WebSocket; instead, `TranscriptPoller` polls `GET /task/{id}/events` at 500ms intervals and converts JSONL transcript events into `ChatMessage[]` for rendering. Session and model selection are persisted to `localStorage` across page refreshes. All monetary values support dual-currency display (sats/USD) via a `currencyMode` prop from the app shell. First-time users see an onboarding overlay; after task completion, a completion card links to the conversation detail view.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `chat.ts` | 851 | `<dm-chat>` — main chat panel: message list, thinking indicator, tool cards, budget strip, scroll management, send/cancel flow, transcript polling, conversation history loading (with fallback), active task recovery, currency-aware budget errors, onboarding, completion card, file attachment forwarding, pre-send balance check |
| `chat-input.ts` | 675 | `<dm-chat-input>` — input row with text field, send/cancel buttons, model picker flyout, sats-spent pill (currency-aware), token counter pill, file attachment support (picker, drag-and-drop, paste). Exports `ModelDef` interface and `MODELS` array |
| `chat-header.ts` | 89 | `<dm-chat-header>` — conversation header bar with title, share menu, "All Chats" link, "New Chat" button |

## Key Exports

### `chat-input.ts`

```ts
export interface ModelDef {
  id: string;      // e.g. 'gpt-5-mini', 'claude-sonnet-4-6'
  label: string;   // Display name
  cost: string;    // Approximate cost string
  provider: string; // 'OpenAI' or 'Claude'
}

export const MODELS: ModelDef[]  // 9 models: 6 OpenAI + 3 Claude
```

### Custom Elements

| Element | Tag | Parent |
|---------|-----|--------|
| `WormChat` | `<dm-chat>` | `app.ts` (default route) |
| `WormChatInput` | `<dm-chat-input>` | `<dm-chat>` |
| `WormChatHeader` | `<dm-chat-header>` | `<dm-chat>` |

## Architecture

### Message Flow

1. User types in `<dm-chat-input>` → dispatches `send` custom event with `{ text, attachments? }`
2. `<dm-chat>` receives event → calls `sendMessage(text, attachments)`
3. `sendMessage()` checks wallet balance via `GET /agent` (warns if zero) → adds optimistic user message (with `pending: true`, attachments) → `POST /chat` with `{ message, session_id?, model?, client_command_id, attachments? }`
4. Server returns `{ task_id, session_id }` → creates `TranscriptPoller`
5. Poll loop runs at 500ms, processing transcript events into `ChatMessage[]`
6. When `result.active === false`, poll loop resolves, completion card is shown, and UI resets to idle

### Transcript Event Mapping

The `TranscriptPoller` (in `lib/transcript-poller.ts`) maps these transcript event types to UI state:

| Event Type | UI Effect |
|------------|-----------|
| `user` | Appends user message |
| `think_request` | Sets `thinking = true`, flushes pending tool calls |
| `think_response` | Sets `thinking = false`, registers tool calls or pushes text message |
| `tool_call` | Adds pending tool call (if not already tracked from `think_response`) |
| `tool_result` | Resolves pending tool call with output and success status |
| `budget_check` | Updates `sessionSats`, triggers low-balance snackbar (<50K sats) |
| `error` | Flushes pending, pushes error message (budget errors reformatted via `_formatBudgetError`) |
| `session_end` | Flushes pending, marks inactive, emits `done` event with iterations/sats |

Events ignored by chat UI: `system`, `session_start`, `proof_created`, `receipt_stored`, `checkpoint_created`, `continuation_save`, `continuation_resume`, `loop_warning`.

### State Management

| State | Location | Persistence |
|-------|----------|-------------|
| Selected model | `WormChatInput.selectedModel` | `localStorage` via `getModel()`/`setModel()` |
| Session ID | `WormChat.sessionId` | `localStorage` via `getSessionId()`/`setSessionId()` |
| Message history | `WormChat.messages` | Reloaded from `GET /conversations/{id}` on session restore |
| Active task | `WormChat.currentTaskId` | Recovered via `detectActiveTask()` on page load |
| Session sats | `WormChat.sessionSats` | Transient — updated from `budget_check` poll events |
| Session tokens | `WormChat.sessionTokens` | Transient — updated from `totalTokens` in poll results |
| Iteration | `WormChat.iteration` | Transient — updated from `result.iteration` in poll results, drives budget strip |
| Currency mode | `WormChat.currencyMode` | Inherited from app shell via property; default from `getCurrencyDisplay()` |
| USD rate | `WormChat.usdRate` | Transient — fetched once on connect via `fetchBsvUsdRate()` |
| Show onboarding | `WormChat.showOnboarding` | Checked once on connect; dismissed via `markOnboardingSeen()` in `localStorage` |
| Completion card | `WormChat.completionCard` | Transient — set when task completes with `{ iterations, sats, conversationId }` |
| Attachments | `WormChatInput.attachments` | Transient — cleared on send, base64-encoded images |

### Custom Events

| Event | Source | Detail | Consumed By |
|-------|--------|--------|-------------|
| `send` | `<dm-chat-input>` | `{ text: string, attachments?: Attachment[] }` | `<dm-chat>` |
| `cancel` | `<dm-chat-input>` | none | `<dm-chat>` |
| `model-change` | `<dm-chat-input>` | `{ model: string }` | `app.ts` (status bar) |
| `new-conversation` | `<dm-chat-header>` | none | `<dm-chat>` |
| `send-prompt` | `<dm-onboarding>` | `{ text: string }` | `<dm-chat>` |
| `dismiss` | `<dm-onboarding>` | none | `<dm-chat>` |
| `budget` | `<dm-chat>` | `{ balance, spent }` | parent |
| `done` | `<dm-chat>` | `{ iterations, sats_spent }` | parent |

## Key Behaviors

### Onboarding

On first connect, `checkOnboarding()` determines whether to show the `<dm-onboarding>` overlay. It is shown only when: the user hasn't seen it before (`hasSeenOnboarding()` returns false), there is no active session, no messages, and `GET /conversations` returns an empty list. The onboarding component dispatches `send-prompt` to start the first conversation, or `dismiss` to hide itself. Once a message is sent, `markOnboardingSeen()` persists the flag to `localStorage`.

### Budget Strip

`<dm-budget-strip>` is embedded between the header and message area. It receives `fetchFn`, `active` (bound to `busy`), `iteration`, `taskSats` (bound to `sessionSats`), `currencyMode`, and `usdRate`. The strip shows iteration progress and budget tier usage during active tasks, providing at-a-glance spending context without leaving the chat view.

### Completion Card

After a task finishes, if the session has an ID, a green completion card appears showing iteration count, total sats spent (currency-formatted), and a "View details" link to `#conversation/{id}`. The card is cleared when a new message is sent.

### Pre-Send Balance Check

Before sending a message, `sendMessage()` fetches `GET /agent` and checks `balance`. If the wallet reports zero sats, a warning snackbar appears with instructions to fund the wallet, and the message is not sent.

### File Attachments

`<dm-chat-input>` supports image attachments via three input methods:

| Method | Trigger |
|--------|---------|
| File picker | Paperclip button opens native file dialog |
| Drag and drop | Drop files onto the input wrapper |
| Clipboard paste | Paste images from clipboard |

**Constraints**: Max 5MB per file, max 10 files, accepted types: PNG, JPEG, GIF, WebP. Files are read as base64 via `FileReader` and stored as `Attachment[]` state. Thumbnails appear above the input row with individual remove buttons. Validation errors display above the attachment row.

Attachments are sent with the `send` event and included in the `POST /chat` body as `{ data, mime_type, filename }`. During conversation history loading, attachments are reconstructed from persisted metadata using task file URLs (`/files/{task_id}/{filename}`). When the transcript poll replaces the optimistic user message, attachments from the original message are carried forward to the transcript-reconstructed message.

### Share Menu

`<dm-chat-header>` includes a "Share" button that toggles a `<dm-share-menu>` dropdown. The share menu receives the current `taskId`, `sessionId`, and `fetchFn` for share/export actions.

### DOM Update Throttling

`scheduleSync()` batches DOM updates at 80ms (`SYNC_THROTTLE_MS`). This prevents excessive re-renders during rapid poll cycles when multiple transcript events arrive at once.

### Scroll Anchoring

- `userScrolledUp` tracks whether the user has scrolled away from the bottom (>150px threshold)
- Auto-scroll is suppressed when `userScrolledUp === true`
- A "New activity below" pill button appears for manual scroll-to-bottom

### Active Task Recovery

On `connectedCallback`, `detectActiveTask()` checks `GET /status` for running tasks. If a running task matches the persisted `sessionId`, it resumes polling — enabling seamless page-refresh recovery mid-conversation.

### Message Queue

If the user sends a message while `busy === true`, it's pushed to `messageQueue`. After the current task completes, `flushQueue()` sends the next queued message.

### Pre-Task Message Preservation

When polling begins, `preTaskMessageCount` snapshots the current message count (minus the optimistic user message). During polling, `getPreTaskMessages()` slices the original pre-task messages and the poller's new messages are appended after them. This prevents conversation history from being overwritten by poll results.

### Conversation History

When `sessionId` changes, `loadConversationHistory()` fetches `GET /conversations/{id}` and reconstructs `ChatMessage[]` from the stored messages (BRC-60 format) via `_buildMessagesFromConversation()`, including tool call results matched by `tool_call_id` and attachment reconstruction from persisted metadata. If no assistant messages are found but the conversation has task IDs, a fallback fetches `GET /task/{lastTaskId}/conversation` and reconstructs messages from OpenAI-format via `_buildMessagesFromOpenAI()`. This handles cases where transcript events ran but weren't assembled into conversation messages.

### Model Picker

The flyout menu groups 9 models by provider (OpenAI/Claude) with colored dots (green=#10a37f for OpenAI, amber=#d97706 for Claude). Current models:

| Provider | Models |
|----------|--------|
| OpenAI | GPT-5 Nano, GPT-5 Mini (default), GPT-5, GPT-5.2, o4 Mini, GPT-5.2 Pro |
| Claude | Haiku 4.5, Sonnet 4.6, Opus 4.6 |

Selection is persisted to `localStorage` and sent with each `POST /chat` request. Escape key closes the flyout.

### Token and Cost Display

The input row shows two inline pills during active sessions:
- **Tokens pill**: Cumulative token count via `formatTokens()` (e.g. "12.4k tok"), updated from `result.totalTokens` on each poll
- **Sats pill**: Cumulative cost via `formatInlineCurrency()` (lightning bolt icon), currency-aware — shows sats or USD depending on `currencyMode`, updated from `budget_check` events

Both pills are conditionally rendered only when their values are > 0, and reset on new conversation.

### Budget Error Formatting

`_formatBudgetError()` rewrites server budget error messages (e.g. "spent 50000 of 100000 sats limit") to use currency-aware formatting via `formatInlineCurrency()`, respecting the current `currencyMode` and `usdRate`.

### Low-Balance Warning

When a `budget_check` event reports balance below 50,000 sats, a warning snackbar appears with a currency-formatted remaining balance. The warning is shown at most once per task (`budgetWarningShown` flag, reset on each `sendMessage`).

### New Conversation

`newConversation()` detaches from any running task without cancelling it — the task continues in the background. Stops polling, clears all UI state (messages, session, title, sats, tokens, iteration, completion card), removes the session from localStorage, and resets the URL hash.

### Auto-Title

After a task completes, if no `conversationTitle` is set, the first user message is used as the title (truncated to 60 chars).

## API Endpoints Used

| Endpoint | Method | Used By |
|----------|--------|---------|
| `/chat` | POST | `submitViaRest()` — send message (with optional attachments) |
| `/task/{id}/events?since={cursor}` | GET | `TranscriptPoller.poll()` — stream events |
| `/task/{id}/cancel` | POST | `handleCancel()` — cancel active task |
| `/task/{id}/conversation` | GET | `loadConversationHistory()` — fallback when BRC-60 messages lack assistant content |
| `/conversations/{id}` | GET | `loadConversationHistory()` — restore session (primary) |
| `/conversations` | GET | `checkOnboarding()` — determine if first-time user |
| `/status` | GET | `detectActiveTask()` — page refresh recovery |
| `/agent` | GET | `sendMessage()` — pre-send balance check |
| `/files/{task_id}/{filename}` | GET | Attachment display URLs reconstructed during history loading |

## Dependencies

| Import | From |
|--------|------|
| `ChatMessage`, `ToolCallInfo`, `Attachment` | `../../lib/ui-types.js` |
| `ConversationDetail` | `../../lib/shared-types.js` |
| `TranscriptPoller` | `../../lib/transcript-poller.js` |
| `getSessionId`, `setSessionId`, `clearSessionId`, `getCurrencyDisplay`, `hasSeenOnboarding`, `markOnboardingSeen`, `CurrencyDisplay` | `../../lib/storage.js` |
| `getModel`, `setModel` | `../../lib/storage.js` |
| `fetchBsvUsdRate`, `formatInlineCurrency` | `../../lib/usd.js` |
| `formatTokens` | `../../lib/util.js` |
| `<dm-message>` | `../../components/message.js` |
| `<dm-tool-card>` | `../../components/tool-card.js` |
| `<dm-snackbar>` | `../../components/snackbar.js` |
| `<dm-onboarding>` | `../../components/onboarding.js` |
| `<dm-budget-strip>` | `../../components/budget-strip.js` |
| `<dm-share-menu>` | `../../components/share-menu.js` |

## Related

- [`ui/src/CLAUDE.md`](../../CLAUDE.md) — UI-wide conventions, shared styles, routing
- [`ui/src/lib/transcript-poller.ts`](../../lib/transcript-poller.ts) — Stateful transcript->ChatMessage converter
- [`ui/src/lib/storage.ts`](../../lib/storage.ts) — localStorage helpers for model, session, currency, and onboarding persistence
- [`ui/src/lib/ui-types.ts`](../../lib/ui-types.ts) — `ChatMessage`, `ToolCallInfo`, `Attachment`, `TranscriptEventDto`, `TaskEventsResponse`
- [`ui/src/lib/usd.ts`](../../lib/usd.ts) — BSV/USD conversion and dual-currency formatting
- [`ui/src/lib/util.ts`](../../lib/util.ts) — `formatTokens()` and other formatting helpers
- [`ui/src/components/tool-card.ts`](../../components/tool-card.ts) — Tool call result cards
- [`ui/src/components/message.ts`](../../components/message.ts) — Chat message bubbles
- [`ui/src/components/budget-strip.ts`](../../components/budget-strip.ts) — Compact iteration progress and budget strip
- [`ui/src/components/onboarding.ts`](../../components/onboarding.ts) — First-run onboarding overlay
- [`ui/src/components/share-menu.ts`](../../components/share-menu.ts) — Share/export menu for conversations
- [`ui/src/pages/conversations.ts`](../conversations.ts) — Conversation list view (linked from header "All Chats")
