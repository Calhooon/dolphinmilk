# ui/src
> Lit web components for the bsv-worm parent console — chat UI with transcript-driven streaming, multi-turn conversations, audit dashboard, reports, compliance, and wallet auth.

## Overview

Browser frontend for interacting with the worm agent. Built with Lit 3 web components, Vite, and TypeScript (52 source files). Connects to the axum HTTP server (`src/server/`) via transcript polling and REST endpoints. Features hash-based routing across fifteen views: Chat, Dashboard, Budget, Reports, Compliance, Audit Trail (global), Tasks (Activity), Conversations, Automations, Certificates, Agent (with Identity/Tools/Memory/Telemetry tabs), Audit (per-task), Proofs, Demo, and Replay. Branded as "Lobster Farm".

Authentication uses BRC-31 AuthFetch from `@bsv/sdk` (provided at runtime by MetaNet Client), with automatic fallback to dev mode (plain fetch, no auth) when no wallet is present.

## Directory Structure

```
ui/src/
├── main.ts                          # Entry point — registers all custom elements
├── app.ts                           # <worm-app> root shell, hash router, sidebar
├── styles.css                       # Global CSS custom properties (dark theme)
├── views/                           # Standalone view components (not route-level pages)
│   └── telemetry-view.ts            # <worm-telemetry> — Prometheus metrics dashboard
├── lib/                             # Shared utilities and types
│   ├── constants.ts                 # Shared constants (SERVICE_NAMES, TYPE_COLORS, etc.)
│   ├── data-cache.ts                # TTL-based fetch cache with promise coalescing
│   ├── icons.ts                     # 15 SVG nav/brand icons as Lit TemplateResult exports
│   ├── labels.ts                    # Business-language labels for events, proofs, budget
│   ├── proof-utils.ts               # SHA-256 proof hash computation (Web Crypto API)
│   ├── router.ts                    # Hash-based route parser (Route type, parseRoute())
│   ├── shared-styles.ts             # Reusable Lit CSS modules (16 templates + 4 render helpers)
│   ├── shared-types.ts              # Canonical API response interfaces (30)
│   ├── storage.ts                   # localStorage helpers (model, session, currency, date range)
│   ├── transcript-poller.ts         # TranscriptPoller — polls /task/{id}/events
│   ├── ui-types.ts                  # UI-specific types (ChatMessage, ToolCallInfo, transcript DTOs)
│   ├── usd.ts                       # BSV/USD conversion + dual-currency formatting helpers
│   ├── util.ts                      # Formatting helpers (time, sats, keys)
│   └── wallet-auth.ts               # BRC-31 AuthFetch setup + dev-mode fallback
├── controllers/                     # Lit ReactiveControllers
│   ├── fetch-controller.ts          # Async data loading with data/loading/error
│   └── polling-controller.ts        # Interval-based polling with lifecycle
├── components/                      # Reusable UI components
│   ├── bar-chart.ts                 # <worm-bar-chart> — Chart.js wrapper (bar/doughnut/line/stacked)
│   ├── budget-strip.ts              # <worm-budget-strip> — Compact progress + budget bar for chat
│   ├── date-range.ts                # <worm-date-range> — Date picker with presets + custom range
│   ├── message.ts                   # <worm-message> — Chat bubble with markdown
│   ├── proof-chain.ts               # <worm-proof-chain> — Visual proof chain
│   ├── proof-inline.ts              # <worm-proof-inline> — Single proof card
│   ├── receipt-table.ts             # <worm-receipt-table> — BEEF receipt breakdown
│   ├── snackbar.ts                  # <worm-snackbar> — Toast notifications
│   ├── sparkline.ts                 # <worm-sparkline> — Inline SVG trend line
│   ├── stat-ticker.ts               # <worm-stat-ticker> — Animated counters
│   ├── tool-card.ts                 # <worm-tool-card> — Tool call display
│   └── tooltip.ts                   # <worm-tooltip> + <worm-help> — Hover tooltips
└── pages/                           # Route-level page components
    ├── agent.ts                     # <worm-agent> — Agent identity, tools, memory, telemetry tabs
    ├── automations.ts               # <worm-automations> — Recurring schedule list
    ├── certificates.ts              # <worm-certificates> — BRC-52 cert mgmt
    ├── compliance.ts                # <worm-compliance> — Compliance dashboard
    ├── conversations.ts             # <worm-conversations> — Conversation list
    ├── dashboard.ts                 # <worm-dashboard> — Overview page
    ├── demo-overlay.ts              # <worm-demo-overlay> — Investor demo
    ├── memory.ts                    # <worm-memory> — Memory browser
    ├── replay-view.ts               # <worm-replay-view> — Task replay timeline + fork
    ├── reports.ts                   # <worm-reports> — CFO accounting + export
    ├── task-list.ts                 # <worm-task-list> — All tasks list
    ├── audit/                       # Audit views
    │   ├── audit.ts                 # <worm-audit> — Per-task audit (3 views + export)
    │   ├── audit-global.ts          # <worm-audit-global> — Cross-task timeline
    │   └── audit-timeline.ts        # <worm-audit-timeline> — Timeline renderer
    ├── budget/                      # Budget views
    │   ├── budget-panel.ts          # <worm-budget-panel> — Budget gauges
    │   └── spending-export.ts       # <worm-spending-export> — CSV export
    ├── chat/                        # Chat views
    │   ├── chat.ts                  # <worm-chat> — Main chat panel
    │   ├── chat-header.ts           # <worm-chat-header> — Conversation header bar
    │   └── chat-input.ts            # <worm-chat-input> — Model picker + input row
    └── proofs/                      # Proof views
        └── proofs.ts                # <worm-proofs> — Proof viewer + verification
```

