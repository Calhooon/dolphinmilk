# audit/
> Per-task and cross-task audit timeline views for inspecting agent activity, spending, and on-chain proofs.

## Overview

This module provides three audit views: a per-task detailed audit (`dm-audit` + `dm-audit-timeline` + `dm-audit-artifacts`), and a cross-task global audit feed (`dm-audit-global`). Both render JSONL transcript events from the backend, grouped and filtered by type. The per-task view groups events into collapsible iterations with cost breakdowns and supports CSV/JSON audit export plus a share menu, while the global view aggregates events from the 30 most recent tasks into a unified, filterable timeline. All sats values support dual-currency display (sats/USD) via `currencyMode`.

## Files

| File | Purpose |
|------|---------|
| `audit.ts` | Per-task audit page — fetches from 4 endpoints, renders summary cards with context-aware breadcrumb nav, export + share buttons, and delegates to four sub-views (timeline, conversation, chain, artifacts) |
| `audit-timeline.ts` | Timeline renderer — groups events into `IterationGroup`s, filters by category, renders collapsed/expanded event details with dual-currency amounts and budget breakdowns |
| `audit-artifacts.ts` | Artifacts tab — fetches file artifacts from `GET /task/{id}/artifacts`, renders cards with thumbnails (images) or file-type icons, download/open links |
| `audit-global.ts` | Cross-task audit feed — aggregates events from all recent tasks with chip filters, date range picker, and paginated event cards |

## Key Exports

### `audit-timeline.ts`

```typescript
export interface AuditEvent {
  timestamp: number;
  event_type: string;
  data: Record<string, unknown>;
}

export interface IterationGroup {
  number: number;
  events: AuditEvent[];
  model: string;
  sats: number;
  duration: number;
  toolCount: number;
  proofCount: number;
  receipt?: ReceiptDetail;
}
```

`AuditEvent` is the core event type consumed by `audit.ts` (imported from `audit-timeline.ts`). `audit-global.ts` uses `GlobalAuditEvent` and `AuditEvent` from `shared-types.ts`. `IterationGroup` is built internally by `buildIterations()` — events are grouped by `think_request` boundaries.

### Components

| Element | File | Route |
|---------|------|-------|
| `<dm-audit>` | `audit.ts` | `#task/{id}` |
| `<dm-audit-timeline>` | `audit-timeline.ts` | (child of `dm-audit`) |
| `<dm-audit-artifacts>` | `audit-artifacts.ts` | (child of `dm-audit`) |
| `<dm-audit-global>` | `audit-global.ts` | `#audit-trail` |

## Architecture

### `<dm-audit>` — Per-Task Audit Page

**Properties:**
- `taskId: string` — task ID from route
- `fetchFn: typeof fetch` — injectable fetch for testing
- `currencyMode: CurrencyDisplay` — sats/usd/dual display mode (from `getCurrencyDisplay()`)

**Internal state:**
- `usdRate: number` — BSV/USD exchange rate fetched on load
- `view: ViewMode` — current sub-view (`timeline`, `conversation`, `chain`, or `artifacts`)
- `conversation: AuditConversationMessage[]` — lazy-loaded conversation data
- `conversationLoading: boolean` — tracks in-progress conversation fetch
- `budgetExpanded: boolean` — synced with timeline child's budget panel
- `exporting: string | null` — tracks in-progress export (`'csv'`, `'json'`, or `null`)
- `exportError: string | null` — last export error message
- `shareOpen: boolean` — controls `<dm-share-menu>` visibility

**Lifecycle:** `firstUpdated()` triggers initial load. `updated()` re-triggers `loadAudit()` when `taskId` changes, resetting conversation state.

**Data loading** (`loadAudit()`): Uses `FetchController<AuditPageData>` to fetch 4 endpoints in parallel via `Promise.all`:
1. `/task/{id}/audit` — events + summary (`AuditSummary`) + `conversation_id`
2. `/task/{id}/receipts` — BEEF payment receipts keyed by iteration
3. `/budget/detail?task_id={id}` — per-service spending breakdown
4. `/task/{id}/proofs` — on-chain proofs (returns both `proofs: ProofDetail[]` for BRC-18 OP_RETURN proofs and `checkpoints: ProofDetail[]` for BRC-48 state tokens)

Also fetches BSV/USD rate via `fetchBsvUsdRate()` in parallel (non-blocking).

Extracts task description from the first `user` event's content, falling back to `session_start` event's task field. Extracts `conversationId` from the audit response for breadcrumb context.

