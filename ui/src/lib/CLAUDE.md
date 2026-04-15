# ui/src/lib
> Shared utilities, types, styles, and infrastructure for the Lit web component frontend.

## Overview

This directory contains all shared code imported by page and component files in `ui/src/`. It provides the canonical type definitions for API responses, reusable CSS modules for the dark theme, a TTL-based data cache with promise coalescing, transcript polling for live chat streaming, BRC-31 wallet authentication, hash-based routing, BSV/USD conversion, business-language labels, and formatting helpers. Every file is a single-purpose module — components import only what they need.

## Files

| File | Purpose |
|------|---------|
| `constants.ts` | Service name mappings, color palettes, proof type shapes |
| `data-cache.ts` | `DataCache` class — TTL fetch cache with promise coalescing + server restart detection |
| `icons.ts` | 20 SVG nav/brand icons as Lit `TemplateResult` exports (stroke-based, 24x24, currentColor) |
| `labels.ts` | Business-language translations for event types, proof types, budget gauges, and section help |
| `proof-utils.ts` | SHA-256 hashing + proof hash computation (Web Crypto API) |
| `router.ts` | Hash-based route parser (`Route` type with 19 routes, `parseRoute()`) |
| `shared-styles.ts` | 16 reusable Lit CSS modules + 5 render helpers |
| `shared-types.ts` | 33 canonical API response interfaces + helpers |
| `storage.ts` | localStorage helpers for model, session ID, currency display, onboarding, and date range |
| `transcript-poller.ts` | `TranscriptPoller` — incremental transcript-to-chat-state mapper with token tracking |
| `ui-types.ts` | UI-specific types: `Attachment`, `ChatMessage`, `ToolCallInfo`, transcript DTOs |
| `usd.ts` | BSV/USD rate fetching, satoshi conversion, and dual-currency formatting |
| `util.ts` | Formatting helpers (time, duration, sats, keys, tokens, chart dates, truncation) |
| `wallet-auth.ts` | BRC-31 AuthFetch setup via `@bsv/sdk` with wallet URL from `/health` |

## Key Types

### shared-types.ts — API Response Interfaces

The canonical type definitions for all server API responses. No duplicate definitions elsewhere. 33 interfaces organized by domain.

| Interface | API Endpoint / Purpose |
|-----------|-------------|
| `TaskSummary` | `GET /tasks`, `GET /task/{id}` |
| `AgentInfo` | `GET /agent` |
| `BudgetReport` | `GET /budget` |
| `OperationStats` | Per-operation stats within a service |
| `ServiceBreakdown` | Per-service total + operations map |
| `SpendingEntry` | Single spending log entry |
| `BudgetDetailResponse` | `GET /budget/detail` |
| `BudgetGaugeData` | Gauge bar data (label, spent, limit, remaining) |
| `SpendingExportRow` | CSV export row shape |
| `GlobalAuditEvent` | Cross-task audit events |
| `ReceiptDetail` | Single BEEF receipt |
| `ReceiptsResponse` | `GET /task/{id}/receipts` |
| `ProofsResponse` | `GET /task/{id}/proofs` (wrapper with proofs + checkpoints) |
| `ProofDetail` | Individual proof/checkpoint within `ProofsResponse` |
| `Schedule` | `GET /schedules` (includes `schedule_type`, `conversation_id`, `created_at`, `created_by`, `run_history`) |
| `ScheduleRun` | Individual run within a schedule's `run_history` |
| `MemoryListItem` / `MemoryListResponse` | `GET /memory` |
| `MemoryDetail` | `GET /memory/{id}` |
| `MemorySearchResult` | `GET /memory/search` |
| `CertificateFields` | Certificate field values (name, capabilities, budget limits, etc.) |
| `CertificateStatus` | `GET /certificates` |
| `ConversationSummary` | `GET /conversations` |
| `ConversationDetail` | `GET /conversations/{id}` (includes `summary: ConversationSummaryStats`) |
| `ConversationSummaryStats` | Aggregated stats: iterations, proofs, artifacts, duration, cost_by_service, models_used |
| `ConversationTaskDetail` | Per-task detail within a conversation summary |
| `ConversationMessage` | Messages within a conversation (BRC-60 hash chain linked) |
| `MessageVerification` | Per-message hash chain verification result |
| `ChainVerification` | `GET /conversations/{id}/verify` |
| `AuditSummary` | `GET /task/{id}/audit` summary stats |
| `AuditEvent` | Single audit event from `GET /task/{id}/audit` |
| `AuditConversationMessage` | Conversation message in audit context |
| `OnChainVerification` | On-chain hash verification result |