## Files

### Root Files
| File | Purpose |
|------|---------|
| `main.ts` | Entry point. Imports all 35 page and component modules to register custom elements. |
| `styles.css` | Global CSS. Root custom properties (dark theme), scrollbar styling, code block formatting, spinner animation. |
| `app.ts` | `<worm-app>` — Root shell. Hash-based router (15 routes via `parseRoute()`), collapsible sidebar nav with SVG icons (from `lib/icons.ts`). Top-level nav: Chat, Conversations, Agent. Two collapsible groups: **Operations** (Dashboard, Activity, Automations, Budget, Reports) and **Governance** (Audit Trail, Compliance, Certificates). Governance collapsed by default. Auth state, health polling, balance + USD display with currency toggle ($/S button), mobile hamburger drawer. Branded "Lobster Farm" with logo.png. 5-click logo easter egg toggles demo overlay. Passes `currencyMode` to child pages. |

### `lib/` — Shared Utilities
| File | Purpose |
|------|---------|
| `constants.ts` | Shared constants: `SERVICE_NAMES` (16 entries), `SERVICE_COLORS` (8-color palette), `PROOF_TYPE_LABELS` (7 types), `TYPE_COLORS`, `TYPE_SHAPES`, `friendlyName()`. |
| `data-cache.ts` | `DataCache` class — TTL-based fetch cache with promise coalescing. Methods for tasks, agent, budget, budgetDetail, conversations, schedules, certificates. Server restart detection. |
| `icons.ts` | 15 SVG nav/brand icons as Lit `TemplateResult` exports (stroke-based, 24x24, currentColor). Chat, Dashboard, Budget, Reports, Compliance, Search, Tasks, Conversations, Memory, Schedules, Certificates, Agent, Bug (brand), Menu (hamburger), Chevron (group toggle). |
| `labels.ts` | Business-language translations: `EVENT_LABELS` (17 types), `EVENT_DESCRIPTIONS` (9 tooltips), `PROOF_LABELS` (6 types with descriptions), `BUDGET_HELP` (6 gauges), `SECTION_HELP` (5 sections). |
| `proof-utils.ts` | `computeProofHash(data, timestamp, prevHash?)` — SHA-256 via Web Crypto API. Matches Rust's `compute_proof_hash()` in `src/onchain/proofs.rs`. |
| `router.ts` | `Route` type (15 routes) and `parseRoute(hash)` — maps URL hash fragments to `{ route, params }`. Handles nested routes (`task/{id}/proofs`, `task/{id}/replay`), agent sub-routes (`agent/tools`, `agent/memory`, `agent/telemetry`), legacy aliases (`#memory`/`#knowledge` → agent memory tab, `#schedules` → `#automations`, `#telemetry` → agent telemetry tab, `#activity` → tasks). |
| `shared-styles.ts` | 16 reusable Lit CSS modules + 4 render helpers (`renderError`, `renderEmpty`, `renderLoading`, `renderBreadcrumb`). Includes `skeleton` module for shimmer loading placeholders. See Shared Infrastructure below. |
| `shared-types.ts` | Canonical shared interfaces: 30 interfaces including `TaskSummary`, `AgentInfo`, `BudgetReport`, `ProofsResponse`, `ConversationSummary`, `AuditEvent`, `ProofDetail`, `CertificateFields`, plus `getStatusClass()` helper. |
| `storage.ts` | localStorage helpers: model (`worm-model`, default `gpt-5-mini`), session ID (`worm-session-id`), currency display (`worm-currency`, default `usd-first`), date range (`worm-date-range`, default `30d`). Exports `CurrencyDisplay` and `StoredDateRange` types. |
| `transcript-poller.ts` | `TranscriptPoller` class — Polls `/task/{id}/events`, converts JSONL transcript events into `ChatMessage[]`. Core state machine for chat UI. Tracks cumulative token count and iteration number. |
| `ui-types.ts` | UI-specific types: `ChatMessage` (role, content, toolCalls, timestamp, pending), `ToolCallInfo` (callId, name, arguments, output, success, pending), `TranscriptEventDto`, `TaskEventsResponse`. |
| `usd.ts` | USD conversion helpers. `fetchBsvUsdRate()` (5-min cache, $20 fallback), `satsToUsd()`, `formatUsd()`, `formatDualCurrency()` (primary/secondary by mode), `formatInlineCurrency()` (compact), `formatInlineCurrencyAlt()` (tooltip alternate). |
| `util.ts` | Formatting helpers: `formatRelativeTime()`, `formatDuration()`, `formatUptime()`, `truncate()`, `parseTimestamp()`, `truncateKey()`, `formatSats()`, `formatTokens()`. |
| `wallet-auth.ts` | `authenticate()` — BRC-31 AuthFetch setup via `@bsv/sdk`, with dev-mode fallback. REST-only auth. |

