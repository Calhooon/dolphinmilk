# ui/src/components
> Reusable Lit web components shared across pages — charts, chat messages, proof visualization, tool cards, date pickers, tooltips, notifications, search, sharing, and onboarding.

## Overview

Sixteen self-contained Lit 3 custom elements used by multiple page components. All follow the project's dark theme via CSS custom properties (`--bg-elevated`, `--border`, `--text-dim`, etc. from `styles.css`). No shared base class — each extends `LitElement` directly with scoped styles. Components are registered via `@customElement` decorator and imported by tag name in consuming pages.

## Components

| Component | Tag | Purpose | Lines |
|-----------|-----|---------|-------|
| `bar-chart.ts` | `<worm-bar-chart>` | Chart.js wrapper — bar, doughnut, line, and stacked bar charts with currency-aware tooltips | 504 |
| `budget-strip.ts` | `<worm-budget-strip>` | Compact progress + budget strip for chat view — iteration gauge, task sats, hourly/daily totals, balance | 253 |
| `date-range.ts` | `<worm-date-range>` | Date range picker with preset periods (today/7d/30d/MTD/QTD/YTD) and custom range | 228 |
| `message.ts` | `<worm-message>` | Chat bubble with markdown rendering, DOMPurify sanitization, image lightbox, cost stamp, and pending state | 321 |
| `notification-center.ts` | `<worm-notification-center>` | Bell-icon notification panel with grouped alerts derived from task/budget state changes | 548 |
| `onboarding.ts` | `<worm-onboarding>` | First-run overlay with wallet status, funding guide, capability pills, and suggested prompts | 695 |
| `proof-chain.ts` | `<worm-proof-chain>` | Visual proof chain — clickable node grid with connectors, tooltips, and batch verify | 386 |
| `proof-inline.ts` | `<worm-proof-inline>` | Expandable single proof card with hash verification and WhatsOnChain link | 265 |
| `receipt-table.ts` | `<worm-receipt-table>` | BEEF payment receipt cost breakdown table with currency-aware totals | 136 |
| `search-overlay.ts` | `<worm-search-overlay>` | Full-screen Cmd+K search across conversations, memory, tasks, and audit | 711 |
| `share-menu.ts` | `<worm-share-menu>` | Export dropdown — copy link, JSON/CSV/Markdown export for tasks and conversations | 315 |
| `snackbar.ts` | `<worm-snackbar>` | Fixed-position toast notification with auto-dismiss | 106 |
| `sparkline.ts` | `<worm-sparkline>` | Inline SVG sparkline for trend visualization | 57 |
| `stat-ticker.ts` | `<worm-stat-ticker>` | Animated number counter with eased interpolation and scientific notation support | 113 |
| `tool-card.ts` | `<worm-tool-card>` | Tool call display with smart preview, rich artifact rendering, cost badge, receipt section, and expandable arguments | 734 |
| `tooltip.ts` | `<worm-tooltip>`, `<worm-help>` | Hover tooltip with fixed positioning and contextual help icon | 124 |

## Component Details

### WormBarChart (`bar-chart.ts`)
- **Props:** `data: BarData[]`, `stackedData: StackedBarData | null`, `maxBars: number` (default 14), `height: string`, `orientation: 'vertical' | 'horizontal'`, `chartType: 'bar' | 'doughnut' | 'line' | 'stacked-bar'`, `centerLabel: string`, `usdRate: number`, `currencyMode: CurrencyDisplay` (default `'usd-first'`)
- **Exports:** `BarData` interface (`{ label: string; value: number; color?: string }`), `StackedBarData` interface (`{ labels: string[]; series: { label: string; values: number[]; color?: string }[] }`)
- **Behavior:** Creates a Chart.js instance on a `<canvas>` element. Destroys and recreates on every `updated()` via `requestAnimationFrame`. Shows "No data" placeholder when empty. Doughnut chart has 65% cutout with bottom legend. Line chart fills area with gradient. Stacked bar chart uses `StackedBarData` with multiple series sharing x-axis labels, bottom legend, and tooltip footer showing totals. All tooltips use `formatInlineCurrency()` for currency-aware display (respects `currencyMode`). Uses a 12-color `PALETTE` for auto-coloring segments. Auto-scrolls when >14 bars.
- **Lifecycle:** `destroyChart()` on `disconnectedCallback` to prevent Chart.js memory leaks.
- **Used by:** Budget panel, dashboard, audit views, reports.