Additional types: `StatusClass` (`'running' | 'complete' | 'error' | 'pending'`), `VerifyStatus` (`'pending' | 'match' | 'mismatch'`).

Helper: `getStatusClass(status: string): StatusClass` — normalizes status strings (`"running"`, `"in_progress"`, `"complete"`, `"done"`, etc.) to CSS-friendly values.

### ui-types.ts — Chat Display Types

Separate from `shared-types.ts` because these are UI-only constructs, not API mirrors.

- `Attachment` — file attachment with optional `data` (base64, for new messages), optional `url` (for loaded history), `mime_type`, `filename`
- `ChatMessage` — display message with role (`user | assistant | error | system`), content, optional `ToolCallInfo[]`, optional `timestamp`, optional `pending` (true while awaiting server confirmation), optional `satsCost` (per-iteration LLM cost in sats), optional `attachments` (image attachments)
- `ToolCallInfo` — tool call state: `callId`, `name`, `arguments`, `output`, `success`, `pending`, optional `satsCost` (cost for paid x402 tool calls), optional `durationMs`
- `TranscriptEventDto` — single JSONL transcript event from `GET /task/{id}/events` with `index`, `ts`, `type`, `id`, `data`
- `TaskEventsResponse` — paginated response wrapper with `task_id`, `offset`, `total`, `events[]`, `active`, `session_id`

### router.ts — Route Types

```typescript
type Route = 'chat' | 'tasks' | 'audit' | 'audit-global' | 'proofs' | 'replay' | 'agent'
           | 'conversations' | 'conversation-detail' | 'dashboard' | 'certificates'
           | 'automations' | 'budget' | 'reports' | 'compliance'
           | 'services' | 'settings' | 'wallet' | 'artifacts' | 'demo';
```

`parseRoute(hash)` maps URL hash fragments to `{ route, params }`. Handles nested routes like `task/{id}`, `task/{id}/proofs`, `task/{id}/replay`, `conversation/{id}` (with optional tab sub-routes: `artifacts`, `audit`, `chain`), `chat/{sessionId}`, `demo/{pillar}`, and agent sub-routes (`agent/tools`, `agent/memory`, `agent/telemetry` each set a `tab` param; bare `agent` defaults to `tab: 'identity'`). Legacy aliases: `memory` and `knowledge` -> `agent` with `tab: 'memory'`, `schedules` and `knowledge/schedules` -> `automations`, `telemetry` -> `agent` with `tab: 'telemetry'`, `activity` -> `tasks`, `audit-trail` -> `audit-global`, `dashboard` -> `conversations`. Falls back to `'chat'`.

## Key Classes

### DataCache (data-cache.ts)

TTL-based fetch cache that prevents duplicate HTTP calls when multiple components request the same data simultaneously.

```typescript
const cache = new DataCache(fetchFn);
const tasks = await cache.fetchTasks(15000);    // 15s TTL
const agent = await cache.fetchAgent(30000);    // 30s TTL
cache.invalidate('tasks');                       // clear one key
cache.invalidate();                              // clear all
await cache.checkServerRestart();                // invalidate if server restarted
```

**Promise coalescing**: If two components call `fetchTasks()` within the TTL window, only one HTTP request fires. The second caller receives the same promise.

**Error handling contract** — all methods return graceful fallbacks on HTTP or network errors:
- `fetchTasks()`, `fetchConversations()`, `fetchSchedules()` return `[]`
- `fetchBudget()`, `fetchAgent()`, `fetchBudgetDetail()`, `fetchCertificates()` return `null`

**Server restart detection**: `checkServerRestart()` compares `/health` uptime against cache creation timestamp. If the server restarted since the cache was populated, all entries are invalidated automatically.

Call `setFetchFn(fn)` after re-authentication to update the underlying fetch function.

### TranscriptPoller (transcript-poller.ts)

Stateful incremental poller that converts JSONL transcript events into `ChatMessage[]` for the chat UI. The JSONL transcript is the source of truth — the poller fetches events via `GET /task/{id}/events?since={cursor}` and builds UI state from them.