### `controllers/` — Lit ReactiveControllers
| File | Purpose |
|------|---------|
| `fetch-controller.ts` | `FetchController` — Async data loading with `data`, `loading`, `error` states. Auto-triggers host update on state change. |
| `polling-controller.ts` | `PollingController` — Interval-based polling with start/stop/pause/resume lifecycle. Configurable interval, auto-cleanup on disconnect. |

### `components/` — Reusable UI Components
| File | Purpose |
|------|---------|
| `bar-chart.ts` | `<worm-bar-chart>` — Chart.js wrapper. Supports bar (vertical/horizontal), doughnut, line (gradient fill), and stacked bar chart types. Dark theme, monospace labels, USD tooltips when `usdRate` set. Exports `BarData` and `StackedBarData` interfaces. |
| `budget-strip.ts` | `<worm-budget-strip>` — Compact 28px progress bar + budget strip for chat view. Shows iteration step, per-task sats vs limit with color-coded gauge (blue/yellow/orange/red), hourly/daily totals, wallet balance. Polls `GET /budget` every 5s when active. Clicks navigate to `#budget`. Currency-mode aware. |
| `date-range.ts` | `<worm-date-range>` — Date range picker with 7 preset pills (Today, 7d, 30d, MTD, QTD, YTD, Custom). Custom mode shows two date inputs. Persists to localStorage. Dispatches `range-change` event. Exports `DatePreset`, `DateRange`, `defaultRange()`. |
| `message.ts` | `<worm-message>` — Chat bubble. Markdown (marked + DOMPurify) with image/video/audio support, pending state with spinner. |
| `proof-chain.ts` | `<worm-proof-chain>` — Visual proof chain. Colored nodes by type, connectors (verified/broken/unknown), legend with `<worm-tooltip>` descriptions, detail panel on selection, verify-all button. |
| `proof-inline.ts` | `<worm-proof-inline>` — Single proof card. Type badge with `<worm-tooltip>`, hash, verify button (SHA-256), copy txid, WhatsOnChain link, expandable detail. |
| `receipt-table.ts` | `<worm-receipt-table>` — BEEF payment receipt cost breakdown table. Currency-aware via `currencyMode` prop. Shows per-iteration costs with totals. |
| `snackbar.ts` | `<worm-snackbar>` — Toast notification. Types: info/warning/error/success, auto-dismiss (5s default), close button, fixed bottom-center. |
| `sparkline.ts` | `<worm-sparkline>` — Inline SVG sparkline for trend visualization. Props: `data` (number array), `color`, `width`, `height`. Gradient fill + stroke line. Respects `prefers-reduced-motion`. |
| `stat-ticker.ts` | `<worm-stat-ticker>` — Animated number counter with easeOutCubic over 1.5s. Prefix/suffix support, scientific notation handling. |
| `tool-card.ts` | `<worm-tool-card>` — Expandable card showing tool name, arguments (pretty-printed JSON), output, success/pending state, copy button. |
| `tooltip.ts` | `<worm-tooltip>` — Hover/focus tooltip (top/bottom, 280px max). `<worm-help>` — Small "?" circle icon wrapping `<worm-tooltip>` for contextual help. |

