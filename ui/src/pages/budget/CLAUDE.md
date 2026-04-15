# Budget Pages
> Budget visualization and spending export for the agent's multi-tier spending limits.

## Overview

Two Lit web components that display budget consumption (gauge bars + per-service breakdowns) and export spending history to CSV. `WormBudgetPanel` supports a compact mode for embedding in the dashboard, while `WormSpendingExport` fetches detailed spending entries from `/budget/detail` and triggers a browser download. Both components support currency-aware display (sats, USD, or dual) via `CurrencyDisplay` mode.

Budget gauges show 3 base tiers (per-task, per-hour, per-day) plus up to 3 optional tiers (per-week, per-month, lifetime) when configured with limits > 0.

Budget and status polling is **adaptive** — polling only activates (5s interval) when a task is running, and stops when idle. This avoids unnecessary network traffic when the agent is quiescent.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `budget-panel.ts` | 376 | `<dm-budget-panel>` — gauge bars for up to 6 budget tiers, per-service spending bars, alert banners, help tooltips, adaptive active task tracking |
| `spending-export.ts` | 96 | `<dm-spending-export>` — fetches `/budget/detail`, generates CSV, triggers download |

## Key Exports

### `WormBudgetPanel` (`budget-panel.ts`)

Custom element `<dm-budget-panel>` registered via `@customElement('dm-budget-panel')`.

**Properties:**
- `fetchFn` (`typeof fetch`) — injectable fetch function, defaults to `fetch`. Used by `FetchController`.
- `compact` (`boolean`) — when `true`, hides the page title, section headings, per-service breakdown, and export component. Used when embedded in the dashboard.
- `usdRate` (`number`) — BSV/USD exchange rate. Auto-fetched via `fetchBsvUsdRate()` in `connectedCallback()` if not provided.
- `currencyMode` (`CurrencyDisplay`) — controls how amounts are shown (sats, USD, or dual). Initialized from `getCurrencyDisplay()` (localStorage).

**State:**
- `activeTask` (`TaskSummary | null`) — the currently running or most recently started task, polled from `/status` every 5s. Used for task context display.

**Private fields:**
- `ctrl` (`FetchController<BudgetReport>`) — async budget data loader. Polling started/stopped adaptively by `fetchActiveTask()`.
- `statusTimer` (`number | null`) — interval ID for the `/status` poll (5s). Only active when a task is running. Cleared in `disconnectedCallback()`.

**API dependencies:**
- `GET /budget` — returns `BudgetReport` (imported from `shared-types.ts`) with `task_sats`, `hourly_sats`, `daily_sats`, `weekly_sats`, `monthly_sats`, `lifetime_sats`, `total_operations`, per-service `services` map, and `limits` (max + remaining for each tier). Polled every 5s only when a task is running.
- `GET /status` — returns task list. Used by `fetchActiveTask()` to find the running or most recent task. Polled every 5s only when a task is running.

**Lifecycle:**
- `connectedCallback()` — fetches initial budget data (one-shot, no automatic polling), fetches USD rate if not provided, calls `fetchActiveTask()` which conditionally starts adaptive polling.
- `disconnectedCallback()` — clears the status polling timer via `_clearStatusTimer()`.

**Internal logic:**
- `_clearStatusTimer()` — helper to clear the status polling interval.
- `fetchActiveTask()` — polls `/status`, prefers a running task; otherwise picks the most recent with actual spending (`sats_spent > 0`), then falls back to most recent overall. Only triggers re-render if task ID or `sats_spent` changed. **Manages adaptive polling**: starts 5s status + budget polling when a running task is found, stops both when idle. Non-critical — errors are silently ignored.
- `getGauges()` — maps API response to up to 6 `BudgetGaugeData` objects. Three base gauges are always present: "This Task", "Last Hour", "Last 24 Hours". Three optional gauges appear when their limits are configured > 0: "Last 7 Days" (`max_per_week`), "Last 30 Days" (`max_per_month`), "Lifetime" (`max_lifetime`). For "This Task", prefers `activeTask.sats_spent` over `b.task_sats` from the global tracker (which resets to 0 after server restart).
- `getAlertBanner()` — finds the worst gauge percentage across all tiers and returns severity (`exceeded` at 100%, `critical` at 95%, `warning` at 80%) or `null`.
- `renderTaskContext()` — renders active task info (name link, status dot, task ID) below the "This Task" gauge. Compact mode shows a shorter variant (truncated name + dot only). Uses `truncate()` for label length.
- `renderGauge()` — color-coded fill bar with percentage and currency-aware amounts via `formatInlineCurrency()`. Alt currency shown in `title` attribute via `formatInlineCurrencyAlt()`. Help tooltips via `<dm-help>` when `BUDGET_HELP[label]` exists. Adds `pulse` CSS class at 100% (currently renders as solid fill, no animation). Shows task context below "This Task" gauge.
- `renderServiceBars()` — horizontal bars sorted by total_sats descending, colored via `SERVICE_COLORS` cycle, using `friendlyName()` for labels. Currency-aware amounts with alt tooltip and operation count. Shows "No spending recorded yet" when empty.
- `satsToUsd()` — module-level helper for raw sats-to-USD conversion (handles zero/negative rate).