```typescript
const poller = new TranscriptPoller(fetchFn, taskId);
const result = await poller.poll();
// result.messages — ChatMessage[]
// result.thinking — boolean (LLM currently generating)
// result.pendingToolCalls — ToolCallInfo[] (tools in progress)
// result.active — boolean (task still running)
// result.sessionId — string | undefined
// result.budgetUpdate — { balance, spent, remaining } | undefined
// result.doneInfo — { iterations, sats_spent, result } | undefined
// result.totalTokens — number (cumulative prompt + completion tokens)
// result.iteration — number (current iteration count)
```

**Event processing**: Handles 8 event types (`user`, `think_request`, `think_response`, `tool_call`, `tool_result`, `budget_check`, `error`, `session_end`). Silently ignores 8 others (`system`, `session_start`, `proof_created`, `receipt_stored`, `checkpoint_created`, `continuation_save`, `continuation_resume`, `loop_warning`).

**Token tracking**: Accumulates `prompt_tokens` + `completion_tokens` from each `think_response` event into `totalTokens`.

**Iteration tracking**: Increments `iteration` counter on each `think_request` event. Exposed in `PollResult` for UI status display.

**Cost tracking**: Extracts per-iteration LLM cost from `think_response` events (`sats_effective` preferred, fallback to `sats_paid`). Stored as `pendingIterationCost` and attached to flushed `ChatMessage` via `satsCost`. Tool costs are extracted from tool output JSON via `extractToolCost()` (parses `sats_effective`, `sats_paid`, or `cost_sats` from x402 response payloads) and attached to individual `ToolCallInfo.satsCost`.

**Tool call lifecycle**: Tool calls from `think_response` are registered as pending, resolved by subsequent `tool_result` events, then flushed into a `ChatMessage` when the next iteration starts or the task ends. When the task becomes inactive, any remaining pending tool calls are flushed immediately.

## Key Functions

### wallet-auth.ts

```typescript
async function authenticate(baseUrl?: string): Promise<AuthState>
```
Reads wallet URL from the server's `/health` endpoint (sourced from `dolphin-milk.toml`), defaults to `http://localhost:3322`. Connects via `HTTPWalletJSON` substrate, creates `WalletClient` + `AuthFetch` for BRC-31 signed requests. Returns an `AuthState` with `fetchFn` that wraps all requests with auth headers. Falls back to plain `window.fetch` with error message when no wallet is present.

`AuthState.fetchFn` resolves relative URLs to absolute before calling AuthFetch (required for BRC-31 signature computation).

### proof-utils.ts

```typescript
async function sha256(input: string): Promise<string>
async function computeProofHash(data: string, timestamp: string, prevHash?: string): Promise<string>
```
`sha256()` computes SHA-256 of a plain string, returning hex. `computeProofHash()` computes `SHA-256(prev_hash_bytes || data || timestamp)` using Web Crypto API. Matches the Rust `compute_proof_hash()` in `src/onchain/proofs.rs`. Used by the proof viewer to recompute and verify proof chain hashes client-side.

### usd.ts — Currency Formatting

```typescript
async function fetchBsvUsdRate(): Promise<number>    // 5-min cache, $20 fallback
function satsToUsd(sats: number, rate: number): number
function usdToSats(usd: number, rate: number): number              // inverse conversion
function formatUsd(sats: number, rate: number): string              // "$1.23", "< $0.01"
function formatDualCurrency(sats, rate, mode): { primary, secondary }
function formatInlineCurrency(sats, rate, mode): string             // compact inline
function formatInlineCurrencyAlt(sats, rate, mode): string          // tooltip alternate
```

Rate fetched from `GET /rates/bsv-usd` (server-side proxy to avoid CORS). `formatDualCurrency()` returns primary/secondary strings based on `CurrencyDisplay` mode (`'usd-first'` or `'sats-first'`). `formatInlineCurrency()` returns the preferred format for compact contexts; `formatInlineCurrencyAlt()` returns the alternate for use as tooltip text.

### util.ts — Formatting Helpers

