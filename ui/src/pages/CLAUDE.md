# ui/src/pages
> Route-level Lit page components — one per hash route, rendering the main content area of the Dolphin Milk console.

## Overview

Each file exports a single Lit `LitElement` subclass registered as a custom element (`<dm-*>`). The root shell (`app.ts`) switches between these based on the URL hash. All page components accept a `fetchFn` property for BRC-31 authenticated fetching and follow a consistent pattern: fetch data from the axum REST API, manage loading/error states, render cards and stats. Subdirectories (`audit/`, `budget/`, `chat/`, `proofs/`) group multi-file features.

## Directory Structure

```
pages/
├── agent.ts              # Agent identity, tools, memory + telemetry tabs
├── artifacts.ts          # Artifact gallery — images grid, text preview, code highlight
├── automations.ts        # Recurring schedule list with run history
├── certificates.ts       # BRC-52 certificate management + revocation
├── compliance.ts         # Compliance dashboard — WORM mode, regulations, task table
├── conversation-detail.ts# Conversation detail — 4-tab view (messages, artifacts, audit, chain)
├── conversations.ts      # Conversation list with search, sort, cost filters
├── dashboard.ts          # Overview — balance hero, today stats, budget gauge, trend
├── demo-overlay.ts       # Investor demo with stat tickers + comparisons
├── memory.ts             # Memory browser with BM25 search + category filters
├── replay-view.ts        # Task replay timeline with cost graph + fork
├── reports.ts            # CFO accounting — spending trends, service breakdown, export
├── services.ts           # x402 service catalog with expandable detail panels
├── settings.ts           # Agent config display + client-side preferences
├── task-list.ts          # Paginated task history with origin filters
├── wallet.ts             # Wallet fuel gauge — balance, UTXO tiers, funding flow
├── audit/                # Per-task and global audit views
│   ├── audit.ts          # <dm-audit> — 3-view task inspector
│   ├── audit-artifacts.ts# <dm-audit-artifacts> — Per-task artifact list
│   ├── audit-global.ts   # <dm-audit-global> — Cross-task timeline
│   └── audit-timeline.ts # <dm-audit-timeline> — Iteration-grouped event list
├── budget/               # Budget gauges + export
│   ├── budget-panel.ts   # <dm-budget-panel> — 3-6 gauge bars + service breakdown
│   └── spending-export.ts# <dm-spending-export> — CSV download button
├── chat/                 # Chat interface
│   ├── chat.ts           # <dm-chat> — Transcript-driven streaming chat
│   ├── chat-header.ts    # <dm-chat-header> — Session title bar
│   └── chat-input.ts     # <dm-chat-input> — Input row + model picker
└── proofs/               # On-chain proof viewer
    └── proofs.ts         # <dm-proofs> — Proof cards, hash verify, receipts
```

## Files