**Color thresholds** (`gaugeColor`):
| % Used | Color |
|--------|-------|
| >= 95 | `--error` (red) |
| >= 80 | `#f97316` (orange) |
| >= 60 | `--warning` (yellow) |
| < 60 | `--accent` |

**Gauge tiers:**
| Label | Source fields | Condition |
|-------|-------------|-----------|
| This Task | `activeTask.sats_spent` or `task_sats` / `max_per_task` | always |
| Last Hour | `hourly_sats` / `max_per_hour` | always |
| Last 24 Hours | `daily_sats` / `max_per_day` | always |
| Last 7 Days | `weekly_sats` / `max_per_week` | `max_per_week > 0` |
| Last 30 Days | `monthly_sats` / `max_per_month` | `max_per_month > 0` |
| Lifetime | `lifetime_sats` / `max_lifetime` | `max_lifetime > 0` |

**Task context CSS:** Styles for `.task-context`, `.task-dot` (running with pulse animation / completed), `.task-id`, `.task-status-text` are defined for displaying active task info alongside budget data.

### `WormSpendingExport` (`spending-export.ts`)

Custom element `<dm-spending-export>` registered via `@customElement('dm-spending-export')`.

**Properties:**
- `fetchFn` (`typeof fetch`) — injectable fetch function.
- `usdRate` (`number`) — for USD column in exported CSV.

**State:**
- `exporting` (`boolean`) — disables button during export.

**API dependency:** `GET /budget/detail` — returns `BudgetDetailResponse` with `entries: SpendingEntry[]`.

**Behavior:** Renders a single "Export CSV" button with a download icon. On click:
1. Fetches `/budget/detail`
2. Builds CSV rows: `Date,Service,Operation,Amount (sats),Amount (USD)`
3. Uses `friendlyName()` for service labels, `formatUsd()` for dollar amounts
4. Properly escapes CSV fields (double-quote wrapping, internal quote doubling)
5. Creates a Blob download named `dolphin-milk-spending-{YYYY-MM-DD}.csv`

## Shared Dependencies

| Import | Source | Used By | Purpose |
|--------|--------|---------|---------|
| `BudgetGaugeData`, `BudgetReport`, `TaskSummary` | `lib/shared-types.ts` | budget-panel | API response + gauge data + active task shapes |
| `BudgetDetailResponse`, `SpendingEntry` | `lib/shared-types.ts` | spending-export | Export data shapes |
| `formatSats`, `truncate`, `parseTimestamp` | `lib/util.ts` | budget-panel | Sats formatting, text truncation, timestamp parsing for task sorting |
| `formatInlineCurrency`, `formatInlineCurrencyAlt`, `fetchBsvUsdRate` | `lib/usd.ts` | budget-panel | Currency-aware display (respects currencyMode) |
| `formatUsd` | `lib/usd.ts` | spending-export | BSV-to-USD conversion for CSV |
| `getCurrencyDisplay`, `CurrencyDisplay` | `lib/storage.ts` | budget-panel | User's currency display preference |
| `SERVICE_COLORS`, `friendlyName` | `lib/constants.ts` | both | Service bar colors and display names |
| `FetchController` | `controllers/fetch-controller.ts` | budget-panel | Async data loading with loading/error states + 5s auto-polling |
| `BUDGET_HELP` | `lib/labels.ts` | budget-panel | Help tooltip text for gauge labels |
| `pageHost`, `centerState`, `pageTitle`, `sectionTitle`, `gaugeBar`, `alertBanner`, `stateFeedback` | `lib/shared-styles.ts` | budget-panel | Shared CSS mixins + state feedback styles |
| `renderLoading`, `renderError`, `renderEmpty` | `lib/shared-styles.ts` | budget-panel | Render helpers for loading/error/empty states |

## Usage

Standalone page (routed at `#budget` in `app.ts`):
```html
<dm-budget-panel .usdRate=${rate}></dm-budget-panel>
```
Note: `<dm-spending-export>` is rendered internally by `<dm-budget-panel>` in non-compact mode (imported via `./spending-export.js`).

Compact mode for dashboard embedding:
```html
<dm-budget-panel compact .usdRate=${rate}></dm-budget-panel>
```

## Related

- `../../lib/shared-types.ts` — `BudgetGaugeData`, `BudgetReport`, `TaskSummary`, `BudgetDetailResponse`, `SpendingEntry` interfaces
- `../../lib/shared-styles.ts` — `gaugeBar` and `alertBanner` CSS used by the panel
- `../../lib/constants.ts` — `SERVICE_COLORS`, `SERVICE_NAMES`, `friendlyName()`
- `../../lib/usd.ts` — `formatInlineCurrency()`, `formatInlineCurrencyAlt()`, `fetchBsvUsdRate()`, `formatUsd()`
- `../../lib/util.ts` — `parseTimestamp()` for task sorting, `truncate()`, `formatSats()`
- `../../lib/storage.ts` — `getCurrencyDisplay()` for user currency preference
- `../../lib/labels.ts` — `BUDGET_HELP` tooltip text for gauge labels
- `../../controllers/fetch-controller.ts` — `FetchController` reactive data loader with polling
- `../dashboard.ts` — embeds `<dm-budget-panel compact>` in the overview page
- `../../CLAUDE.md` — parent UI documentation
- `/CLAUDE.md` — root project documentation