| Function | Example Output |
|----------|---------------|
| `formatRelativeTime(unixTs)` | `"just now"`, `"2m ago"`, `"1h ago"`, `"3d ago"` |
| `formatDuration(secs)` | `"< 1s"`, `"12s"`, `"2m 30s"`, `"1h 15m"` |
| `formatUptime(secs)` | `"12m"`, `"2h 15m"`, `"3d 4h"` |
| `truncate(s, maxLen)` | `"Hello wo..."` |
| `parseTimestamp(isoString)` | Unix seconds (number) |
| `truncateKey(hexKey)` | `"02af1234ab...3b1e"` |
| `formatSats(sats)` | `"1,234,567"` |
| `formatChartDate(date, showYear?)` | `"Jan 5"`, `"Dec 31 '25"` |
| `formatTokens(n)` | `"450"`, `"45,230"`, `"1.2M"` |

### storage.ts — localStorage Persistence

```typescript
getModel(): string                           // default: "gpt-5-mini"
setModel(model: string)
getSessionId(): string | null
setSessionId(id: string)
clearSessionId()
getCurrencyDisplay(): CurrencyDisplay        // default: "usd-first"
setCurrencyDisplay(mode: CurrencyDisplay)
hasSeenOnboarding(): boolean                 // onboarding tracking
markOnboardingSeen()
getDateRange(): StoredDateRange              // default: { preset: '30d' }
setDateRange(range: StoredDateRange)
```

Types: `CurrencyDisplay = 'usd-first' | 'sats-first'`, `StoredDateRange = { preset: string; from?: string; to?: string }`.

Keys: `dm-model`, `dm-session-id`, `dm-currency`, `dm-onboarding-seen`, `dm-date-range`.

### labels.ts — Business-Language Translations

Human-readable labels and help text for developer-facing terms. Used across audit, dashboard, budget, and proof views.

| Export | Contents |
|--------|----------|
| `EVENT_LABELS` | 17 event type -> display name mappings (e.g., `think_response` -> `"AI Decision"`, `memory_stored` -> `"Memory Saved"`) |
| `EVENT_DESCRIPTIONS` | 9 event type -> explanatory tooltip strings |
| `PROOF_LABELS` | 6 proof type -> `{ label, description }` for proof viewer |
| `BUDGET_HELP` | 6 budget gauge -> explanation strings (This Task, Last Hour, Last 24 Hours, Last 7 Days, Last 30 Days, Lifetime) |
| `SECTION_HELP` | 5 dashboard section -> help text strings |

### constants.ts — Lookup Tables

- `SERVICE_NAMES` — 16 x402 service key -> display name mappings (llm, proofs, openai-chat, claude-chat, nano-banana-pro, banana, veo-3-1-fast, veo, whisper-large-v3-turbo, whisper, x-research, 1sat, nanostore, messagebox, kling, polymirror)
- `SERVICE_COLORS` — 8-color palette for per-service bar charts (cycled by index)
- `PROOF_TYPE_LABELS` — 7 proof type -> human-readable label mappings
- `TYPE_COLORS` — 6 proof type -> CSS color (uses CSS custom properties)
- `TYPE_SHAPES` — 6 proof type -> Unicode character for chain visualization
- `friendlyName(service)` — lookup with passthrough fallback

### icons.ts — SVG Navigation Icons

Stroke-based 24x24 SVG icons as Lit `TemplateResult` exports. Uses `currentColor` for theming. Used by `app.ts` sidebar navigation.

| Export | Icon | Used For |
|--------|------|----------|
| `iconChat` | Speech bubble | Chat nav |
| `iconDashboard` | Four-panel grid | Dashboard nav |
| `iconBudget` | Dollar sign | Budget nav |
| `iconReports` | Rising bar chart | Reports nav |
| `iconCompliance` | Shield with checkmark | Compliance nav |
| `iconSearch` | Magnifying glass | Audit Trail nav |
| `iconTasks` | Clipboard with lines | Tasks nav |
| `iconConversations` | Two speech bubbles | Conversations nav |
| `iconMemory` | Database cylinder | Memory nav |
| `iconSchedules` | Clock face | Schedules nav |
| `iconCertificates` | Medal with ribbons | Certificates nav |
| `iconAgent` | Terminal prompt | Agent nav |
| `iconBug` | Bug outline | Brand logo |
| `iconMenu` | Three horizontal lines | Mobile hamburger |
| `iconChevron` | Small chevron down | Nav group toggle |
| `iconSettings` | Gear | Settings nav |
| `iconBell` | Bell | Notifications |
| `iconShare` | Box with arrow | Share/export |
| `iconWallet` | Wallet with card slot | Wallet nav |
| `iconArtifacts` | Grid gallery | Artifacts nav |

