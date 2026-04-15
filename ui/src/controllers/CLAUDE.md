# ui/src/controllers
> Lit ReactiveControllers for async data loading and interval-based polling.

## Overview

Two generic controllers that encapsulate common async patterns for Lit web components. Both implement the `ReactiveController` interface, auto-registering with the host via `addController()` and triggering re-renders via `requestUpdate()`.

`FetchController` is the primary data-loading primitive across the UI — used by 14 page components for one-shot fetches and optional periodic polling. `PollingController` provides a standalone interval-based callback with pause/resume lifecycle — available but not currently consumed (pages that need polling use `FetchController.startPolling()` or `TranscriptPoller` from `lib/transcript-poller.ts`).

## Controllers

| Controller | File | Lines | Purpose | Used By |
|------------|------|-------|---------|---------|
| `FetchController<T>` | `fetch-controller.ts` | 85 | Async data loading with `data`/`loading`/`error` state + optional periodic polling | 14 page components |
| `PollingController` | `polling-controller.ts` | 79 | Interval-based callback with start/stop/pause/resume | Available but unused |

## FetchController\<T\>

Generic controller for async operations with optional periodic re-fetch. Tracks three reactive states: `data: T | null`, `loading: boolean`, `error: string`.

**Lifecycle:**
- `hostConnected()` — No-op. Call `fetch()` manually (typically in `connectedCallback` or `updated`).
- `hostDisconnected()` — Automatically calls `stopPolling()` to clean up timers.

**API:**
- `fetch(fetcher: () => Promise<T>): Promise<T | null>` — Execute async function, set loading/error/data, trigger host update. Stores `fetcher` as `lastFetcher` for polling.
- `startPolling(intervalMs: number): void` — Begin periodic re-fetch using the last `fetch()` call's fetcher. Requires a prior `fetch()` call (no-op without one). Stops any existing poll before starting.
- `stopPolling(): void` — Clear the polling interval timer.
- `reset(): void` — Stop polling and clear all state (`data`, `loading`, `error`, `lastFetcher`) to initial values.
- `hasData: boolean` — `data !== null`
- `hasError: boolean` — `error !== ''`

**Usage pattern (one-shot):**
```ts
import { FetchController } from '../controllers/fetch-controller.js';

class MyPage extends LitElement {
  private ctrl = new FetchController<MyData>(this);

  connectedCallback() {
    super.connectedCallback();
    this.ctrl.fetch(() => fetch('/api/data').then(r => r.json()));
  }

  render() {
    if (this.ctrl.loading) return html`<p>Loading...</p>`;
    if (this.ctrl.hasError) return html`<p>${this.ctrl.error}</p>`;
    return html`<pre>${JSON.stringify(this.ctrl.data)}</pre>`;
  }
}
```

**Usage pattern (with polling):**
```ts
connectedCallback() {
  super.connectedCallback();
  this.ctrl.fetch(() => fetch('/api/data').then(r => r.json()))
    .then(() => this.ctrl.startPolling(5000));
}
```

**Consumers (14 pages):**
- `pages/agent.ts` — `FetchController<AgentInfo>`
- `pages/artifacts.ts` — `FetchController<ArtifactsResponse>`
- `pages/automations.ts` — `FetchController<Schedule[]>` + `startPolling(30000)`
- `pages/budget/budget-panel.ts` — `FetchController<BudgetReport>`
- `pages/certificates.ts` — `FetchController<CertificateStatus>`
- `pages/compliance.ts` — `FetchController<ComplianceReport>`
- `pages/conversations.ts` — `FetchController<ConversationSummary[]>`
- `pages/dashboard.ts` — `FetchController<DashboardData>`
- `pages/memory.ts` — `FetchController<MemoryListResponse>`
- `pages/proofs/proofs.ts` — `FetchController<ProofsPageData>`
- `pages/reports.ts` — `FetchController<ReportsData>`
- `pages/task-list.ts` — `FetchController<TaskSummary[]>`
- `pages/audit/audit.ts` — `FetchController<AuditPageData>`
- `pages/audit/audit-global.ts` — `FetchController<AuditGlobalData>`

**Skipped by (1 page)** — with documented reason:
- `pages/knowledge.ts` — non-critical stats-only loading from two endpoints, uses manual fetch

## PollingController

Wraps `setInterval` with Lit lifecycle integration. Auto-starts on `hostConnected`, auto-stops on `hostDisconnected`.

**Constructor:** `new PollingController(host, callback, interval = 5000)`

**API:**
- `start(): void` — Begin interval (no-op if already running)
- `stop(): void` — Clear interval and reset pause state
- `pause(): void` — Skip callback invocations without clearing timer
- `resume(): void` — Resume after pause
- `setInterval(ms): void` — Change interval (restarts timer if running)
- `trigger(): void` — Fire callback immediately outside the interval
- `paused: boolean` — Current pause state
- `running: boolean` — `timer !== null && !paused`

**Note:** Not currently imported by any page component. `FetchController.startPolling()` covers the common case of periodic re-fetch. `PollingController` remains useful for non-fetch callbacks (e.g., DOM animations, heartbeat pings) but hasn't been needed yet.

## FetchController vs PollingController

| Feature | FetchController | PollingController |
|---------|----------------|-------------------|
| Data state tracking | Yes (`data`/`loading`/`error`) | No |
| Auto-start on connect | No (manual `fetch()`) | Yes |
| Polling | Optional via `startPolling()` | Always (core purpose) |
| Pause/resume | No | Yes |
| Async callback | Yes (`Promise<T>`) | Yes (`void \| Promise<void>`) |
| Auto-cleanup on disconnect | Yes (`stopPolling()`) | Yes (`stop()`) |

In practice, `FetchController` has subsumed the most common `PollingController` use case (periodic API fetching). `PollingController` adds pause/resume semantics and works with arbitrary callbacks, but no current component needs those features.

## Error Handling

`FetchController` catches all errors from the fetcher function and stores the message string in `this.error`. The host component is responsible for rendering error state — no global error handling or retry logic. Components that need retry implement it themselves (e.g., `queue.ts` adapts its polling interval based on task state).

## Related

- `../lib/transcript-poller.ts` — Specialized poller for `/task/{id}/events` transcript streaming (used by chat)
- `../lib/data-cache.ts` — TTL-based fetch cache with promise coalescing (used alongside FetchController)
- `../lib/wallet-auth.ts` — BRC-31 AuthFetch used in fetcher functions passed to FetchController
- `../lib/CLAUDE.md` — Shared utilities documentation
- `../pages/CLAUDE.md` — Page components that consume these controllers
- `../components/CLAUDE.md` — Reusable UI components