**Breadcrumb navigation**: Context-aware via `renderBreadcrumb()`:
- With `conversationId`: Conversations > Conversation > Task #{shortId}
- Without `conversationId`: Activity > Task #{shortId}

**Four view modes** (toggled via `ViewMode` = `'timeline' | 'conversation' | 'chain' | 'artifacts'`):
- **Timeline** — delegates to `<dm-audit-timeline>` with events, receipts, budget data, `usdRate`, and `currencyMode`
- **Conversation** — lazy-loaded from `/task/{id}/conversation`, renders chat-style bubbles (user/assistant/system/tool roles) with tool call display
- **Chain** — merges BRC-18 proofs + BRC-48 checkpoints sorted by timestamp, delegates to `<dm-proof-chain>` with `currencyMode` and `usdRate`
- **Artifacts** — delegates to `<dm-audit-artifacts>` with `fetchFn`, `taskId`, `currencyMode`, and `usdRate`

The toolbar also includes a "Proofs" link (`#task/{id}/proofs`) that navigates to the dedicated proof viewer page.

**Summary cards** display: iterations, sats spent, duration, proof count (with chain integrity status), tool calls. The "Sats Spent" card is clickable and toggles the budget breakdown panel in the timeline child via `@query` ref.

**Audit export** (`exportCsv()`, `exportJson()`): Two export buttons in the toolbar fetch `/task/{id}/audit/export?format=csv|json`, create a Blob, and trigger a download via a temporary anchor element. File naming: `audit-{shortId}.csv` or `audit-{shortId}.json`. Buttons are disabled while an export is in progress. Errors display inline below the toolbar.

**Share menu**: A `<dm-share-menu>` component toggled by a Share button in the export group. Receives `taskId` and `fetchFn` props. Closes via `@close` event.

**Proof chain status** (`renderProofChainStatus()`): Checks `prev_hash` continuity across BRC-18 proofs only (not checkpoints). Counts breaks where a proof has a `prev_hash` that doesn't match the preceding proof's hash. Displays "chain intact" (green) or break count (red). Requires at least 2 proofs.

### `<dm-audit-timeline>` — Event Timeline

**Properties received from parent:**
- `events: AuditEvent[]`
- `receiptsByIteration: Map<number, ReceiptDetail>`
- `budgetDetail: BudgetDetailResponse | null`
- `taskId: string`
- `usdRate: number` — BSV/USD rate for dual-currency display
- `currencyMode: CurrencyDisplay` — controls sats vs USD vs dual display

**Internal state:**
- `expandedEvents: Set<string>` — tracks which events are expanded by click
- `collapsedIterations: Set<number>` — tracks which iterations are collapsed
- `allCollapsed: boolean` — starts `true`, all iterations auto-collapsed on first render; set to `false` once user interacts
- `filter: FilterCategory` — current event type filter
- `budgetExpanded: boolean` — budget breakdown panel visibility

**Event grouping** (`buildIterations()`): Splits events into three buckets:
- `preEvents` — events before the first `think_request`
- `iterations` — grouped by `think_request` boundaries, each accumulating sats (from `think_response.sats_effective` + `tool_result.sats_paid`), duration (from `think_response.duration_ms`), tool/proof counts
- `postEvents` — events after `session_end` (final proofs, checkpoints, receipts)

Each iteration header shows: model, sats (with currency tooltip via `formatInlineCurrency`/`formatInlineCurrencyAlt`), refund amount (if `receipt.sats_refunded > 0`), token count, duration, tool count, proof count, collapsed event count. Iterations are collapsible. Expand All / Collapse All buttons in the filter bar.

**Filter categories** (`FilterCategory`): `all`, `think`, `tools`, `proofs`, `budget`, `system`. Each maps to specific `event_type` values via `FILTER_MAP`. The `system` filter includes: `system`, `user`, `session_start`, `session_end`, `error`, `loop_warning`, `continuation_save`, `continuation_resume`, `memory_stored`.

**Event rendering**: Two-level display per event:
- **Collapsed**: timestamp + colored badge (label from `EVENT_LABELS` in `labels.js`, style from `EVENT_STYLES` with icon/color per type) + one-line summary (model, tool name, proof type, balance, etc.) with inline sats using `formatInlineCurrency`
- **Expanded**: full detail panel with key-value rows (`row()`/`satsRow()` helpers) and content blocks (click to toggle)