| File | Element | Route | Description |
|------|---------|-------|-------------|
| `agent.ts` | `<dm-agent>` | `#agent` | Identity card (key with copy button, version, certificate badge with revoked state, uptime), 4-stat summary (balance, tasks, sats spent, tool count) with currency mode. 4-tab interface: **Identity** (hero + stats), **Tools** (registry grouped into 7 categories with color-coded pills), **Memory** (embeds `<dm-memory>`), **Telemetry** (embeds `<worm-telemetry>`). Fetches `/agent` and `/certificates` |
| `artifacts.ts` | `<dm-artifacts>` | `#artifacts` | Artifact gallery across all tasks. Type filter pills (all/image/video/audio/document/file) with counts. Image grid with lightbox preview (arrow key navigation, Escape to close). Text/code file preview with syntax highlighting (highlight.js — 13 languages) and markdown rendering. Group-by-conversation toggle. Stats bar (total, images, files). Broken image fallback. Fetches `GET /artifacts` |
| `automations.ts` | `<dm-automations>` | `#automations` | Schedule cards with enabled/disabled badge, type badge (Cron/One-shot/Interval), human-readable intervals, cron expression display, countdown to next run, last run relative time, run count. Expandable cards with run history (status badges: triggered/completed/failed). Stats bar (total automations, enabled count, total runs). 30s auto-polling. Graceful 404 handling for unimplemented endpoints |
| `certificates.ts` | `<dm-certificates>` | `#certificates` | Three-state cert display (parent-signed/self-signed/none) with revocation badge. Issue form with agent name, 7-capability grid (llm, tools, messaging, x402, wallet, memory, schedule) with select-all toggle and default caps (llm, tools, messaging, x402), collapsible budget limits (per-task/per-hour/per-day) with currency-aware sats/USD conversion. Relinquish and revoke actions (revoke requires confirmation dialog). Revocation outpoint display with WhatsOnChain link. Raw JSON toggle |
| `compliance.ts` | `<dm-compliance>` | `#compliance` | Compliance dashboard: WORM mode, compliance mode, retention policy, regulation pills. Summary stats (tasks/proofs/sats). Date-filtered task table with links. Fetches `GET /compliance/report` |
| `conversation-detail.ts` | `<dm-conversation-detail>` | `#conversation/{id}/detail` | 4-tab conversation inspector: **Messages** (chat-style message list with role badges, tool calls, cost per turn), **Artifacts** (per-conversation artifacts grouped by turn with type filters, fetches `/conversations/{id}/artifacts`), **Audit** (expandable per-task audit with "Show work" toggle, lazy-loads `/task/{id}/audit`), **Chain** (BRC-60 hash chain verification with per-message integrity display). Stats bar (messages, tasks, total cost). Breadcrumb nav to conversations list. Currency mode support |
| `conversations.ts` | `<dm-conversations>` | `#conversations` | Searchable conversation cards with message count, sats, task count. "New Chat" button. Sort selector (recent/cost/messages). Cost filter pills (all/free/paid/expensive). Stats bar (total conversations, messages, sats). BRC-60 hash chain verification per conversation with per-message integrity detail and expandable message list. Cards link to `#conversation/{id}/detail` |
| `dashboard.ts` | `<dm-dashboard>` | `#dashboard` | Simplified 30-second health-check page. Balance hero card with `<worm-sparkline>` (7-day trend, links to `#budget`). "Today" stats row (conversations today with active indicator, spent today, total tasks with all-time spend). Budget health gauge (highest-utilization tier, color-coded at 60/80/95%). Recent conversations (5 max, links to all). 7-day spending trend `<worm-bar-chart type="line">`. Skeleton loading placeholders. Currency mode support |
| `demo-overlay.ts` | `<dm-demo-overlay>` | `#demo/{pillar}` | Investor demo with 4 pillar nav (stats/budgeting/auditability/accounting), stat tickers, competitor comparison cards with market quotes |
| `memory.ts` | `<dm-memory>` | (child) | Category filter tabs (all/knowledge/session/execution) with counts, debounced BM25 search (300ms), expandable memory cards with full content lazy-load via `GET /memory/{id}`, tag pills. Used as child of both agent.ts (Memory tab) and standalone |
| `replay-view.ts` | `<dm-replay-view>` | `#task/{id}/replay` | Task replay timeline: event-by-event breakdown with index/type/cost/elapsed, SVG cumulative cost graph with gradient fill, tool usage table, fork panel to create new task from any event. 9 event type badges (think_request, think_response, tool_call, tool_result, user, system, proof_created, budget_check, error). Breadcrumb nav to task detail |
| `reports.ts` | `<dm-reports>` | `#reports` | CFO accounting page: date range selector (`<worm-date-range>`, default MTD), 4-stat hero with period-over-period comparison badges, spending trend line chart, service breakdown (doughnut + table), top 10 tasks by cost, cost distribution histogram, CSV + JSON export, print stylesheet |
| `services.ts` | `<dm-services>` | `#services` | x402 service catalog. Hardcoded entries for 9+ providers (banana, veo, openai, claude, nanostore, twitter, perplexity, flux, whisper) with expandable detail panels showing endpoints, pricing, example code, constraints, and "Try in Chat" prompt link. Search filter. Live discovery via `GET /services/catalog` merges additional services not in hardcoded catalog. Delivery type badges (Synchronous/Async-poll/Two-step) |
| `settings.ts` | `<dm-settings>` | `#settings` | Read-only display of server configuration. Sections: **Agent** (default model, provider, context window, max tokens), **Wallet** (identity key with copy, wallet URL, connection status), **Budget** (limit tiers with currency display), **Preferences** (currency toggle dispatches `currency-change` event). Fetches `/agent`, `/budget`, `/health` in parallel |
| `task-list.ts` | `<dm-task-list>` | `#tasks` | Paginated task list (25/page) titled "Activity", sorted by `started_at` desc. Origin filter chips (all/chat/schedule/external/system) with 8 origin types (Chat, API, External, Fork, Scheduled, Continuation, Reflection, Checklist). Stats bar (total, spent, tokens, active) — counts update to reflect active filter. Status badges, origin badges, sats/USD (currency mode), token count, proof count. Fetches `/tasks` and BSV/USD rate |
| `wallet.ts` | `<dm-wallet>` | `#wallet` | Wallet "fuel gauge" page. 3-stat hero (spendable balance with USD, UTXO count, estimated calls remaining at ~300 sats/call). 4-tier UTXO distribution bar (dust <1K / small 1K-100K / medium 100K-1M / large >1M) with animated fill and legend. Receive address with copy button. "Check for Funding" button (`POST /wallet/check-funding`). Paginated UTXO table (20/page, expandable) with tier-colored dots and outpoint links. Refresh button. Currency mode. Fetches `/wallet/utxos`, `/wallet/address` |
| `audit/audit.ts` | `<dm-audit>` | `#task/{id}` | 3-view task inspector: timeline (delegates to `<dm-audit-timeline>`), conversation (chat bubbles), chain (delegates to `<worm-proof-chain>` with continuity check via prev_hash validation). Summary cards, budget breakdown, CSV + JSON audit export via `/task/{id}/audit/export`. Breadcrumb nav |
| `audit/audit-artifacts.ts` | `<dm-audit-artifacts>` | (child) | Per-task artifact list. Fetches `GET /task/{taskId}/artifacts`. File cards with type icons (language-specific: Rust, TS, Python, etc.), name, size, tool name. Image thumbnails inline. Links to artifact URLs |
| `audit/audit-global.ts` | `<dm-audit-global>` | `#audit-trail` | Unified timeline from latest 30 tasks. `<worm-date-range>` filter, 6 event type chip filters (all/paid/proofs/tools/decisions/system) with count badges, expandable event detail with WhatsOnChain links, 100-event pagination with "Load More". Currency mode |
| `audit/audit-timeline.ts` | `<dm-audit-timeline>` | (child) | Events grouped by iteration. Collapsible iteration headers with model, sats, tokens, duration. 6-category filter. 17 event types. Budget breakdown tables. Pre/post-event grouping (before first think_request, after session_end). Public `toggleBudget()` method for parent sync |
| `budget/budget-panel.ts` | `<dm-budget-panel>` | `#budget` | 3-6 gauge bars (per-task/per-hour/per-day always, per-week/per-month/lifetime when configured >0) with color transitions at 60/80/95%. Alert banners at warning/critical/exceeded with pulse animation. Per-service bars sorted by sats. Active task context display (name, status dot, task ID). `<worm-help>` tooltips on gauge labels. Adaptive 5s polling (only when task running). Compact mode hides service detail + heading for dashboard embedding. Includes `<dm-spending-export>`. Currency mode |
| `budget/spending-export.ts` | `<dm-spending-export>` | (child) | CSV export button. Fetches `/budget/detail`, generates `lobster-farm-spending-{date}.csv` with service/operation/sats/USD columns |
| `chat/chat.ts` | `<dm-chat>` | `#` (default) | Transcript-driven streaming chat. Poll-based via `TranscriptPoller` (500ms). Inline `<worm-budget-strip>` for live budget display during tasks. Message queue for sends while busy. Session persistence via localStorage. Active task recovery on refresh via `GET /status`. Cancel support. Scroll anchoring with "New activity below" pill. Conversation history fallback (BRC-60 → OpenAI format via `_buildMessagesFromConversation()` / `_buildMessagesFromOpenAI()`). Low-balance warning (<50K sats). Currency-aware budget errors via `_formatBudgetError()` |
| `chat/chat-header.ts` | `<dm-chat-header>` | (child) | Session title bar with "All Chats" link and "New Chat" button. Dispatches `new-conversation` event |
| `chat/chat-input.ts` | `<dm-chat-input>` | (child) | Input row with model picker flyout (9 models: 6 OpenAI + 3 Claude), provider dots, currency-aware sats pill, token pill, Enter-to-send. Persists model selection to localStorage |
| `proofs/proofs.ts` | `<dm-proofs>` | `#task/{id}/proofs` | Proof cards for 6 types (Decision, TaskCompletion, BudgetSnapshot, Checkpoint, CapabilityProof, MemoryCommitment). Client-side SHA-256 hash verification with glow animation (shake on mismatch). "Verify All" button with progress bar. On-chain verification. Checkpoint decryption via `xhrFetch` (XHR bypass for MetaNet Client interceptor). State token section (BRC-48) separated from chain proofs. Explainer `<details>` section. Receipt table via `<worm-receipt-table>`. WhatsOnChain links. Currency mode |