### `pages/` — Route-Level Page Components
| File | Purpose |
|------|---------|
| `agent.ts` | `<worm-agent>` — Agent overview with 4-tab interface: **Identity** (key, version, certificate badge, uptime, 4-stat grid), **Tools** (registry grouped by 7 categories with colored pills), **Memory** (embeds `<worm-memory>`), **Telemetry** (embeds `<worm-telemetry>`). Accepts `activeTab` prop from router. |
| `certificates.ts` | `<worm-certificates>` — BRC-52 certificate management. Issue, relinquish, revoke with confirmation. Revocation outpoint display. Raw JSON toggle. |
| `compliance.ts` | `<worm-compliance>` — Compliance dashboard. WORM mode, compliance mode, retention policy, regulation pills. Summary stats (tasks/proofs/sats). Date-filtered task table. Fetches `GET /compliance/report`. |
| `conversations.ts` | `<worm-conversations>` — Conversations list with search, stats bar, card layout. BRC-60 hash chain verification with per-message integrity detail. |
| `dashboard.ts` | `<worm-dashboard>` — Overview page. 5-stat hero grid with sparklines, embedded budget gauges, date-range-aware spending charts (doughnut + stacked bar/line auto-switch), activity feed, top tasks. Currency mode support. |
| `demo-overlay.ts` | `<worm-demo-overlay>` — Investor-facing demo with pillar navigation, stat tickers, comparison cards, market quotes. |
| `automations.ts` | `<worm-automations>` — Schedule cards with enabled/disabled badge, type badge (Cron/One-shot/Interval), human-readable intervals, cron expression, countdown to next run, run count. Expandable cards with run history (status badges). Stats bar (total, enabled, runs). 30s auto-polling. Graceful 404 handling. |
| `memory.ts` | `<worm-memory>` — Memory browser. Category tabs, debounced BM25 search (300ms), expandable detail cards with tag pills. Used as child of agent.ts (Memory tab) and standalone. |
| `replay-view.ts` | `<worm-replay-view>` — Task replay timeline. Event-by-event breakdown with index/type/cost/elapsed, SVG cumulative cost graph with gradient fill, tool usage table, fork panel to create new task from any event. 9 event type badges. Breadcrumb nav to task detail. Fetches `GET /task/{id}/replay`, `POST /task/{id}/fork`. |
| `reports.ts` | `<worm-reports>` — CFO accounting page. `<worm-date-range>` selector, 4-stat hero with period-over-period comparison badges (up/down arrows), spending trend line chart, service breakdown (doughnut + table), top 10 tasks by cost, cost distribution histogram, CSV + JSON export buttons, print stylesheet. |
| `task-list.ts` | `<worm-task-list>` — Task list page. Summary stats bar, paginated (25/page) card list with status badges, dual-currency costs, token counts. |
| `audit/audit.ts` | `<worm-audit>` — Per-task audit. Three views (timeline/conversation/chain), summary cards, CSV + JSON audit export via `/task/{id}/audit/export`, currency mode. Breadcrumb nav. |
| `audit/audit-global.ts` | `<worm-audit-global>` — Cross-task audit timeline. `<worm-date-range>` filter, 6 event type chip filters, expandable events, load-more pagination. Currency mode. |
| `audit/audit-timeline.ts` | `<worm-audit-timeline>` — Timeline renderer. Iteration grouping, 6-category event filtering, collapsible groups, budget breakdown with cost categories. Exports `AuditEvent` and `IterationGroup` types. |
| `budget/budget-panel.ts` | `<worm-budget-panel>` — Budget limit gauges (per-task/hour/day) with color-coded fill bars, alert banners, per-service breakdown. Compact mode for dashboard embedding. Currency mode. |
| `budget/spending-export.ts` | `<worm-spending-export>` — One-click CSV export of spending data. |
| `chat/chat.ts` | `<worm-chat>` — Main chat panel. Transcript-driven UI via `TranscriptPoller`, 500ms polling, conversation sessions, message queue, scroll tracking, cancel support. Embeds `<worm-budget-strip>` for live iteration progress and budget display during active tasks. |
| `chat/chat-header.ts` | `<worm-chat-header>` — Conversation header bar. Shows title, "All Chats" link, "New Chat" button. |
| `chat/chat-input.ts` | `<worm-chat-input>` — Input row with model picker flyout (9 models across OpenAI + Claude). Persists model via `storage.ts`. Exports `ModelDef` and `MODELS`. |
| `proofs/proofs.ts` | `<worm-proofs>` — Proof viewer. Client-side SHA-256 hash verification, on-chain verification, checkpoint decryption via XHR bypass, structured proof data display, receipt table. Currency mode. Uses `<worm-receipt-table>` for cost breakdown. |

### `views/` — Standalone View Components
| File | Purpose |
|------|---------|
| `telemetry-view.ts` | `<worm-telemetry>` — Prometheus metrics dashboard. Fetches `GET /metrics` (text format), parses counters/gauges/histograms, renders grouped metric cards with live values. 10s auto-refresh. Groups metrics by prefix (http, task, llm, budget, etc.). Shows last refresh timestamp. Embedded in `<worm-agent>` Telemetry tab (no longer a standalone route — `#telemetry` redirects to `#agent/telemetry`). |

