/**
 * Lit ReactiveController for interval-based polling.
 * Encapsulates start/stop/pause/resume lifecycle.
 */

import type { ReactiveController, ReactiveControllerHost } from 'lit';

export class PollingController implements ReactiveController {
  private host: ReactiveControllerHost;
  private timer: number | null = null;
  private _interval: number;
  private callback: () => void | Promise<void>;
  private _paused = false;

  constructor(host: ReactiveControllerHost, callback: () => void | Promise<void>, interval = 5000) {
    this.host = host;
    this.callback = callback;
    this._interval = interval;
    host.addController(this);
  }

  hostConnected(): void {
    this.start();
  }

  hostDisconnected(): void {
    this.stop();
  }

  /** Start the polling loop. */
  start(): void {
    if (this.timer !== null) return;
    this._paused = false;
    this.timer = window.setInterval(() => {
      if (!this._paused) this.callback();
    }, this._interval);
  }

  /** Stop the polling loop entirely. */
  stop(): void {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
    this._paused = false;
  }

  /** Pause without clearing the timer. */
  pause(): void {
    this._paused = true;
  }

  /** Resume after pause. */
  resume(): void {
    this._paused = false;
  }

  /** Change the interval (restarts the timer). */
  setInterval(ms: number): void {
    this._interval = ms;
    if (this.timer !== null) {
      this.stop();
      this.start();
    }
  }

  /** Fire the callback immediately (outside the interval). */
  trigger(): void {
    this.callback();
  }

  get paused(): boolean {
    return this._paused;
  }

  get running(): boolean {
    return this.timer !== null && !this._paused;
  }
}