## Shared Styles (shared-styles.ts)

16 reusable Lit CSS template modules + 5 render helper functions. Components compose them:

```typescript
static styles = [pageHost, centerState, badges, stateFeedback, css`/* local */`];
```

### CSS Modules

| Export | Purpose | Used By |
|--------|---------|---------|
| `pageHost` | Flex column with padding, responsive breakpoints | All pages |
| `centerState` | Loading/error/empty centering with `.center-state` | All pages |
| `pageTitle` | 24px bold heading `.page-title` | Most pages |
| `summaryGrid` | 6-card responsive grid (auto-fit, 140px min) | Dashboard, Agent |
| `statsBar` | Compact horizontal stats row | Task list, Audit |
| `badges` | Status pills (`.badge-running`, `-complete`, `-error`, `-pending`) | Tasks, Queue |
| `cardBase` | Elevated card with hover/focus states | Conversations, Tasks |
| `tabBar` | Horizontal tab filter | Knowledge, Audit |
| `gaugeBar` | Progress bar with label, percentage, detail row | Budget |
| `alertBanner` | Warning/critical/exceeded alert banners | Budget |
| `sectionTitle` | 16px sub-section heading | Budget, Agent |
| `buttonStyles` | Base, primary, danger, small button variants | Certificates, Chat |
| `searchInput` | Search bar with icon, styled input | Conversations, Memory |
| `statusBadges` | `.status-running/completed/failed/cancelled/paused` pills | Queue, Tasks |
| `skeleton` | Shimmer loading placeholders (text, card, chart, hero-grid variants) | Dashboard, Reports |
| `stateFeedback` | Error/empty/loading/smart-empty states + spinner + breadcrumb nav | All pages |

### Render Helpers

| Function | Purpose |
|----------|---------|
| `renderError(message, suggestion?)` | Error state with icon and optional hint |
| `renderEmpty(title, subtitle?)` | Empty state with title and optional subtitle |
| `renderSmartEmpty(options)` | Enhanced empty state with icon, description, action button, and suggested prompts |
| `renderLoading(text?)` | Loading spinner with text (default "Loading...") |
| `renderBreadcrumb(items)` | Breadcrumb trail with links and current item |

Render helpers require `stateFeedback` CSS module to be included in the component's styles. `renderSmartEmpty()` accepts `{ icon, title, description, action?: EmptyStateAction, prompts?: EmptyStatePrompt[] }` — designed for first-time empty pages with call-to-action buttons and suggested prompt links.

The `skeleton` module provides `.skeleton` (shimmer animation), `.skeleton-text`, `.skeleton-card`, `.skeleton-chart`, and `.skeleton-hero-grid` classes. Respects `prefers-reduced-motion`.

All styles use CSS custom properties (`--accent`, `--bg-elevated`, `--border`, `--text-dim`, etc.) defined in `ui/src/styles.css`. Responsive breakpoints: 639px (mobile), 1024px (tablet).

## Usage Patterns

**Typical page component imports:**
```typescript
import { pageHost, centerState, pageTitle, badges, stateFeedback, renderError, renderLoading } from '../lib/shared-styles.js';
import type { TaskSummary } from '../lib/shared-types.js';
import { formatRelativeTime, formatSats } from '../lib/util.js';
import { friendlyName } from '../lib/constants.js';
```

**Chat page setup:**
```typescript
import { TranscriptPoller } from '../lib/transcript-poller.js';
import { authenticate } from '../lib/wallet-auth.js';
import { DataCache } from '../lib/data-cache.js';
import { getModel, setModel, getSessionId, setSessionId } from '../lib/storage.js';
```

**Dual-currency display:**
```typescript
import { formatDualCurrency, formatInlineCurrency } from '../lib/usd.js';
import { getCurrencyDisplay } from '../lib/storage.js';
const mode = getCurrencyDisplay();
const { primary, secondary } = formatDualCurrency(sats, rate, mode);
```

## Related

- `../CLAUDE.md` — UI source overview, directory structure, component registry
- `../../CLAUDE.md` — Root project documentation with full architecture
- `../components/` — Shared UI components that consume these utilities
- `../pages/` — Page-level components that import from this directory
- `../styles.css` — CSS custom properties referenced by `shared-styles.ts`