## Routing

`WormApp` uses hash-based routing. Route parsing is handled by `parseRoute()` in `lib/router.ts`.

| Hash | Route | Component | API |
|------|-------|-----------|-----|
| `#` (empty) | `chat` | `<worm-chat>` | `POST /chat`, `GET /task/{id}/events` |
| `#dashboard` | `dashboard` | `<worm-dashboard>` | `GET /tasks`, `GET /agent`, `GET /budget/detail`, `GET /task/{id}/audit`, `GET /rates/bsv-usd` |
| `#budget` | `budget` | `<worm-budget-panel>` | `GET /budget` |
| `#reports` | `reports` | `<worm-reports>` | `GET /tasks`, `GET /agent`, `GET /budget/detail`, `GET /rates/bsv-usd` |
| `#compliance` | `compliance` | `<worm-compliance>` | `GET /compliance/report` |
| `#audit-trail` | `audit-global` | `<worm-audit-global>` | `GET /tasks`, `GET /task/{id}/audit`, `GET /rates/bsv-usd` |
| `#conversations` | `conversations` | `<worm-conversations>` | `GET /conversations`, `GET /conversations/{id}/verify` |
| `#conversation/{id}` | `chat` | `<worm-chat>` | `POST /chat`, `GET /conversations/{id}` |
| `#tasks` / `#activity` | `tasks` | `<worm-task-list>` | `GET /tasks` |
| `#automations` | `automations` | `<worm-automations>` | `GET /schedules` |
| `#certificates` | `certificates` | `<worm-certificates>` | `GET /certificates`, `POST /certificates/issue`, `POST /certificates/relinquish`, `POST /certificates/revoke` |
| `#task/{id}` | `audit` | `<worm-audit>` | `GET /task/{id}/audit`, `GET /task/{id}/receipts`, `GET /budget/detail?task_id={id}`, `GET /task/{id}/proofs` |
| `#task/{id}/proofs` | `proofs` | `<worm-proofs>` | `GET /task/{id}/proofs`, `GET /task/{id}/proofs/verify` |
| `#task/{id}/replay` | `replay` | `<worm-replay-view>` | `GET /task/{id}/replay`, `POST /task/{id}/fork` |
| `#agent` | `agent` | `<worm-agent>` (Identity tab) | `GET /agent`, `GET /certificates` |
| `#agent/tools` | `agent` | `<worm-agent>` (Tools tab) | `GET /agent` |
| `#agent/memory` | `agent` | `<worm-agent>` (Memory tab) | `GET /memory`, `GET /memory/search`, `GET /memory/{id}` |
| `#agent/telemetry` | `agent` | `<worm-agent>` (Telemetry tab) | `GET /metrics` |
| `#demo/{pillar}` | `demo` | `<worm-demo-overlay>` | none (static content) |

Legacy aliases: `#memory`/`#knowledge` → `#agent/memory`, `#schedules`/`#knowledge/schedules` → `#automations`, `#telemetry` → `#agent/telemetry`, `#activity` → `#tasks`.

The `Route` type is defined in `lib/router.ts`: `'chat' | 'tasks' | 'audit' | 'audit-global' | 'proofs' | 'replay' | 'agent' | 'conversations' | 'dashboard' | 'certificates' | 'automations' | 'budget' | 'reports' | 'compliance' | 'demo'`.

## Component Tree

