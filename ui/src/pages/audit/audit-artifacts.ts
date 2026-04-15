/**
 * Artifacts tab for the per-task audit page.
 * Fetches file artifacts from GET /task/{taskId}/artifacts.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { centerState, stateFeedback, renderLoading, renderEmpty, renderError } from '../../lib/shared-styles.js';

// ---- Types ----

type ArtifactType = 'image' | 'video' | 'audio' | 'document' | 'file';

interface ArtifactItem {
  name: string;
  type: ArtifactType;
  size_bytes: number;
  created_by: string;
  created_at: number;
  url: string;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function getFileExtension(name: string): string {
  const parts = name.split('.');
  return parts.length > 1 ? parts[parts.length - 1].toLowerCase() : '';
}

function getFileIcon(ext: string): string {
  const icons: Record<string, string> = {
    rs: '\u{1F980}',
    ts: '\u{1F7E6}',
    js: '\u{1F7E8}',
    json: '{}',
    toml: '\u2699',
    yaml: '\u2699',
    yml: '\u2699',
    md: '\u{1F4DD}',
    html: '\u{1F310}',
    css: '\u{1F3A8}',
    py: '\u{1F40D}',
    sh: '$',
    svg: '\u25B3',
    png: '\u{1F5BC}',
    jpg: '\u{1F5BC}',
    jpeg: '\u{1F5BC}',
    webp: '\u{1F5BC}',
    gif: '\u{1F5BC}',
    txt: '\u{1F4C4}',
    pdf: '\u{1F4D1}',
    mp4: '\u{1F3AC}',
    mp3: '\u{1F3B5}',
    wav: '\u{1F3B5}',
  };
  return icons[ext] || '\u{1F4C4}';
}

@customElement('dm-audit-artifacts')
export class WormAuditArtifacts extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @property() taskId = '';

  @state() private loading = false;
  @state() private error = '';
  @state() private artifacts: ArtifactItem[] = [];

  static styles = [centerState, stateFeedback, css`
    :host {
      display: block;
    }

    .artifact-count {
      font-size: 12px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      font-family: var(--mono, monospace);
      margin-bottom: 12px;
    }

    .artifact-grid {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }

    .artifact-card {
      display: flex;
      align-items: center;
      gap: 14px;
      padding: 12px 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-sm, 8px);
      transition: border-color 0.15s;
    }
    .artifact-card:hover {
      border-color: rgba(255,255,255,0.08);
    }

    .artifact-card img {
      width: 64px;
      height: 64px;
      object-fit: cover;
      border-radius: 6px;
      flex-shrink: 0;
      background: rgba(0,0,0,0.2);
    }

    .image-broken {
      width: 64px;
      height: 64px;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 6px;
      flex-shrink: 0;
      background: rgba(0,0,0,0.2);
      color: var(--text-dim, rgba(255,255,255,0.3));
      font-size: 10px;
      text-align: center;
    }

    .file-icon {
      width: 64px;
      height: 64px;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 28px;
      background: rgba(255,255,255,0.04);
      border-radius: 6px;
      flex-shrink: 0;
    }

    .artifact-info {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .artifact-name {
      font-size: 13px;
      font-weight: 600;
      color: var(--text-bright, #fff);
      font-family: var(--mono, monospace);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .artifact-meta {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.4));
    }

    .artifact-actions {
      display: flex;
      gap: 8px;
      flex-shrink: 0;
    }

    .artifact-actions a {
      font-size: 11px;
      color: var(--accent, #14A8C4);
      text-decoration: none;
      padding: 4px 10px;
      border: 1px solid rgba(20, 168, 196, 0.3);
      border-radius: 4px;
      transition: background 0.15s;
    }
    .artifact-actions a:hover {
      background: rgba(20, 168, 196, 0.1);
    }

    @media (max-width: 639px) {
      .artifact-card {
        flex-wrap: wrap;
        gap: 10px;
      }
      .artifact-actions {
        width: 100%;
        justify-content: flex-end;
      }
    }
  `];

  connectedCallback() {
    super.connectedCallback();
    if (this.taskId) this.loadArtifacts();
  }

  updated(changed: Map<string, unknown>) {
    if (changed.has('taskId') && this.taskId) this.loadArtifacts();
  }

  private async loadArtifacts() {
    if (!this.taskId) return;
    this.loading = true;
    this.error = '';
    try {
      const res = await this.fetchFn(`/task/${this.taskId}/artifacts`);
      if (!res.ok) throw new Error(`${res.status}`);
      const data = await res.json();
      this.artifacts = data.artifacts || [];
    } catch (e: unknown) {
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      this.loading = false;
    }
  }

  render() {
    if (this.loading) return renderLoading('Loading artifacts...');
    if (this.error) return renderError(this.error);
    if (this.artifacts.length === 0) {
      return renderEmpty('No artifacts found', 'This task did not produce any files.');
    }

    return html`
      <div class="artifact-count">${this.artifacts.length} artifact${this.artifacts.length !== 1 ? 's' : ''}</div>
      <div class="artifact-grid">
        ${this.artifacts.map(a => this.renderArtifact(a))}
      </div>
    `;
  }

  private renderArtifact(a: ArtifactItem) {
    const isImage = a.type === 'image';
    return html`
      <div class="artifact-card">
        ${isImage ? this.renderImageThumb(a) : this.renderFileIcon(a)}
        <div class="artifact-info">
          <span class="artifact-name" title="${a.name}">${a.name}</span>
          <span class="artifact-meta">${formatSize(a.size_bytes)} · ${a.created_by || 'unknown tool'}</span>
        </div>
        <div class="artifact-actions">
          <a href="${a.url}" target="_blank" rel="noopener">Open</a>
          <a href="${a.url}" download="${a.name}">Download</a>
        </div>
      </div>
    `;
  }

  private renderImageThumb(a: ArtifactItem) {
    return html`
      <img
        src="${a.url}"
        loading="lazy"
        alt="${a.name}"
        @error=${(e: Event) => {
          const img = e.target as HTMLImageElement;
          const placeholder = document.createElement('div');
          placeholder.className = 'image-broken';
          placeholder.textContent = 'Image unavailable';
          img.replaceWith(placeholder);
        }}
      />
    `;
  }

  private renderFileIcon(a: ArtifactItem) {
    const ext = getFileExtension(a.name);
    return html`<div class="file-icon">${getFileIcon(ext)}</div>`;
  }
}