## Key Exports

### From `chat/chat-input.ts`
- **`ModelDef`** — Interface: `{ id, label, cost, provider }`
- **`MODELS`** — Array of 9 model definitions (6 OpenAI: gpt-5-nano, gpt-5-mini, gpt-5, gpt-5.2, o4-mini, gpt-5.2-pro; 3 Claude: claude-haiku-4-5, claude-sonnet-4-6, claude-opus-4-6)

### From `replay-view.ts`
- **`ReplayEvent`** — Interface: `{ index, timestamp, event_type, id, iteration, event_sats, cumulative_sats, tool_name?, model?, elapsed_secs, data }`
- **`CostPoint`** — Interface: `{ elapsed_secs, cumulative_sats, event_type, index }`
- **`ToolUsage`** — Interface for tool call tracking in replay
- **`ReplayData`** — Interface: `{ task_id, total_events, total_iterations, total_sats, duration_secs, events[], cost_timeline[], tool_usage[] }`

### From `certificates.ts`
- **`CapabilityDef`** — Interface: `{ id, label, desc, icon, color }`
- **`CAPABILITIES`** — Array of 7 capability definitions (llm, tools, messaging, x402, wallet, memory, schedule)
- **`DEFAULT_CAPS`** — Set of default capabilities (llm, tools, messaging, x402)