```
<worm-app>
  ├── sidebar                        (collapsible, 220px / 64px collapsed)
  │     ├── brand header             (logo.png + "Lobster Farm", 5-click → demo toggle)
  │     ├── nav: Chat                (top-level, SVG icons from lib/icons.ts)
  │     ├── nav: Conversations       (top-level)
  │     ├── nav: Agent               (top-level)
  │     ├── nav group: OPERATIONS    (Dashboard, Activity, Automations, Budget, Reports)
  │     ├── nav group: GOVERNANCE    (Audit Trail, Compliance, Certificates) — collapsed by default
  │     └── footer                   (connection dot, identity key, balance + USD, currency toggle, model)
  ├── mobile bar                     (hamburger + logo, <640px only)
  ├── <worm-chat>                    (route: chat)
  │     ├── <worm-chat-header>       (conversation title + nav links)
  │     ├── <worm-budget-strip>      (iteration progress + budget bar)
  │     ├── <worm-chat-input>        (model picker flyout + input row)
  │     ├── <worm-message>           (per chat message)
  │     ├── <worm-tool-card>         (per tool call)
  │     └── <worm-snackbar>          (toast notifications)
  ├── <worm-dashboard>               (route: dashboard)
  │     ├── <worm-sparkline>         (hero card trends)
  │     └── <worm-bar-chart>         (spending trend line)
  ├── <worm-budget-panel>            (route: budget, full view)
  ├── <worm-reports>                 (route: reports)
  │     ├── <worm-date-range>        (period selector)
  │     ├── <worm-bar-chart>         (trend + doughnut charts)
  │     └── <worm-spending-export>   (CSV export)
  ├── <worm-compliance>              (route: compliance)
  ├── <worm-audit-global>            (route: audit-global)
  │     └── <worm-date-range>        (date range filter)
  ├── <worm-conversations>           (route: conversations)
  ├── <worm-task-list>               (route: tasks / activity)
  ├── <worm-automations>             (route: automations)
  ├── <worm-certificates>            (route: certificates)
  ├── <worm-audit>                   (route: audit, task/{id})
  │     └── <worm-audit-timeline>    (timeline renderer with iteration groups)
  ├── <worm-proofs>                  (route: proofs, task/{id}/proofs)
  │     ├── <worm-proof-chain>       (visual chain visualization)
  │     ├── <worm-proof-inline>      (individual proof cards)
  │     └── <worm-receipt-table>     (BEEF receipt cost breakdown)
  ├── <worm-agent>                   (route: agent, 4 tabs)
  │     ├── Identity tab             (hero stats, cert badge)
  │     ├── Tools tab                (7-category registry)
  │     ├── Memory tab               (embeds <worm-memory>)
  │     └── Telemetry tab            (embeds <worm-telemetry>)
  ├── <worm-replay-view>             (route: replay, task/{id}/replay)
  └── <worm-demo-overlay>            (route: demo)
        └── <worm-stat-ticker>       (animated counters)
```

Only one page component renders at a time based on the current route. Sidebar footer shows connection status, identity key, balance (sats + USD with currency toggle), and current model on all routes.

## Shared Infrastructure

### `shared-styles.ts` — Reusable CSS Modules + Render Helpers
Sixteen CSS template literals and 4 render helper functions. Components compose them via `static styles = [pageHost, centerState, ...]`.

**CSS Modules**: `pageHost` (flex column with padding), `centerState` (loading/error centering), `pageTitle` (heading style), `summaryGrid` (stat grid layout), `statsBar` (inline stats row), `badges` (status/category badges), `cardBase` (card styling), `tabBar` (tab button row), `gaugeBar` (budget limit progress bars), `alertBanner` (warning/critical/exceeded budget alerts with pulse animation), `sectionTitle` (section headings), `buttonStyles` (base/primary/danger/small button variants), `searchInput` (search bar with icon), `statusBadges` (running/completed/failed/cancelled/paused pills), `skeleton` (shimmer loading placeholders with text/card/chart/hero-grid variants), `stateFeedback` (error/empty/loading states + spinner + breadcrumb nav). Responsive at 640px breakpoint.

**Render Helpers**: `renderError(message, suggestion?)`, `renderEmpty(title, subtitle?)`, `renderLoading(text?)`, `renderBreadcrumb(items)`. Require `stateFeedback` CSS module to be included.

### `shared-types.ts` — Canonical Interfaces
Single source of truth for API response types. 30 interfaces organized by domain: `TaskSummary`, `AgentInfo`, `BudgetReport`, `OperationStats`, `ServiceBreakdown`, `SpendingEntry`, `BudgetDetailResponse`, `BudgetGaugeData`, `SpendingExportRow`, `GlobalAuditEvent`, `ReceiptDetail`, `ProofsResponse`, `ProofDetail`, `Schedule`, `MemoryListItem`, `MemoryListResponse`, `MemoryDetail`, `MemorySearchResult`, `CertificateFields`, `CertificateStatus`, `ConversationSummary`, `ConversationDetail`, `ConversationMessage`, `MessageVerification`, `ChainVerification`, `AuditSummary`, `AuditEvent`, `AuditConversationMessage`, `ReceiptsResponse`, `OnChainVerification`. Plus `StatusClass`, `VerifyStatus` types and `getStatusClass()` helper.

### `data-cache.ts` — TTL Cache with Coalescing
`DataCache` class prevents duplicate HTTP calls when multiple components request the same data simultaneously. TTL-based expiry (10s-30s per resource), promise coalescing for concurrent requests, `invalidate()` for cache busting, server restart detection via `/health` uptime check. Methods: `fetchTasks()`, `fetchAgent()`, `fetchBudget()`, `fetchBudgetDetail()`, `fetchConversations()`, `fetchSchedules()`, `fetchCertificates()`.

