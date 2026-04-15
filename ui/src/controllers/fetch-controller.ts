/**
 * Lit ReactiveController for async data loading.
 * Provides data/loading/error states with automatic host updates.
 * Supports optional periodic polling via startPolling/stopPolling.
 */

import type { ReactiveController, ReactiveControllerHost } from 'lit';

export class FetchController<T> implements ReactiveController {
  private host: ReactiveControllerHost;
  private pollTimer: number | null = null;
  private lastFetcher: (() => Promise<T>) | null = null;

  data: T | null = null;
  loading = false;
  error = '';

  constructor(host: ReactiveControllerHost) {
    this.host = host;
    host.addController(this);
  }

  hostConnected(): void {
    // No-op — call fetch() manually when ready.
  }

  hostDisconnected(): void {
    this.stopPolling();
  }

  /** Execute an async fetcher, tracking loading/error state. */
  async fetch(fetcher: () => Promise<T>): Promise<T | null> {
    this.lastFetcher = fetcher;
    this.loading = true;
    this.error = '';
    this.host.requestUpdate();

    try {
      this.data = await fetcher();
      return this.data;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
      return null;
    } finally {
      this.loading = false;
      this.host.requestUpdate();
    }
  }

  /** Start periodic re-fetch at the given interval. Requires a prior fetch() call. */
  startPolling(intervalMs: number): void {
    if (!this.lastFetcher) return;
    this.stopPolling();
    const fetcher = this.lastFetcher;
    this.pollTimer = window.setInterval(() => {
      this.fetch(fetcher);
    }, intervalMs);
  }

  /** Stop periodic re-fetch. */
  stopPolling(): void {
    if (this.pollTimer !== null) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  /** Reset all state and stop polling. */
  reset(): void {
    this.stopPolling();
    this.data = null;
    this.loading = false;
    this.error = '';
    this.lastFetcher = null;
    this.host.requestUpdate();
  }

  get hasData(): boolean {
    return this.data !== null;
  }

  get hasError(): boolean {
    return this.error !== '';
  }
}