### WormBudgetStrip (`budget-strip.ts`)
- **Tag:** `<worm-budget-strip>`
- **Props:** `fetchFn: typeof fetch` (default `fetch.bind(window)`), `active: boolean`, `iteration: number`, `taskSats: number`, `currencyMode: CurrencyDisplay` (default `'usd-first'`), `usdRate: number` (default 0)
- **State:** `data: BudgetReport | null`
- **Imports:** `BudgetReport` from `lib/shared-types.ts`, `CurrencyDisplay` from `lib/storage.ts`, `formatInlineCurrency` from `lib/usd.ts`, `./tooltip.js`
- **Behavior:** Compact 30px-tall strip showing per-task budget consumption. Progress bar fills based on `taskSats / limits.max_per_task` with color coding: accent (<60%), yellow (60-80%), orange (80-95%), red (95%+) via `gaugeColor()`. Displays step number, task sats vs limit, hourly/daily totals (wrapped in `<worm-tooltip>` with descriptive text), and wallet balance. Currency-aware formatting via `_fmt()` — uses `formatInlineCurrency()` when `usdRate > 0` and `currencyMode === 'usd-first'`, otherwise falls back to `formatCompactSats()` (e.g. `1.5M`, `23.4K`). Clicking navigates to `#budget`. Polls `GET /budget` every 5s when `active=true`; stops polling when inactive. Returns `nothing` when no task is running and no data exists.
- **Responsive:** At 640px, reduces padding/gap/font-size and hides hourly/daily metrics (`.hide-mobile`).
- **Used by:** Chat view (`pages/chat/`).

### WormDateRange (`date-range.ts`)
- **Tag:** `<worm-date-range>`
- **Props:** `value: DateRange` (restored from localStorage on init)
- **Exports:** `DatePreset` type (`'today' | '7d' | '30d' | 'mtd' | 'qtd' | 'ytd' | 'custom'`), `DateRange` interface (`{ from: Date; to: Date; preset: DatePreset }`), `defaultRange(preset, customFrom?, customTo?)` helper function
- **Behavior:** Renders preset pill buttons (Today, 7 Days, 30 Days, MTD, QTD, YTD, Custom). Active preset highlighted in accent blue. When "Custom" is selected, shows two date inputs for from/to range. Persists selection to localStorage via `getDateRange()`/`setDateRange()` from `lib/storage.ts`. Dispatches `range-change` CustomEvent with `{ from, to, preset }` detail (bubbles, composed).
- **Used by:** Budget panel, reports, spending views.

### WormMessage (`message.ts`)
- **Props:** `role: string` ('user' | 'assistant' | 'error' | 'system'), `content: string`, `timestamp: number` (unix seconds), `pending: boolean`, `satsCost: number` (default 0), `currencyMode: CurrencyDisplay` (default `'usd-first'`), `usdRate: number` (default 0), `attachments: Attachment[]`
- **State:** `_lightboxSrc: string | null`
- **Behavior:** User messages render as plain text. Assistant/error/system messages are parsed through `marked` → `DOMPurify.sanitize()` → `unsafeHTML`. Images get lazy loading and are wrapped in `<a target="_blank">` links (unless already linked). System messages render centered and dimmed. When `pending=true`, bubble shows reduced opacity with a spinning indicator. Markdown content includes styled headings (h1–h3 with differentiated sizes), tables (collapsed borders, themed header), links (accent colored), and media (images, video, audio with rounded corners and max-width).
- **Lightbox:** Clicking an image opens a full-screen overlay (`_lightboxSrc` state). Overlay renders the image at full resolution with a close button. Pressing Escape dismisses the lightbox.
- **Cost stamp:** Assistant messages with `satsCost > 0` display a footer row with timestamp and a monospace cost badge. Three color tiers via `costTier()`: green (`tier-low`, <10K sats), yellow (`tier-mid`, 10K–50K), red (`tier-high`, >50K). Formatted via `formatInlineCurrency()`, with alt currency on hover title via `formatInlineCurrencyAlt()`.
- **Security:** DOMPurify allowlists `img`, `video`, `source`, `audio` tags with `src`, `alt`, `controls`, `poster`, `width`, `height`, `loading`, `type` attributes. All other HTML is stripped.
- **Responsive:** At 639px, max-width increases to 92%, padding and font-size reduce for mobile.
- **Used by:** Chat view (`pages/chat/`).