**17 event types** handled with per-type collapsed and expanded renderers: `think_request`, `think_response`, `tool_call`, `tool_result`, `proof_created`, `checkpoint_created`, `receipt_stored`, `budget_check`, `error`, `user`, `system`, `session_start`, `session_end`, `continuation_save`, `continuation_resume`, `loop_warning`, `memory_stored`.

**Budget breakdown** (`renderBudgetBreakdown()`): Computes cost categories via `computeCostBreakdown()` which scans events for LLM sats (`think_response.sats_effective`), tool sats (`tool_result.sats_paid`), and proof sats (`proof_created`/`checkpoint_created` `sats_cost`, defaulting to 200), then overrides with `budgetDetail.by_service` data when available. Renders a proportional stacked bar of 4 cost categories (LLM, Tools, Proofs, State) with percentage labels, plus two tables — per-tool costs (sorted by sats desc) and per-service/operation costs from budget detail.

**Public methods:**
- `toggleBudget()` — called by parent `dm-audit` via `@query` ref to sync budget panel state
- `isBudgetExpanded` (getter) — read by parent for chevron direction

### `<dm-audit-artifacts>` — Task Artifacts

**Properties:**
- `fetchFn: typeof fetch` — injectable fetch for testing
- `taskId: string` — task ID from route

**Internal state:**
- `loading: boolean` — fetch in progress
- `error: string` — error message from failed fetch
- `artifacts: ArtifactItem[]` — fetched artifact list

**Types:**
- `ArtifactType` = `'image' | 'video' | 'audio' | 'document' | 'file'`
- `ArtifactItem` — `{ name, type, size_bytes, created_by, created_at, url }`

**Data loading** (`loadArtifacts()`): Fetches `GET /task/{taskId}/artifacts`. Triggered on `connectedCallback()` and when `taskId` changes via `updated()`.

**Rendering**: Shows artifact count header, then a vertical list of cards. Each card displays:
- **Image artifacts**: Lazy-loaded `<img>` thumbnail (64x64, cover fit) with error fallback to "Image unavailable" placeholder
- **Non-image artifacts**: File-type icon based on extension (20+ extensions mapped to emoji icons via `getFileIcon()`)
- **Info**: filename (monospace, truncated with ellipsis), size (formatted via `formatSize()`) + creating tool name
- **Actions**: "Open" link (new tab) and "Download" link

**Helpers**: `formatSize()` (B/KB/MB), `getFileExtension()`, `getFileIcon()` (extension-to-emoji map covering rs, ts, js, json, toml, yaml, md, html, css, py, sh, svg, png, jpg, gif, webp, txt, pdf, mp4, mp3, wav).

### `<dm-audit-global>` — Cross-Task Feed

**Properties:**
- `fetchFn: typeof fetch` — injectable fetch for testing
- `currencyMode: CurrencyDisplay` — sats/usd/dual display mode

**Internal state:**
- `activeFilter: FilterId` — current type chip filter (default `'all'`)
- `periodFilter: string` — vestigial period preset key (default `'7d'`, unused since date range component replaced dropdown)
- `dateRange: DateRange` — date range from `<dm-date-range>` component (default 7 days via `defaultRange('7d')`)
- `visibleCount: number` — pagination cursor (default 100)
- `expandedIds: Set<string>` — expanded event card IDs (keyed by `task_id-id-timestamp`)

**Data loading** (`loadAudit()`): Uses `FetchController<AuditGlobalData>`. Triggered via `connectedCallback()`. Two-stage fetch:
1. `GET /tasks` — sorted by `started_at`, limited to 30 most recent
2. Per-task `GET /task/{id}/audit` — all fetched in parallel (errors silently return empty events)

Events are flattened into `GlobalAuditEvent[]` (annotated with `task_id`/`task_text`/`task_status`/`id`), sorted by timestamp descending. Fetches BSV/USD rate in parallel via `fetchBsvUsdRate()`.

**Filters:**
- **Type chips** (`FilterId`): `all`, `paid`, `proofs`, `tools`, `decisions`, `system` — each with count badge (via `countForFilter()`) and color from `FILTER_COLORS`
- **Date range**: `<dm-date-range>` component providing start/end date picker. Default range is 7 days. Emits `range-change` custom event, resets `visibleCount` on change.

**Filtering** (`filteredEvents` getter): Applies date range filter (Unix timestamp comparison) then type filter. Both filters compose.

**Summary bar**: total events, paid events, total sats (with dual-currency via `formatInlineCurrency`/`formatInlineCurrencyAlt`), unique task count.

**Empty state**: Context-aware messages — distinguishes "no events match this filter" from "no events found" in the selected time period.

