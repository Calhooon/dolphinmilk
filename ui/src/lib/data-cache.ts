/**
 * TTL-based fetch cache with promise coalescing.
 * Prevents duplicate HTTP calls when multiple components request the same data.
 *
 * Error handling contract:
 * - All methods return a graceful fallback on HTTP errors (non-2xx status):
 *   - fetchTasks(), fetchConversations(), fetchSchedules(): return []
 *   - fetchBudget(): returns null
 *   - fetchAgent(), fetchBudgetDetail(), fetchCertificates(): return null
 * - Network errors (fetch rejects) are caught and return the same fallback.
 */

import type {
  TaskSummary,
  AgentInfo,
  BudgetReport,
  BudgetDetailResponse,
  Schedule,
  CertificateStatus,
} from './shared-types.js';
import type { ConversationSummary } from './shared-types.js';

interface CacheEntry<T> {
  data: T;
  ts: number;
}

export class DataCache {
  private cache = new Map<string, CacheEntry<any>>();
  private inflight = new Map<string, Promise<any>>();
  private fetchFn: typeof fetch;
  /** Tracks the oldest cache entry timestamp for server-restart detection. */
  private cacheCreatedAt: number = Date.now();

  constructor(fetchFn: typeof fetch) {
    this.fetchFn = fetchFn;
  }

  /** Update the fetch function (e.g. after re-auth). */
  setFetchFn(fn: typeof fetch) {
    this.fetchFn = fn;
  }

  /**
   * Check server uptime from /health and invalidate cache if the server
   * has been restarted more recently than our oldest cache entry.
   * Call this periodically (e.g. on visibility change or before critical fetches).
   */
  async checkServerRestart(): Promise<void> {
    try {
      const res = await this.fetchFn('/health');
      if (!res.ok) return;
      const data = await res.json();
      const uptimeSecs = data.uptime_secs;
      if (typeof uptimeSecs !== 'number') return;
      const serverStartedAt = Date.now() - uptimeSecs * 1000;
      // If the server started after our cache was created, it was restarted
      if (serverStartedAt > this.cacheCreatedAt && this.cache.size > 0) {
        this.invalidate();
        this.cacheCreatedAt = Date.now();
      }
    } catch {
      // Best effort — don't break on network errors
    }
  }

  /** Fetch tasks list. Returns [] on error. */
  async fetchTasks(ttl = 15000): Promise<TaskSummary[]> {
    return this.get('tasks', ttl, async () => {
      try {
        const res = await this.fetchFn('/tasks');
        if (!res.ok) return [];
        const data = await res.json();
        return data.tasks ?? [];
      } catch {
        return [];
      }
    });
  }

  /** Fetch agent info. Returns null on error. */
  async fetchAgent(ttl = 30000): Promise<AgentInfo | null> {
    return this.get('agent', ttl, async () => {
      try {
        const res = await this.fetchFn('/agent');
        if (!res.ok) return null;
        return res.json();
      } catch {
        return null;
      }
    });
  }

  /** Fetch budget report. Returns null on error. */
  async fetchBudget(ttl = 10000): Promise<BudgetReport | null> {
    return this.get('budget', ttl, async () => {
      try {
        const res = await this.fetchFn('/budget');
        if (!res.ok) return null;
        return res.json();
      } catch {
        return null;
      }
    });
  }

  /** Fetch detailed budget breakdown. Returns null on error. */
  async fetchBudgetDetail(ttl = 30000): Promise<BudgetDetailResponse | null> {
    return this.get('budget-detail', ttl, async () => {
      try {
        const res = await this.fetchFn('/budget/detail');
        if (!res.ok) return null;
        return res.json();
      } catch {
        return null;
      }
    });
  }

  /** Fetch conversations list. Returns [] on error. */
  async fetchConversations(ttl = 15000): Promise<ConversationSummary[]> {
    return this.get('conversations', ttl, async () => {
      try {
        const res = await this.fetchFn('/conversations');
        if (!res.ok) return [];
        const data = await res.json();
        return Array.isArray(data) ? data : [];
      } catch {
        return [];
      }
    });
  }

  /** Fetch schedules list. Returns [] on error. */
  async fetchSchedules(ttl = 15000): Promise<Schedule[]> {
    return this.get('schedules', ttl, async () => {
      try {
        const res = await this.fetchFn('/schedules');
        if (!res.ok) return [];
        const data = await res.json();
        return data.schedules ?? [];
      } catch {
        return [];
      }
    });
  }

  /** Fetch certificate status. Returns null on error. */
  async fetchCertificates(ttl = 30000): Promise<CertificateStatus | null> {
    return this.get('certificates', ttl, async () => {
      try {
        const res = await this.fetchFn('/certificates');
        if (!res.ok) return null;
        return res.json();
      } catch {
        return null;
      }
    });
  }

  /** Invalidate a single key or all cached data. */
  invalidate(key?: string) {
    if (key) {
      this.cache.delete(key);
    } else {
      this.cache.clear();
      this.cacheCreatedAt = Date.now();
    }
  }

  /** Core get-or-fetch with TTL and promise coalescing. */
  private async get<T>(key: string, ttl: number, fetcher: () => Promise<T>): Promise<T> {
    // Return cached if fresh
    const cached = this.cache.get(key);
    if (cached && Date.now() - cached.ts < ttl) {
      return cached.data as T;
    }

    // Coalesce concurrent requests
    const existing = this.inflight.get(key);
    if (existing) return existing as Promise<T>;

    const promise = fetcher()
      .then((data) => {
        this.cache.set(key, { data, ts: Date.now() });
        this.inflight.delete(key);
        return data;
      })
      .catch((err) => {
        this.inflight.delete(key);
        throw err;
      });

    this.inflight.set(key, promise);
    return promise;
  }
}