### WormNotificationCenter (`notification-center.ts`)
- **Tag:** `<worm-notification-center>`
- **Props:** `fetchFn: typeof fetch`, `open: boolean`, `currencyMode: CurrencyDisplay`
- **State:** `notifications: Notification[]`, `usdRate: number`
- **Types:** `Notification` (id, type, title, body, timestamp, read, link), `NotificationState`
- **Behavior:** Fixed bell panel (bottom-left, 360px wide) with notification list grouped by day. Derives notifications from task/budget state changes via polling. localStorage persistence (max 50 notifications). "Mark all read" and "Clear all" actions. Dispatches a badge update event on change so the app shell can show an unread count. Notifications link to relevant views (e.g., task detail, budget). Currency-aware body text via `formatInlineCurrency()`.
- **Used by:** App shell (`app.ts`).

### WormOnboarding (`onboarding.ts`)
- **Tag:** `<worm-onboarding>`
- **Props:** `fetchFn: typeof fetch`, `currencyMode: CurrencyDisplay`
- **State:** `agent: AgentInfo | null`, `usdRate: number`, `visible: boolean`, `fundingAddress: string`, `copied: boolean`, `checking: boolean`
- **Behavior:** First-run overlay shown until dismissed via `markOnboardingSeen()`. Features: hero section, wallet status card (fetches `GET /agent`), funding guide with BSV receive address (fetches `GET /wallet/address`) and copy-to-clipboard when balance is zero, "Check for payment" button that polls `GET /wallet/check-funding`, 6 capability pills, 4 suggested prompts with estimated costs, and footer links. Dispatches `send-prompt` event (to trigger a chat message) and `dismiss` event. Currency-aware via `formatDualCurrency()` and `fetchBsvUsdRate()`.
- **Used by:** Chat view (shown on first visit).

### WormProofChain (`proof-chain.ts`)
- **Props:** `proofs: ProofDetail[]`, `currencyMode: CurrencyDisplay` (default `'usd-first'`), `usdRate: number` (default 0)
- **State:** `selectedNode`, `hoveredNode`, `verifyResults` map, `verifyingAll` flag
- **Behavior:** Sorts proofs by timestamp and renders a flex-wrapped grid of colored shape nodes connected by lines. Each node is styled by `TYPE_COLORS`, `TYPE_SHAPES` from `lib/constants.ts`. Legend uses `PROOF_LABELS` from `lib/labels.ts` with `<worm-tooltip>` for descriptions, falling back to `PROOF_TYPE_LABELS` for types without labels. Hover shows tooltip with type, hash prefix, and cost. Click selects a node and renders a `<worm-proof-inline>` detail panel below (passing `currencyMode` and `usdRate`). "Verify All" button calls `GET /task/{id}/proofs/verify` (extracts task ID from URL hash). Connector lines are colored green (verified), red (broken chain), or gray (unknown). Respects `prefers-reduced-motion` media query.
- **Chain integrity:** Checks `prev_hash` linkage across sorted proofs. Displays a status banner: "Chain Intact", "Chain Broken", or partial check count.
- **Used by:** Proof viewer (`pages/proofs/`), audit views.

### WormProofInline (`proof-inline.ts`)
- **Props:** `proof: ProofDetail`, `verifyStatus: 'unverified' | 'verified' | 'mismatch'`, `currencyMode: CurrencyDisplay` (default `'usd-first'`), `usdRate: number` (default 0)
- **State:** `expanded`, `verifying`, `copied`
- **Behavior:** Collapsed view shows type badge (colored by `TYPE_COLORS`, labeled via `PROOF_LABELS` from `lib/labels.ts` with `<worm-tooltip>` descriptions, falling back to `PROOF_TYPE_LABELS`), truncated txid, and verify status badge. Expanded view shows full txid, hash, iteration, cost (formatted via `formatInlineCurrency()`), prev_hash (with italic field hint), and action buttons: Copy txid, Verify hash (client-side via `computeProofHash` from `lib/proof-utils.ts`), and WhatsOnChain external link.
- **Used by:** `proof-chain.ts` (as detail panel for selected node), audit timeline.