**Pagination**: Shows 100 events at a time with "Load More" button showing remaining count.

**Event cards**: Each card shows icon (from `EVENT_ICONS`), timestamp, type badge (from `EVENT_LABELS` via `labels.js`, color-coded by type), linked task ID (first 8 chars), detail summary (`extractDetail()`: model for think_response, tool name for tool_call/tool_result, proof_type for proof_created, truncated error message), and sats cost (`extractSats()`: checks `sats_paid`, `sats` and `sats_cost` fields) with currency tooltip. Expandable to show task text, proof hash + WhatsOnChain link for proof/checkpoint events, and raw JSON data.

## API Endpoints Used

| Endpoint | Used By | Purpose |
|----------|---------|---------|
| `GET /task/{id}/audit` | `audit.ts`, `audit-global.ts` | Event list + summary + conversation_id |
| `GET /task/{id}/receipts` | `audit.ts` | BEEF payment receipts |
| `GET /budget/detail?task_id={id}` | `audit.ts` | Per-service spending |
| `GET /task/{id}/proofs` | `audit.ts` | On-chain proofs + checkpoints |
| `GET /task/{id}/conversation` | `audit.ts` | Reconstructed conversation (lazy) |
| `GET /task/{id}/audit/export?format=csv` | `audit.ts` | CSV audit export |
| `GET /task/{id}/audit/export?format=json` | `audit.ts` | JSON audit export |
| `GET /task/{id}/artifacts` | `audit-artifacts.ts` | File artifacts for task |
| `GET /tasks` | `audit-global.ts` | Task list for aggregation |
| `GET /rates/bsv-usd` | `audit.ts`, `audit-global.ts` (via `fetchBsvUsdRate`) | USD conversion |

## Dependencies

- `../../lib/util.js` — `formatDuration` (audit.ts, audit-timeline.ts), `truncate` (audit-timeline.ts, audit-global.ts), `formatSats` (audit-timeline.ts, audit-global.ts), `parseTimestamp` (audit-global.ts)
- `../../lib/usd.js` — `fetchBsvUsdRate`, `formatInlineCurrency`, `formatInlineCurrencyAlt` (audit.ts); `formatInlineCurrency`, `formatInlineCurrencyAlt` (audit-timeline.ts); `fetchBsvUsdRate`, `formatInlineCurrency`, `formatInlineCurrencyAlt`, `formatUsd`, `formatDualCurrency` (audit-global.ts)
- `../../lib/storage.js` — `getCurrencyDisplay`, `CurrencyDisplay` type (audit.ts, audit-timeline.ts, audit-global.ts)
- `../../lib/labels.js` — `EVENT_LABELS`, `EVENT_DESCRIPTIONS` (audit-timeline.ts); `EVENT_LABELS` as `BUSINESS_LABELS` (audit-global.ts)
- `../../lib/shared-types.js` — `ReceiptDetail`, `BudgetDetailResponse`, `ProofDetail`, `ProofsResponse`, `AuditSummary`, `AuditConversationMessage` (audit.ts); `ReceiptDetail`, `BudgetDetailResponse` (audit-timeline.ts); `TaskSummary`, `GlobalAuditEvent`, `AuditEvent` (audit-global.ts)
- `../../lib/shared-styles.js` — `pageHost`, `centerState`, `pageTitle`, `stateFeedback`, `renderLoading`, `renderBreadcrumb`, `renderError` (audit.ts); `centerState` (audit-timeline.ts); `centerState`, `stateFeedback`, `renderLoading`, `renderEmpty`, `renderError` (audit-artifacts.ts); `pageHost`, `centerState`, `pageTitle`, `sectionTitle`, `stateFeedback`, `renderLoading`, `renderError` (audit-global.ts)
- `../../controllers/fetch-controller.js` — `FetchController` (audit.ts, audit-global.ts)
- `../../components/proof-chain.js` — `<dm-proof-chain>` (chain view in audit.ts)
- `../../components/share-menu.js` — `<dm-share-menu>` (share button in audit.ts)
- `../../components/date-range.js` — `<dm-date-range>`, `defaultRange`, `DateRange` type (audit-global.ts)

## Related

- `../../../CLAUDE.md` — root project documentation
- `../../lib/CLAUDE.md` — shared library utilities and types
- `../../components/CLAUDE.md` — shared UI components (proof-chain, date-range, share-menu, tool-card)
- `../proofs/` — dedicated proof viewer page (`#task/{id}/proofs`)
- `../budget/` — budget panel with spending export