### From `agent.ts`
- **`AgentTab`** — Type union: `'identity' | 'tools' | 'memory' | 'telemetry'`
- **`categorize(toolName)`** — Maps tool names to 7 categories (sandbox, system, wallet, memory, messagebox, x402, other)
- **`CATEGORY_STYLES`** — Record of category color styling
- **`CATEGORY_ORDER`** — Array defining category display order

### From `reports.ts`
- **`ReportsData`** — Interface: `{ tasks, agent, budgetDetail, usdRate }`
- **`computePreviousPeriod()`** — Computes previous-period metrics for comparison badges
- **`pctChange()`** — Calculates percentage change between periods

### From `memory.ts`
- **`CategoryFilter`** — Type union: `'all' | 'knowledge' | 'session' | 'execution'`
- **`CATEGORY_COLORS`** — Record mapping categories to colors

### From `audit/audit-timeline.ts`
- **`AuditEvent`** — Interface for transcript events
- **`IterationGroup`** — Interface for grouped event batches

## Patterns

### Data Fetching
Pages use `FetchController<T>` from `../controllers/fetch-controller.ts` for one-shot loads, or manual fetch with `@state loading/error` for more control. The `fetchFn` property carries BRC-31 auth from `wallet-auth.ts`.

### Adaptive Polling
`budget-panel.ts` and `automations.ts` use polling for live data. Budget panel polls `/status` and `/budget` every 5s only when a task is running (adaptive — stops when idle). Automations polls at a fixed 30s interval.

### Chat Streaming
`chat.ts` uses `TranscriptPoller` (from `../lib/transcript-poller.ts`) which polls `GET /task/{id}/events?since={index}` at 500ms. A `scheduleSync()` throttle (80ms) batches DOM updates. Messages sent while the agent is busy are queued and auto-flushed.

### Currency Display
Nearly all pages accept a `currencyMode` property from the app shell. They use `formatDualCurrency()`, `formatInlineCurrency()`, and `formatInlineCurrencyAlt()` from `../lib/usd.ts` for sats/USD display. Currency preference comes from `getCurrencyDisplay()` in `../lib/storage.ts`.

### Date Range Filtering
`reports.ts` and `audit-global.ts` use the `<worm-date-range>` component and filter spending entries/tasks by date. `dashboard.ts` uses a fixed 7-day window for its spending trend.

### Period-over-Period Comparison
`reports.ts` computes previous-period metrics (`computePreviousPeriod()`) and renders comparison badges showing percentage change with up/down arrows.