### `usd.ts` — USD Conversion & Dual-Currency Formatting
`fetchBsvUsdRate()` calls `GET /rates/bsv-usd` with 5-min in-memory cache and $20 fallback. `satsToUsd(sats, rate)` converts to dollar value. `formatUsd(sats, rate)` returns `"$X.XX"`. `formatDualCurrency(sats, rate, mode)` returns `{ primary, secondary }` strings based on `CurrencyDisplay` mode. `formatInlineCurrency()` / `formatInlineCurrencyAlt()` for compact inline and tooltip alternate formats. Used by dashboard, reports, audit views, budget panel, task list, and proofs.

## Transcript-Driven Chat Architecture

The chat UI uses a transcript-polling model instead of parsing SSE events directly:

1. **Submit**: `POST /chat` returns JSON `{ task_id, session_id }`
2. **Poll**: `TranscriptPoller` polls `GET /task/{id}/events?since={cursor}` for incremental JSONL events
3. **Timer**: 500ms polling interval drives UI updates
4. **Convert**: `TranscriptPoller.processEvent()` converts 16 event types into `ChatMessage[]` with tool call state
5. **Render**: Chat component reads `PollResult` and updates messages, thinking indicator, and tool cards

**Page refresh recovery**: `detectActiveTask()` on connect checks `GET /status` for running tasks and resumes polling.

**DOM throttling**: `scheduleSync()` batches re-renders with 80ms debounce. `force=true` flushes immediately for `done`/`error` events.

**Message queue**: If `sendMessage()` is called while `busy=true`, the text is queued. `flushQueue()` sends the next queued message when the current stream completes.

**Scroll tracking**: Passive scroll listener tracks user position. When scrolled up >150px, a "New activity below" pill appears instead of auto-scrolling.

## Model Picker

`WormChatInput` (`pages/chat/chat-input.ts`) includes a flyout model selector with 9 models:

| Provider | Models |
|----------|--------|
| OpenAI | gpt-5-nano, gpt-5-mini (default), gpt-5, gpt-5.2, o4-mini, gpt-5.2-pro |
| Claude | claude-haiku-4-5, claude-sonnet-4-6, claude-opus-4-6 |

Selected model is persisted to localStorage (`worm-model`) via `storage.ts` and sent with each `POST /chat` request.

## Currency Mode

`WormApp` manages a `currencyMode` state (`'usd-first' | 'sats-first'`) persisted via `storage.ts`. A currency toggle button in the sidebar footer switches between `$` and `S` modes. The mode is passed as a `currencyMode` prop to: `<worm-chat>`, `<worm-dashboard>`, `<worm-task-list>`, `<worm-reports>`, `<worm-audit>`, `<worm-proofs>`, `<worm-budget-panel>`, `<worm-audit-global>`, `<worm-conversations>`, `<worm-agent>`, `<worm-compliance>`, `<worm-certificates>`, `<worm-replay-view>`. Not passed to `<worm-automations>` (no cost display). Pages use `formatDualCurrency()`, `formatInlineCurrency()`, and `formatInlineCurrencyAlt()` from `usd.ts` for display. `<worm-certificates>` also receives `usdRate` directly.

## Custom Events (Component Communication)

| Event | Source | Target | Detail |
|-------|--------|--------|--------|
| `budget` | `<worm-chat>` | `<worm-app>` | `{ balance, spent }` |
| `done` | `<worm-chat>` | `<worm-app>` | `{ iterations, sats_spent }` |
| `model-change` | `<worm-chat-input>` | `<worm-app>` | `{ model }` — bubbles through chat |
| `send` | `<worm-chat-input>` | `<worm-chat>` | `{ text }` |
| `cancel` | `<worm-chat-input>` | `<worm-chat>` | (none) |
| `new-conversation` | `<worm-chat-header>` | `<worm-chat>` | (none) |
| `range-change` | `<worm-date-range>` | parent | `{ from, to, preset }` — bubbles, composed |

## CSS Custom Properties (Theming)

All components use CSS custom properties from `styles.css` with dark-theme defaults:

| Property | Default | Usage |
|----------|---------|-------|
| `--bg` | `#1b2431` | Page background |
| `--bg-surface` | `#222d3e` | Sidebar, input row |
| `--bg-elevated` | `#273142` | Message bubbles, inputs, cards |
| `--bg-user` | `#4880ff` | User message bubble |
| `--bg-input` | `#323d4e` | Input fields |
| `--border` | `#313d4f` | All borders |
| `--text` | `rgba(255,255,255,0.8)` | Primary text |
| `--text-bright` | `#ffffff` | Headings, user messages |
| `--text-dim` | `rgba(255,255,255,0.5)` | Labels, secondary text |
| `--accent` | `#4880ff` | Links, tool names, focus ring |
| `--error` | `#fd5454` | Error messages, failed tools |
| `--success` | `#00b69b` | Connected dot, successful tools, proofs |
| `--warning` | `#fcbe2d` | Balance display, budget |
| `--mono` | `SF Mono, Cascadia Code, ...` | Monospace font stack |
| `--sans` | `Nunito Sans, system-ui, ...` | Sans-serif font stack |
| `--radius-sm/md/pill` | `8px / 14px / 999px` | Border radius variants |