### WormReceiptTable (`receipt-table.ts`)
- **Props:** `receipts: ReceiptDetail[]`, `totalPaid: number`, `totalRefunded: number`, `usdRate: number`, `currencyMode: CurrencyDisplay`
- **Behavior:** Renders a table with columns: Iter, Model, Tokens, Paid, Refunded, Effective, USD. USD column uses `formatUsd()` from `lib/usd.ts`. Summary row shows total paid (yellow), refunded (green), and net cost with currency-aware formatting via `formatInlineCurrency()` / `formatInlineCurrencyAlt()` from `lib/usd.ts`. Title attributes show alt currency (hover for sats when showing USD, or vice versa). `currencyMode` defaults from `getCurrencyDisplay()` in `lib/storage.ts`. Returns `nothing` when receipts array is empty.
- **Used by:** Audit view (`pages/audit/`), proof viewer.

### WormSearchOverlay (`search-overlay.ts`)
- **Tag:** `<worm-search-overlay>`
- **Props:** `open: boolean`, `fetchFn: typeof fetch`
- **State:** `query: string`, `results: SearchResultItem[]`, `loading: boolean`, `selectedIndex: number`, `recentSearches: string[]`
- **Types:** `SearchResultItem` (type, icon, title, subtitle, time, hash)
- **Behavior:** Full-screen modal search dialog triggered by Cmd+K / Ctrl+K. Searches 4 categories in parallel: conversations (`GET /conversations`), memory (`GET /memory/search`), tasks (`GET /tasks`), and audit events. Results grouped by category with flat indexing for keyboard selection. Arrow up/down navigates, Enter selects (navigates to item's hash route), Escape closes. Recent searches persisted to localStorage (max 5). Category icons and color-coded result types. Shows keyboard hints in footer.
- **Used by:** App shell (`app.ts`).

### WormShareMenu (`share-menu.ts`)
- **Tag:** `<worm-share-menu>`
- **Props:** `taskId: string`, `sessionId: string`, `open: boolean`, `fetchFn: typeof fetch`
- **State:** `copied: boolean`, `exporting: string | null`
- **Behavior:** Absolutely-positioned dropdown menu with 4 actions: **Copy link** (works for both conversations and tasks, writes URL to clipboard), **JSON export** (calls `GET /task/{id}/audit/export?format=json`), **CSV export** (calls `GET /task/{id}/audit/export?format=csv`), **Markdown export** (formats messages with role labels and tool calls as pretty-printed JSON, creates blob download). Uses temporary anchor element for file downloads. Shows `<worm-snackbar>` on error.
- **Used by:** Chat header, audit views.

### WormSnackbar (`snackbar.ts`)
- **Props:** `type: 'info' | 'warning' | 'error' | 'success'`, `message: string`, `duration: number` (default 5000ms)
- **Behavior:** Fixed-position toast at bottom center. Auto-dismisses after `duration` ms (set 0 to disable). Colored left border by type. Close button and slide-up animation. Dispatches `dismiss` CustomEvent and calls `this.remove()`.
- **Accessibility:** `role="alert"`, `aria-live="polite"`, `aria-atomic="true"`.
- **Usage:** Create programmatically — `document.body.appendChild(document.createElement('worm-snackbar'))` with properties set.

### WormSparkline (`sparkline.ts`)
- **Tag:** `<worm-sparkline>`
- **Props:** `data: number[]`, `color: string` (default `var(--accent, #E8734A)`), `width: string` (default `80px`), `height: string` (default `24px`)
- **Behavior:** Renders a pure SVG sparkline (no Chart.js dependency). Requires at least 2 data points. Computes min/max range for vertical scaling with 1px padding. Draws a gradient-filled area and a 1.5px stroke line with rounded caps. Respects `prefers-reduced-motion` media query.
- **Used by:** Dashboard hero cards for at-a-glance spending trends.

### WormStatTicker (`stat-ticker.ts`)
- **Props:** `value: string`, `prefix: string`, `suffix: string`, `label: string`, `sublabel: string`, `source: string`, `duration: number` (default 1500ms), `shouldAnimate: boolean` (default true)
- **State:** `displayValue: string`
- **Behavior:** Animates from 0 to target value using `requestAnimationFrame` with cubic ease-out. Respects decimal places from the input `value` string. Non-numeric values display immediately without animation. Handles scientific notation (e.g., `"1.5e7"`) via `normalizeValue()` which converts to fixed notation before animation.
- **Used by:** Demo overlay, dashboard.

### WormToolCard (`tool-card.ts`)
- **Props:** `name: string`, `callId: string`, `arguments: string`, `output: string`, `success: boolean`, `pending: boolean`, `compact: boolean`, `satsCost: number` (default 0), `durationMs: number` (default 0), `currencyMode: CurrencyDisplay` (default `'usd-first'`), `usdRate: number` (default 0)
- **State:** `expanded`, `copied`, `codeExpanded`, `imgError`
- **Smart preview:** Collapsed header shows a one-line preview extracted from arguments JSON via `getPreview()`. Scans `PREVIEW_KEYS` (`prompt`, `query`, `url`, `endpoint`, `message`, `text`, `title`, `description`, `action`, `content`) in priority order, truncated to 60 chars with ellipsis.
- **Output summary:** `getOutputSummary()` pattern-matches output JSON to produce brief summaries for common tools — inbox messages, certificate operations, balance checks, and result counts.
- **Cost display:** When `satsCost > 0`, header right side shows primary cost (bold yellow, via `formatInlineCurrency()`) and secondary cost (dim, via `formatInlineCurrencyAlt()`).
- **Rich artifact previews:** `getArtifactType()` detects 4 artifact types from tool name and output, rendered by `renderRichPreview()`:
  - **Image** (`generate_image`): Inline `<img>` with "Open full size" link and "Copy URL" button. Falls back to text on `imgError`.
  - **File write** (`file_write`): Code block with filename header, line count badge, and expand/collapse for long content.
  - **Screenshot** (`browser` with screenshot action): Filepath display.
  - **NanoStore upload** (`upload_to_nanostore`): UHRP badge with the upload URL.
- **Receipt section:** Expanded view includes a receipt grid (shown when `satsCost > 0`) displaying service, paid, refund, net, and duration. Values extracted from output JSON fields (`sats_paid`, `sats_refunded`, `sats_effective`, `duration_ms`, `service`) when available, falling back to the `satsCost`/`durationMs` props.
- **Behavior:** Header shows status dot (green/red/pulsing blue), tool name in monospace, smart preview, and output summary. Rich preview rendered below header when applicable. Output section always visible below header when present (JSON pretty-printed). Arguments and receipt sections shown only when expanded. Copy button for output. Error state shows red border. Failed output text rendered in red.
- **Used by:** Chat view (inline tool results), audit timeline.

### WormTooltip & WormHelp (`tooltip.ts`)
- **`<worm-tooltip>`** — Wraps slotted content with a hover/focus tooltip. Props: `text: string`, `position: 'top' | 'bottom'` (default `'top'`). State: `visible`, `tipTop`, `tipLeft`. Uses `position: fixed` with dynamic positioning via `getBoundingClientRect()` — calculates trigger element position and flips top↔bottom when tooltip would overflow viewport edges. Horizontal clamping keeps tooltip within 8px of viewport edges. Tooltip appears on `mouseenter`/`focusin` with 0.15s opacity transition. Max width 280px, dark background with border, box shadow, `z-index: 10000`.
- **`<worm-help>`** — Small "?" circle icon that wraps `<worm-tooltip>`. Props: `text: string`. Renders as a 16px round badge with hover highlight. Convenience element for adding contextual help to labels and headings.
- **Used by:** proof-chain (legend tooltips), proof-inline (type badge tooltips), budget panel, budget-strip, reports.

## Patterns

- **Expand/collapse:** `proof-inline`, `tool-card`, and `search-overlay` use `@state() expanded` with chevron rotation (`transform: rotate(90deg)`) and conditional template rendering.
- **Copy to clipboard:** `proof-inline`, `tool-card`, `share-menu`, and `onboarding` use `navigator.clipboard.writeText()` with a `copied` state that resets via `setTimeout` (2s).
- **JSON formatting:** `tool-card.formatJson()` attempts `JSON.parse` → `JSON.stringify(_, null, 2)`, falls back to raw string.
- **Empty states:** Components return `nothing` (from `lit`) or a `<div class="empty">` message when data is absent.
- **CSS custom properties:** All components use the theme variables defined in `styles.css` (e.g., `--bg-elevated`, `--border`, `--text-dim`, `--accent`, `--success`, `--error`, `--warning`, `--mono`, `--sans`, `--radius-md`, `--radius-sm`).
- **No shared styles import:** Unlike pages, these components define all styles inline rather than importing from `lib/shared-styles.ts`.
- **Reduced motion:** `sparkline` and `proof-chain` respect `prefers-reduced-motion` media query to disable animations.
- **Currency awareness:** `bar-chart`, `budget-strip`, `message`, `notification-center`, `onboarding`, `proof-chain`, `proof-inline`, `receipt-table`, and `tool-card` all support dual sats/USD display via `usdRate` and `currencyMode` props. Formatting uses `formatInlineCurrency()` and `formatInlineCurrencyAlt()` from `lib/usd.ts`, adapting display based on the active currency mode.
- **Keyboard shortcuts:** `search-overlay` responds to Cmd+K / Ctrl+K globally. Arrow keys, Enter, and Escape for navigation within the overlay.
- **localStorage persistence:** `date-range` (selected preset), `search-overlay` (recent searches), `notification-center` (notification list).
- **Polling:** `budget-strip` (5s when active), `notification-center` (task/budget state), `onboarding` (check-for-payment).

## Dependencies

| Import | Used by | Source |
|--------|---------|--------|
| `ProofDetail` | proof-chain, proof-inline | `lib/shared-types.ts` |
| `ReceiptDetail` | receipt-table | `lib/shared-types.ts` |
| `BudgetReport` | budget-strip | `lib/shared-types.ts` |
| `TaskSummary` | notification-center, search-overlay | `lib/shared-types.ts` |
| `AgentInfo` | onboarding | `lib/shared-types.ts` |
| `ConversationSummary`, `MemorySearchResult` | search-overlay | `lib/shared-types.ts` |
| `BarData`, `StackedBarData` | bar-chart (exported) | Defined locally |
| `DatePreset`, `DateRange` | date-range (exported) | Defined locally |
| `CurrencyDisplay` | bar-chart, budget-strip, message, notification-center, onboarding, proof-chain, proof-inline, receipt-table, tool-card | `lib/storage.ts` |
| `TYPE_COLORS` | proof-chain, proof-inline | `lib/constants.ts` |
| `TYPE_SHAPES` | proof-chain | `lib/constants.ts` |
| `PROOF_TYPE_LABELS` | proof-chain, proof-inline | `lib/constants.ts` |
| `PROOF_LABELS` | proof-chain, proof-inline | `lib/labels.ts` |
| `computeProofHash` | proof-inline | `lib/proof-utils.ts` |
| `formatInlineCurrency` | bar-chart, budget-strip, message, notification-center, proof-chain, proof-inline, receipt-table, tool-card | `lib/usd.ts` |
| `formatInlineCurrencyAlt` | message, receipt-table, tool-card | `lib/usd.ts` |
| `formatDualCurrency` | onboarding | `lib/usd.ts` |
| `formatUsd` | receipt-table | `lib/usd.ts` |
| `fetchBsvUsdRate` | onboarding | `lib/usd.ts` |
| `formatRelativeTime`, `truncate`, `parseTimestamp` | notification-center, search-overlay | `lib/util.ts` |
| `formatSats` | receipt-table | `lib/util.ts` |
| `getCurrencyDisplay` | receipt-table | `lib/storage.ts` |
| `getDateRange`, `setDateRange`, `StoredDateRange` | date-range | `lib/storage.ts` |
| `<worm-tooltip>` (side-effect import) | budget-strip | `./tooltip.js` |
| `marked` | message | External package |
| `DOMPurify` | message | External package |
| `Chart` (chart.js) | bar-chart | External package |

## Related

- `../lib/CLAUDE.md` — Shared utilities, types, and styles
- `../pages/CLAUDE.md` — Page components that consume these
- `../pages/audit/CLAUDE.md` — Audit views using proof-chain, proof-inline, receipt-table, tool-card
- `../pages/chat/CLAUDE.md` — Chat view using message, tool-card, snackbar