### Shared Styles
All pages import from `../lib/shared-styles.ts`. Common templates: `pageHost`, `centerState`, `pageTitle`, `summaryGrid`, `statsBar`, `badges`, `cardBase`, `tabBar`, `gaugeBar`, `alertBanner`, `sectionTitle`, `buttonStyles`, `searchInput`, `stateFeedback`, `skeleton`.

### Render Helpers
Pages use `renderLoading()`, `renderEmpty()`, `renderSmartEmpty()`, `renderError()`, `renderBreadcrumb()` from `../lib/shared-styles.ts` for consistent loading/empty/error states. `renderSmartEmpty()` provides context-aware empty states with custom icons, titles, subtitles, and optional action buttons.

### Custom Events
- `send` / `cancel` / `model-change` — chat-input -> chat
- `new-conversation` — chat-header -> chat
- `budget` / `done` — chat -> app shell
- `currency-change` — settings -> app shell
- `range-change` — date-range -> reports/audit-global

### Responsive Breakpoints
- `<640px` — Mobile: stacked layouts, 2-column grids
- `640-1023px` — Tablet: wrapped flex, 2-column cards
- `1024px+` — Desktop: full sidebar, 4-column grids

## API Endpoints Used

| Endpoint | Used By |
|----------|---------|
| `GET /agent` | agent, dashboard, reports, settings |
| `GET /tasks` | task-list, dashboard, reports, audit-global |
| `GET /status` | chat (reconnect), budget-panel (active task) |
| `GET /task/{id}/audit` | audit, audit-global, conversation-detail |
| `GET /task/{id}/audit/export` | audit (CSV + JSON export) |
| `GET /task/{id}/receipts` | audit, proofs |
| `GET /task/{id}/proofs` | audit, proofs |
| `GET /task/{id}/proofs/verify` | proofs |
| `GET /task/{id}/conversation` | audit (lazy) |
| `GET /task/{id}/events` | chat (via TranscriptPoller) |
| `GET /task/{id}/artifacts` | audit-artifacts |
| `POST /task/{id}/cancel` | chat |
| `GET /conversations` | conversations, dashboard |
| `GET /conversations/{id}` | chat (history), conversation-detail |
| `GET /conversations/{id}/artifacts` | conversation-detail (artifacts tab) |
| `GET /conversations/{id}/verify` | conversations, conversation-detail |
| `GET /budget` | budget-panel, budget-strip, dashboard, settings |
| `GET /budget/detail` | dashboard, reports, audit, spending-export |
| `GET /rates/bsv-usd` | agent, chat, dashboard, task-list, reports, replay-view, budget-panel, compliance, conversations, conversation-detail, audit, audit-global, proofs, settings |
| `GET /memory` | memory (standalone + agent Memory tab) |
| `GET /memory/search` | memory |
| `GET /memory/{id}` | memory |
| `GET /schedules` | automations |
| `GET /task/{id}/replay` | replay-view |
| `POST /task/{id}/fork` | replay-view |
| `GET /certificates` | certificates, agent |
| `POST /certificates/issue` | certificates |
| `POST /certificates/relinquish` | certificates |
| `POST /certificates/revoke` | certificates |
| `GET /compliance/report` | compliance |
| `GET /artifacts` | artifacts |
| `GET /services/catalog` | services |
| `GET /wallet/utxos` | wallet |
| `GET /wallet/address` | wallet |
| `POST /wallet/check-funding` | wallet |
| `GET /health` | settings |
| `POST /chat` | chat |
| `POST /decrypt` | proofs |
| `GET /output/{basket}/{txid}` | proofs |

## Related

- **Parent**: [`ui/src/CLAUDE.md`](../CLAUDE.md) — Full UI architecture, shared lib, components, controllers
- **Components**: `../components/` — Reusable widgets (`tool-card`, `proof-chain`, `bar-chart`, `date-range`, `sparkline`, `budget-strip`, `snackbar`, etc.)
- **Shared lib**: `../lib/` — Utilities (`util.ts`), types (`shared-types.ts`), styles (`shared-styles.ts`), auth (`wallet-auth.ts`), polling (`transcript-poller.ts`), storage (`storage.ts`), routing (`router.ts`), labels (`labels.ts`), constants (`constants.ts`)
- **Server routes**: `src/server/` — Axum handlers that these pages consume