## Responsive Breakpoints

- **< 640px** (mobile): Sidebar hidden, hamburger drawer, stacked layouts, smaller fonts, 2-column stat grids
- **640px-1023px** (tablet): Sidebar collapses to icon-only (64px), 2-column stat grids, wrapped task cards
- **1024px+** (desktop): Full sidebar visible (220px), 4-column grids, horizontal layouts

## Development

```bash
cd ui
npm install
npm run dev        # Vite dev server with proxy to localhost:8080
npm run build      # Build to ui/dist/
```

Vite proxies API paths (`/chat`, `/health`, `/budget`, `/task`, `/tasks`, `/status`, `/message`, `/agent`, `/conversations`, `/schedules`, `/output`, `/files`, `/decrypt`, `/rates`, `/certificates`, `/compliance`, `/lifecycle`, `/staged`, `/audit`, `/v1`, `/heartbeat`, `/memory`, `/.well-known`) to `http://localhost:8080`. Base path is `/ui/` — in production, axum serves `ui/dist/` via `tower-http::ServeDir` at `GET /ui/*`.

## Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| `lit` | ^3.2.0 | Web component framework (LitElement, html, css, decorators) |
| `marked` | ^15.0.0 | Markdown -> HTML rendering |
| `dompurify` | ^3.2.0 | HTML sanitization for rendered markdown |
| `chart.js` | runtime | Chart rendering: bar, doughnut, line, stacked bar (used by `<worm-bar-chart>`) |
| `@bsv/sdk` | runtime | AuthFetch + WalletClient + Peer (provided by MetaNet Client, not bundled) |

## Notable Implementation Details

- **`proofs.ts` XHR bypass**: Uses a custom `xhrFetch()` helper (XMLHttpRequest-based) that bypasses MetaNet Client's global fetch interceptor. Used for `/output/` and `/decrypt` calls so wallet auth doesn't interfere with raw data retrieval.
- **`proof-chain.ts` visual chain**: Renders proof nodes as colored shapes connected by lines. Green = verified, red = broken, gray = unknown. Uses `PROOF_LABELS` from `labels.ts` for human-readable names and `<worm-tooltip>` for descriptions.
- **`bar-chart.ts` multi-type**: Supports bar, doughnut (65% cutout with legend), line (gradient fill), and stacked bar (multi-series sharing x-axis) chart types. 12-color palette with USD tooltips.
- **`reports.ts` period comparison**: Computes previous period of equal duration and renders up/down percentage change badges. Print stylesheet for PDF export.
- **`compliance.ts` standalone**: Fetches `GET /compliance/report` with optional `?from=&to=` date params. Self-contained with status cards, summary stats, and task table.
- **`date-range.ts` persistence**: Selected preset and custom dates persist to localStorage. Restored on component init.
- **`data-cache.ts` coalescing**: When two components request `/tasks` simultaneously, only one HTTP call fires. Server restart detection invalidates stale cache.
- **`audit.ts` export**: CSV and JSON audit export via `/task/{id}/audit/export?format=csv|json`. Download triggered via temporary anchor element.
- **`budget-panel.ts` compact mode**: Embeddable in dashboard with `compact` attribute — hides service breakdown and heading, showing only gauge bars.
- **`replay-view.ts` fork**: The replay view allows forking a task from any event index via `POST /task/{id}/fork`. SVG cost graph renders cumulative sats over elapsed time with gradient fill.
- **`telemetry-view.ts` Prometheus parser**: Hand-rolled parser for Prometheus text exposition format. Groups metrics by prefix and renders counter/gauge/histogram metric types.

## Related

- `/CLAUDE.md` — Root project docs, architecture overview, HTTP API table
- `src/server/` — Axum server that serves the UI and handles all API endpoints
- `src/session/events.rs` — `StepEvent` enum (Rust side of SSE protocol)
- `src/runner/` — `step()` and `run()` that emit SSE events
- `src/session/conversation.rs` — Multi-turn conversation storage with BRC-60 hash chains
- `src/session/transcript.rs` — JSONL transcript format consumed by audit and chat polling APIs
- `src/config/` — `ParentConfig` with `WORM_PARENT_KEY` (controls auth requirement)
- `src/server/handlers/metrics.rs` — Prometheus metrics endpoint consumed by telemetry view
