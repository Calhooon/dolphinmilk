/**
 * Artifacts gallery page — displays all file artifacts produced by the agent
 * across all tasks. Images render as a visual grid gallery; non-image files
 * appear in a clean list view below.
 *
 * Fetches from GET /artifacts with optional ?type= filter.
 */

import { LitElement, html, css, nothing } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { unsafeHTML } from 'lit/directives/unsafe-html.js';
import {
  pageHost, centerState, pageTitle, statsBar, stateFeedback, skeleton,
  renderLoading, renderEmpty, renderSmartEmpty, renderError,
} from '../lib/shared-styles.js';
import { FetchController } from '../controllers/fetch-controller.js';
import { formatRelativeTime, truncate } from '../lib/util.js';
import hljs from 'highlight.js/lib/core';
import hljsJson from 'highlight.js/lib/languages/json';
import hljsPython from 'highlight.js/lib/languages/python';
import hljsRust from 'highlight.js/lib/languages/rust';
import hljsTypescript from 'highlight.js/lib/languages/typescript';
import hljsJavascript from 'highlight.js/lib/languages/javascript';
import hljsBash from 'highlight.js/lib/languages/bash';
import hljsYaml from 'highlight.js/lib/languages/yaml';
import hljsXml from 'highlight.js/lib/languages/xml';
import hljsCss from 'highlight.js/lib/languages/css';
import hljsSql from 'highlight.js/lib/languages/sql';
import hljsMarkdown from 'highlight.js/lib/languages/markdown';
import hljsIni from 'highlight.js/lib/languages/ini';
import hljsDiff from 'highlight.js/lib/languages/diff';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

// Register languages
hljs.registerLanguage('json', hljsJson);
hljs.registerLanguage('python', hljsPython);
hljs.registerLanguage('rust', hljsRust);
hljs.registerLanguage('typescript', hljsTypescript);
hljs.registerLanguage('javascript', hljsJavascript);
hljs.registerLanguage('bash', hljsBash);
hljs.registerLanguage('yaml', hljsYaml);
hljs.registerLanguage('xml', hljsXml);
hljs.registerLanguage('css', hljsCss);
hljs.registerLanguage('sql', hljsSql);
hljs.registerLanguage('markdown', hljsMarkdown);
hljs.registerLanguage('ini', hljsIni);
hljs.registerLanguage('diff', hljsDiff);

// ---- Types ----

type ArtifactType = 'image' | 'video' | 'audio' | 'document' | 'file';

interface ArtifactItem {
  name: string;
  type: ArtifactType;
  size_bytes: number;
  url: string;
  task_id: string;
  task_text: string;
  created_by: string;
  created_at: number;
}

interface ArtifactsResponse {
  artifacts: ArtifactItem[];
  total: number;
}

type FilterTab = 'all' | ArtifactType;

// ---- Helpers ----

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function formatTotalSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function getFileExtension(name: string): string {
  const parts = name.split('.');
  return parts.length > 1 ? parts[parts.length - 1].toLowerCase() : '';
}

function getFileIcon(ext: string): string {
  const icons: Record<string, string> = {
    // Code
    rs: '\u{1F980}', ts: '\u{1F7E6}', js: '\u{1F7E8}', py: '\u{1F40D}',
    json: '{}', toml: '\u2699', yaml: '\u2699', yml: '\u2699',
    // Documents
    md: '\u{1F4DD}', txt: '\u{1F4C4}', pdf: '\u{1F4D1}', csv: '\u{1F4CA}',
    html: '\u{1F310}', css: '\u{1F3A8}', sh: '$',
    // Media
    svg: '\u25B3',
    png: '\u{1F5BC}', jpg: '\u{1F5BC}', jpeg: '\u{1F5BC}', webp: '\u{1F5BC}', gif: '\u{1F5BC}',
    mp4: '\u{1F3AC}', webm: '\u{1F3AC}', mov: '\u{1F3AC}',
    mp3: '\u{1F3B5}', wav: '\u{1F3B5}', ogg: '\u{1F3B5}', flac: '\u{1F3B5}',
    // Archives
    zip: '\u{1F4E6}', tar: '\u{1F4E6}', gz: '\u{1F4E6}',
  };
  return icons[ext] || '\u{1F4C4}';
}

const TEXT_EXTENSIONS = new Set([
  'rs', 'ts', 'js', 'jsx', 'tsx', 'py', 'rb', 'go', 'java', 'c', 'cpp', 'h',
  'json', 'toml', 'yaml', 'yml', 'xml', 'csv',
  'md', 'txt', 'log', 'cfg', 'ini', 'env',
  'html', 'css', 'scss', 'less', 'sass',
  'sh', 'bash', 'zsh', 'fish',
  'sql', 'graphql', 'proto',
  'dockerfile', 'makefile',
]);

function isTextFile(name: string): boolean {
  const ext = getFileExtension(name);
  if (TEXT_EXTENSIONS.has(ext)) return true;
  // Files with no extension might be text (Makefile, Dockerfile, etc.)
  const base = name.split('/').pop()?.toLowerCase() ?? '';
  return ['makefile', 'dockerfile', 'readme', 'license', 'changelog'].includes(base);
}

const EXT_TO_LANG: Record<string, string> = {
  rs: 'rust', ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript',
  py: 'python', json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'ini',
  html: 'xml', xml: 'xml', svg: 'xml', css: 'css', scss: 'css',
  sh: 'bash', bash: 'bash', zsh: 'bash', fish: 'bash',
  sql: 'sql', graphql: 'graphql', md: 'markdown',
  ini: 'ini', cfg: 'ini', env: 'ini',
  diff: 'diff', patch: 'diff',
};

function highlightCode(text: string, ext: string): string {
  const lang = EXT_TO_LANG[ext];
  if (lang && hljs.getLanguage(lang)) {
    try {
      return hljs.highlight(text, { language: lang }).value;
    } catch { /* fall through */ }
  }
  // Auto-detect for unknown extensions
  try {
    return hljs.highlightAuto(text).value;
  } catch {
    // Escape HTML for plain display
    return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
}

function tryPrettyPrintJson(text: string): string {
  try {
    const parsed = JSON.parse(text);
    return JSON.stringify(parsed, null, 2);
  } catch {
    return text;
  }
}

function renderMarkdownToHtml(text: string): string {
  const raw = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(raw);
}

function getOriginLabel(created_by: string): { label: string; isAgent: boolean } {
  if (!created_by || created_by === 'user' || created_by === 'upload') {
    return { label: 'You', isAgent: false };
  }
  return { label: created_by, isAgent: true };
}

const FILTER_TABS: { key: FilterTab; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'image', label: 'Images' },
  { key: 'video', label: 'Videos' },
  { key: 'audio', label: 'Audio' },
  { key: 'document', label: 'Documents' },
  { key: 'file', label: 'Files' },
];

@customElement('dm-artifacts')
export class DmArtifacts extends LitElement {
  @property({ attribute: false }) fetchFn: typeof fetch = fetch.bind(window);
  @state() private activeFilter: FilterTab = 'all';
  @state() private groupByConversation = false;
  @state() private brokenImages = new Set<string>();
  @state() private previewItem: ArtifactItem | null = null;
  @state() private previewContent: string | null = null;
  @state() private previewLoading = false;

  private ctrl = new FetchController<ArtifactsResponse>(this);
  private _keyHandler = this._handleKey.bind(this);

  static styles = [pageHost, centerState, pageTitle, statsBar, stateFeedback, skeleton, css`
    /* ---- Filter pills ---- */
    .filter-bar {
      display: flex;
      gap: 6px;
      margin-bottom: 20px;
      flex-wrap: wrap;
    }

    .filter-pill {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 6px 16px;
      border-radius: var(--radius-pill, 999px);
      border: 1px solid var(--border, #1A3550);
      background: transparent;
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 13px;
      font-family: var(--sans, sans-serif);
      cursor: pointer;
      transition: all 0.2s ease;
      user-select: none;
    }

    .filter-pill:hover {
      color: var(--text, rgba(255,255,255,0.8));
      border-color: var(--text-dim);
    }

    .filter-pill.active {
      background: var(--accent, #14A8C4);
      color: var(--text-bright, #fff);
      border-color: var(--accent, #14A8C4);
    }

    .filter-pill .count {
      font-size: 11px;
      font-family: var(--mono, monospace);
      opacity: 0.7;
    }

    .filter-pill.active .count {
      opacity: 0.9;
    }

    /* ---- Section headers ---- */
    .section-header {
      font-size: 13px;
      font-weight: 600;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-transform: uppercase;
      letter-spacing: 0.8px;
      margin: 28px 0 14px;
      padding-bottom: 8px;
      border-bottom: 1px solid var(--border, #1A3550);
    }

    .section-header:first-of-type {
      margin-top: 0;
    }

    /* ---- Image gallery grid ---- */
    .image-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: 12px;
      margin-bottom: 8px;
    }

    .image-card {
      position: relative;
      border-radius: var(--radius-md, 14px);
      overflow: hidden;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      transition: transform 0.2s ease, box-shadow 0.2s ease, border-color 0.2s ease;
      cursor: pointer;
    }

    .image-card:hover {
      transform: scale(1.02);
      box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
      border-color: var(--accent-dim, #0E8FA8);
    }

    .image-card:focus-visible {
      outline: 2px solid var(--accent, #14A8C4);
      outline-offset: 2px;
    }

    @media (prefers-reduced-motion: reduce) {
      .image-card { transition: none; }
      .image-card:hover { transform: none; }
    }

    .image-card img {
      width: 100%;
      aspect-ratio: 1 / 1;
      object-fit: cover;
      display: block;
      background: rgba(0, 0, 0, 0.2);
    }

    .image-card .image-broken-placeholder {
      width: 100%;
      aspect-ratio: 1 / 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 6px;
      background: rgba(0, 0, 0, 0.15);
      color: var(--text-dim, rgba(255,255,255,0.4));
      font-size: 11px;
    }

    .image-broken-icon {
      font-size: 28px;
      opacity: 0.5;
    }

    /* Hover overlay */
    .image-overlay {
      position: absolute;
      inset: 0;
      background: linear-gradient(
        0deg,
        rgba(0, 0, 0, 0.85) 0%,
        rgba(0, 0, 0, 0.4) 50%,
        transparent 100%
      );
      display: flex;
      flex-direction: column;
      justify-content: flex-end;
      padding: 12px;
      opacity: 0;
      transition: opacity 0.2s ease;
    }

    .image-card:hover .image-overlay,
    .image-card:focus-within .image-overlay {
      opacity: 1;
    }

    @media (prefers-reduced-motion: reduce) {
      .image-overlay { transition: none; }
    }

    .overlay-name {
      font-size: 12px;
      font-weight: 600;
      color: var(--text-bright, #fff);
      font-family: var(--mono, monospace);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      margin-bottom: 2px;
    }

    .overlay-meta {
      font-size: 11px;
      color: rgba(255, 255, 255, 0.6);
      margin-bottom: 8px;
    }

    .overlay-task {
      font-size: 10px;
      color: rgba(255, 255, 255, 0.5);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      margin-bottom: 8px;
    }

    .overlay-actions {
      display: flex;
      gap: 6px;
    }

    .overlay-actions a {
      font-size: 11px;
      color: var(--text-bright, #fff);
      text-decoration: none;
      padding: 4px 12px;
      border-radius: 6px;
      background: rgba(255, 255, 255, 0.12);
      backdrop-filter: blur(8px);
      transition: background 0.15s ease;
      white-space: nowrap;
    }

    .overlay-actions a:hover {
      background: rgba(255, 255, 255, 0.22);
    }

    /* ---- File list ---- */
    .file-list {
      display: flex;
      flex-direction: column;
      gap: 6px;
    }

    .file-row {
      display: flex;
      align-items: center;
      gap: 14px;
      padding: 12px 16px;
      background: var(--bg-elevated, #142D42);
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-sm, 8px);
      transition: border-color 0.15s ease, background 0.15s ease;
    }

    .file-row:hover {
      border-color: rgba(255, 255, 255, 0.08);
      background: rgba(255, 255, 255, 0.02);
    }

    .file-icon {
      width: 44px;
      height: 44px;
      display: flex;
      align-items: center;
      justify-content: center;
      font-size: 22px;
      background: rgba(255, 255, 255, 0.04);
      border-radius: var(--radius-sm, 8px);
      flex-shrink: 0;
    }

    .file-info {
      flex: 1;
      min-width: 0;
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .file-name {
      font-size: 13px;
      font-weight: 600;
      color: var(--text-bright, #fff);
      font-family: var(--mono, monospace);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .file-meta {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.4));
      display: flex;
      align-items: center;
      gap: 8px;
      flex-wrap: wrap;
    }

    .file-meta .sep {
      color: var(--border, #1A3550);
    }

    .type-badge {
      display: inline-flex;
      align-items: center;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 10px;
      font-weight: 600;
      font-family: var(--mono, monospace);
      text-transform: uppercase;
      letter-spacing: 0.5px;
      background: rgba(20, 168, 196, 0.12);
      color: var(--accent, #14A8C4);
    }

    .type-badge.video {
      background: rgba(168, 85, 247, 0.12);
      color: #a855f7;
    }

    .type-badge.audio {
      background: rgba(252, 190, 45, 0.12);
      color: var(--warning, #fcbe2d);
    }

    .type-badge.document {
      background: rgba(0, 182, 155, 0.12);
      color: var(--success, #00b69b);
    }

    .type-badge.file {
      background: rgba(255, 255, 255, 0.06);
      color: var(--text-dim, rgba(255,255,255,0.5));
    }

    .task-link {
      font-size: 11px;
      color: var(--accent, #14A8C4);
      text-decoration: none;
      transition: opacity 0.15s ease;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      max-width: 200px;
    }

    .task-link:hover {
      opacity: 0.8;
    }

    .file-actions {
      display: flex;
      gap: 8px;
      flex-shrink: 0;
    }

    .file-actions a {
      font-size: 11px;
      color: var(--accent, #14A8C4);
      text-decoration: none;
      padding: 4px 10px;
      border: 1px solid rgba(20, 168, 196, 0.3);
      border-radius: 4px;
      transition: background 0.15s ease;
      white-space: nowrap;
    }

    .file-actions a:hover {
      background: rgba(20, 168, 196, 0.1);
    }

    /* ---- Lightbox / Preview overlay ---- */
    .lightbox-backdrop {
      position: fixed;
      inset: 0;
      z-index: 9999;
      background: rgba(0, 0, 0, 0.88);
      backdrop-filter: blur(12px);
      display: flex;
      align-items: center;
      justify-content: center;
      animation: lightbox-fade-in 0.2s ease;
    }

    @keyframes lightbox-fade-in {
      from { opacity: 0; }
      to { opacity: 1; }
    }

    .lightbox-close {
      position: absolute;
      top: 16px;
      right: 16px;
      width: 40px;
      height: 40px;
      border: none;
      background: rgba(255, 255, 255, 0.1);
      color: var(--text-bright, #fff);
      font-size: 20px;
      border-radius: 50%;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      transition: background 0.15s;
      z-index: 10;
    }

    .lightbox-close:hover {
      background: rgba(255, 255, 255, 0.2);
    }

    .lightbox-nav {
      position: absolute;
      top: 50%;
      transform: translateY(-50%);
      width: 48px;
      height: 48px;
      border: none;
      background: rgba(255, 255, 255, 0.08);
      color: var(--text-bright, #fff);
      font-size: 22px;
      border-radius: 50%;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      transition: background 0.15s;
      z-index: 10;
    }

    .lightbox-nav:hover {
      background: rgba(255, 255, 255, 0.16);
    }

    .lightbox-nav.prev { left: 16px; }
    .lightbox-nav.next { right: 16px; }

    .lightbox-content {
      max-width: calc(100vw - 120px);
      max-height: calc(100vh - 140px);
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 16px;
    }

    .lightbox-content img {
      max-width: 100%;
      max-height: calc(100vh - 220px);
      object-fit: contain;
      border-radius: var(--radius-sm, 8px);
      box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    }

    .lightbox-content video {
      max-width: 100%;
      max-height: calc(100vh - 220px);
      border-radius: var(--radius-sm, 8px);
      box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
      background: #000;
    }

    .lightbox-content audio {
      width: min(500px, 90vw);
    }

    .lightbox-code-wrap {
      width: min(900px, calc(100vw - 120px));
      max-height: calc(100vh - 220px);
      overflow: auto;
      background: #1a1e2e;
      border: 1px solid var(--border, #1A3550);
      border-radius: var(--radius-md, 14px);
      box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    }

    .code-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 8px 20px;
      border-bottom: 1px solid rgba(255,255,255,0.06);
      background: rgba(0,0,0,0.15);
    }

    .code-lang {
      font-size: 11px;
      font-family: var(--mono, monospace);
      font-weight: 600;
      color: var(--accent, #14A8C4);
      text-transform: uppercase;
      letter-spacing: 0.5px;
    }

    .code-lines {
      font-size: 11px;
      font-family: var(--mono, monospace);
      color: var(--text-dim, rgba(255,255,255,0.3));
    }

    .lightbox-code-wrap pre {
      margin: 0;
      padding: 16px 20px;
      font-family: var(--mono, monospace);
      font-size: 13px;
      line-height: 1.65;
      color: #c9d1d9;
      white-space: pre-wrap;
      word-break: break-word;
      tab-size: 2;
      background: transparent;
    }

    .lightbox-code-wrap pre code {
      font-family: inherit;
    }

    .lightbox-code-wrap .line {
      display: block;
    }

    .lightbox-code-wrap .line-num {
      display: inline-block;
      width: 3.5em;
      margin-right: 1.2em;
      text-align: right;
      color: rgba(255,255,255,0.15);
      font-size: 12px;
      user-select: none;
      -webkit-user-select: none;
    }

    /* ---- highlight.js token colors (GitHub Dark inspired) ---- */
    .hljs-keyword, .hljs-selector-tag, .hljs-built_in { color: #ff7b72; }
    .hljs-string, .hljs-attr { color: #a5d6ff; }
    .hljs-number, .hljs-literal { color: #79c0ff; }
    .hljs-comment, .hljs-quote { color: #8b949e; font-style: italic; }
    .hljs-function .hljs-title, .hljs-title.function_ { color: #d2a8ff; }
    .hljs-type, .hljs-title.class_ { color: #ffa657; }
    .hljs-variable, .hljs-template-variable { color: #ffa657; }
    .hljs-name, .hljs-tag { color: #7ee787; }
    .hljs-attribute { color: #79c0ff; }
    .hljs-regexp { color: #a5d6ff; }
    .hljs-symbol, .hljs-bullet { color: #ffa657; }
    .hljs-section { color: #79c0ff; font-weight: bold; }
    .hljs-meta { color: #8b949e; }
    .hljs-punctuation { color: #c9d1d9; }
    .hljs-addition { color: #aff5b4; background: rgba(46,160,67,0.15); }
    .hljs-deletion { color: #ffdcd7; background: rgba(248,81,73,0.15); }

    /* ---- Markdown rendered view ---- */
    .lightbox-markdown {
      background: var(--bg-elevated, #142D42);
    }

    .markdown-body {
      padding: 28px 32px;
      font-family: var(--sans, sans-serif);
      font-size: 14px;
      line-height: 1.7;
      color: var(--text, rgba(255,255,255,0.8));
    }

    .markdown-body h1, .markdown-body h2, .markdown-body h3,
    .markdown-body h4, .markdown-body h5, .markdown-body h6 {
      color: var(--text-bright, #fff);
      margin: 1.5em 0 0.5em;
      line-height: 1.3;
    }

    .markdown-body h1 { font-size: 1.8em; border-bottom: 1px solid var(--border); padding-bottom: 0.3em; }
    .markdown-body h2 { font-size: 1.4em; border-bottom: 1px solid var(--border); padding-bottom: 0.2em; }
    .markdown-body h3 { font-size: 1.15em; }

    .markdown-body p { margin: 0.8em 0; }

    .markdown-body a {
      color: var(--accent, #14A8C4);
      text-decoration: none;
    }

    .markdown-body a:hover { text-decoration: underline; }

    .markdown-body code {
      font-family: var(--mono, monospace);
      font-size: 0.9em;
      padding: 0.2em 0.4em;
      border-radius: 4px;
      background: rgba(255,255,255,0.06);
      color: #c9d1d9;
    }

    .markdown-body pre {
      background: rgba(0,0,0,0.2);
      border-radius: 8px;
      padding: 14px 18px;
      overflow-x: auto;
      margin: 1em 0;
    }

    .markdown-body pre code {
      padding: 0;
      background: none;
    }

    .markdown-body blockquote {
      border-left: 3px solid var(--accent, #14A8C4);
      margin: 1em 0;
      padding: 0.5em 1em;
      color: var(--text-dim);
      background: rgba(255,255,255,0.02);
      border-radius: 0 6px 6px 0;
    }

    .markdown-body ul, .markdown-body ol {
      padding-left: 1.8em;
      margin: 0.6em 0;
    }

    .markdown-body li { margin: 0.3em 0; }

    .markdown-body table {
      border-collapse: collapse;
      width: 100%;
      margin: 1em 0;
    }

    .markdown-body th, .markdown-body td {
      border: 1px solid var(--border, #1A3550);
      padding: 8px 12px;
      text-align: left;
      font-size: 13px;
    }

    .markdown-body th {
      background: rgba(255,255,255,0.04);
      font-weight: 600;
      color: var(--text-bright);
    }

    .markdown-body hr {
      border: none;
      border-top: 1px solid var(--border);
      margin: 1.5em 0;
    }

    .markdown-body img {
      max-width: 100%;
      border-radius: 8px;
    }

    .markdown-body strong { color: var(--text-bright, #fff); }

    .lightbox-pdf {
      width: min(900px, calc(100vw - 120px));
      height: calc(100vh - 220px);
      border: none;
      border-radius: var(--radius-md, 14px);
      box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    }

    .lightbox-info {
      display: flex;
      align-items: center;
      gap: 16px;
      padding: 12px 20px;
      background: rgba(255, 255, 255, 0.06);
      border-radius: var(--radius-pill, 999px);
      max-width: 100%;
    }

    .lightbox-info-name {
      font-size: 13px;
      font-weight: 600;
      color: var(--text-bright, #fff);
      font-family: var(--mono, monospace);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .lightbox-info-meta {
      font-size: 11px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      white-space: nowrap;
    }

    .lightbox-info-actions {
      display: flex;
      gap: 8px;
      margin-left: auto;
      flex-shrink: 0;
    }

    .lightbox-info-actions a {
      font-size: 11px;
      color: var(--text-bright, #fff);
      text-decoration: none;
      padding: 5px 14px;
      border-radius: 6px;
      background: rgba(255, 255, 255, 0.1);
      transition: background 0.15s;
      white-space: nowrap;
    }

    .lightbox-info-actions a:hover {
      background: rgba(255, 255, 255, 0.2);
    }

    .lightbox-info-actions a.primary {
      background: var(--accent, #14A8C4);
    }

    .lightbox-info-actions a.primary:hover {
      background: #0E8FA8;
    }

    .lightbox-loading {
      color: var(--text-dim, rgba(255,255,255,0.5));
      font-size: 14px;
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .lightbox-unsupported {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 16px;
      padding: 60px 40px;
      color: var(--text-dim, rgba(255,255,255,0.5));
      text-align: center;
    }

    .lightbox-unsupported .big-icon {
      font-size: 56px;
      opacity: 0.6;
    }

    .lightbox-unsupported .hint {
      font-size: 13px;
      max-width: 300px;
    }

    @media (max-width: 639px) {
      .lightbox-content {
        max-width: calc(100vw - 32px);
      }

      .lightbox-nav { display: none; }

      .lightbox-info {
        flex-wrap: wrap;
        border-radius: var(--radius-sm, 8px);
        gap: 8px;
        padding: 10px 14px;
      }

      .lightbox-info-actions {
        width: 100%;
        justify-content: flex-end;
      }

      .lightbox-code-wrap {
        width: calc(100vw - 32px);
      }
    }

    /* ---- Skeleton loading ---- */
    .skeleton-grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
      gap: 12px;
      margin-bottom: 24px;
    }

    .skeleton-image {
      aspect-ratio: 1 / 1;
      border-radius: var(--radius-md, 14px);
    }

    .skeleton-row {
      height: 68px;
      margin-bottom: 6px;
    }

    .skeleton-pill {
      height: 32px;
      width: 80px;
      border-radius: var(--radius-pill, 999px);
      display: inline-block;
      margin-right: 6px;
    }

    .skeleton-stats {
      height: 20px;
      width: 300px;
      margin-bottom: 20px;
    }

    /* ---- Responsive ---- */
    @media (max-width: 639px) {
      .image-grid {
        grid-template-columns: repeat(2, 1fr);
        gap: 8px;
      }

      .image-card:hover {
        transform: none;
      }

      /* Always show overlay on mobile since hover is unavailable */
      .image-overlay {
        opacity: 1;
        background: linear-gradient(
          0deg,
          rgba(0, 0, 0, 0.75) 0%,
          transparent 60%
        );
      }

      .file-row {
        flex-wrap: wrap;
        gap: 10px;
        padding: 10px 12px;
      }

      .file-actions {
        width: 100%;
        justify-content: flex-end;
      }

      .task-link {
        max-width: 140px;
      }

      .filter-bar {
        gap: 4px;
      }

      .filter-pill {
        padding: 5px 12px;
        font-size: 12px;
      }
    }

    @media (min-width: 640px) and (max-width: 1023px) {
      .image-grid {
        grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
      }
    }
  `];

  connectedCallback() {
    super.connectedCallback();
    document.addEventListener('keydown', this._keyHandler);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener('keydown', this._keyHandler);
  }

  firstUpdated() {
    this.ctrl.fetch(async () => {
      const res = await this.fetchFn('/artifacts');
      if (!res.ok) throw new Error(`Failed to load artifacts: ${res.status}`);
      return await res.json() as ArtifactsResponse;
    });
  }

  private _handleKey(e: KeyboardEvent) {
    if (!this.previewItem) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      this.closePreview();
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      this.navigatePreview(-1);
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      this.navigatePreview(1);
    }
  }

  private openPreview(a: ArtifactItem) {
    this.previewItem = a;
    this.previewContent = null;
    this.previewLoading = false;
    if (isTextFile(a.name)) {
      if (a.size_bytes === 0) {
        this.previewContent = '(empty file)';
      } else {
        this.loadTextContent(a);
      }
    }
  }

  private closePreview() {
    this.previewItem = null;
    this.previewContent = null;
    this.previewLoading = false;
  }

  private async loadTextContent(a: ArtifactItem) {
    this.previewLoading = true;
    try {
      // Use plain fetch — /files/ doesn't need BRC-31 auth, and AuthFetch can interfere
      const res = await window.fetch(a.url);
      if (!res.ok) throw new Error(`${res.status}`);
      const text = await res.text();
      if (text.length === 0) {
        this.previewContent = '(empty file)';
      } else if (text.length > 200_000) {
        this.previewContent = text.slice(0, 200_000) + '\n\n... (truncated)';
      } else {
        this.previewContent = text;
      }
    } catch {
      this.previewContent = '(Failed to load file content)';
    } finally {
      this.previewLoading = false;
    }
  }

  private navigatePreview(dir: -1 | 1) {
    if (!this.previewItem) return;
    const list = this.filteredArtifacts;
    const idx = list.findIndex(a => a.url === this.previewItem!.url);
    if (idx < 0) return;
    const next = idx + dir;
    if (next >= 0 && next < list.length) {
      this.openPreview(list[next]);
    }
  }

  private get previewIndex(): number {
    if (!this.previewItem) return -1;
    return this.filteredArtifacts.findIndex(a => a.url === this.previewItem!.url);
  }

  private get allArtifacts(): ArtifactItem[] {
    return this.ctrl.data?.artifacts ?? [];
  }

  private get filteredArtifacts(): ArtifactItem[] {
    if (this.activeFilter === 'all') return this.allArtifacts;
    return this.allArtifacts.filter(a => a.type === this.activeFilter);
  }

  private countByType(type: FilterTab): number {
    if (type === 'all') return this.allArtifacts.length;
    return this.allArtifacts.filter(a => a.type === type).length;
  }

  private get images(): ArtifactItem[] {
    return this.filteredArtifacts.filter(a => a.type === 'image');
  }

  private get nonImages(): ArtifactItem[] {
    return this.filteredArtifacts.filter(a => a.type !== 'image');
  }

  private get totalSize(): number {
    return this.filteredArtifacts.reduce((sum, a) => sum + a.size_bytes, 0);
  }

  private get uniqueTaskCount(): number {
    return new Set(this.filteredArtifacts.map(a => a.task_id)).size;
  }

  render() {
    if (this.ctrl.loading) {
      return this.renderSkeleton();
    }

    if (this.ctrl.hasError) {
      return renderError(
        this.ctrl.error ?? 'Failed to load artifacts',
        'Check that the agent server is running.',
      );
    }

    if (this.allArtifacts.length === 0) {
      return renderSmartEmpty({
        icon: '\u{1F5BC}',
        title: 'No artifacts yet',
        description: 'Artifacts appear here when your agent produces files \u2014 images, documents, code, and more. Start a task that generates output to see your first artifacts.',
        action: { label: 'Start a Conversation \u2192', href: '#' },
      });
    }

    const filtered = this.filteredArtifacts;
    const images = this.images;
    const nonImages = this.nonImages;
    const isFiltered = this.activeFilter !== 'all';
    const showingImages = this.activeFilter === 'all' || this.activeFilter === 'image';
    const showingNonImages = this.activeFilter !== 'image';

    return html`
      ${this.renderPreview()}

      <div class="page-title">Artifacts <span style="font-size:16px;color:var(--text-dim);font-weight:400">${this.allArtifacts.length.toLocaleString()}</span></div>

      ${this.renderFilterBar()}
      <div style="display:flex;justify-content:flex-end;margin-bottom:8px">
        <button class="group-toggle" @click=${() => { this.groupByConversation = !this.groupByConversation; }}
          style="background:${this.groupByConversation ? 'var(--accent)' : 'var(--bg-elevated)'};
                 color:${this.groupByConversation ? '#fff' : 'var(--text-dim)'};
                 border:1px solid ${this.groupByConversation ? 'var(--accent)' : 'var(--border)'};
                 border-radius:20px;padding:4px 12px;font-size:12px;cursor:pointer;font-family:var(--sans)">
          Group by task
        </button>
      </div>

      <div class="stats-bar">
        <div class="stat">
          <span class="stat-value">${filtered.length.toLocaleString()}</span>
          <span class="stat-label">${isFiltered ? `of ${this.allArtifacts.length}` : 'items'}</span>
        </div>
        <div class="stat">
          <span class="stat-value">${formatTotalSize(this.totalSize)}</span>
          <span class="stat-label">total size</span>
        </div>
        <div class="stat">
          <span class="stat-value">${this.uniqueTaskCount}</span>
          <span class="stat-label">task${this.uniqueTaskCount !== 1 ? 's' : ''}</span>
        </div>
      </div>

      ${filtered.length === 0 && isFiltered
        ? renderEmpty('No matching artifacts', 'Try a different filter.')
        : this.groupByConversation
          ? this.renderGrouped(filtered)
          : html`
            ${showingImages && images.length > 0 ? html`
              ${this.activeFilter === 'all' && nonImages.length > 0
                ? html`<div class="section-header">Images (${images.length})</div>`
                : nothing}
              <div class="image-grid">
                ${images.map(a => this.renderImageCard(a))}
              </div>
            ` : nothing}

            ${showingNonImages && nonImages.length > 0 ? html`
              ${this.activeFilter === 'all' && images.length > 0
                ? html`<div class="section-header">Files (${nonImages.length})</div>`
                : nothing}
              <div class="file-list">
                ${nonImages.map(a => this.renderFileRow(a))}
              </div>
            ` : nothing}
          `
      }
    `;
  }

  private renderGrouped(filtered: ArtifactItem[]) {
    // Group artifacts by task_id
    const groups = new Map<string, { text: string; items: ArtifactItem[] }>();
    for (const a of filtered) {
      const key = a.task_id || 'unknown';
      if (!groups.has(key)) groups.set(key, { text: a.task_text || key.slice(0, 8), items: [] });
      groups.get(key)!.items.push(a);
    }

    return html`
      ${[...groups.entries()].map(([taskId, group]) => html`
        <div style="margin-bottom:20px">
          <div style="display:flex;align-items:center;gap:8px;padding-bottom:6px;border-bottom:1px solid var(--border);margin-bottom:8px">
            <span style="font-size:13px;color:var(--text);flex:1">${truncate(group.text, 80)}</span>
            <a href="#task/${taskId}" style="font-size:11px;color:var(--accent);text-decoration:none;white-space:nowrap">View task \u2192</a>
          </div>
          <div class="image-grid">
            ${group.items.filter(a => a.type === 'image').map(a => this.renderImageCard(a))}
          </div>
          ${group.items.filter(a => a.type !== 'image').length > 0 ? html`
            <div class="file-list">
              ${group.items.filter(a => a.type !== 'image').map(a => this.renderFileRow(a))}
            </div>
          ` : nothing}
        </div>
      `)}
    `;
  }

  private renderFilterBar() {
    return html`
      <div class="filter-bar">
        ${FILTER_TABS.map(tab => {
          const count = this.countByType(tab.key);
          return html`
            <button
              class="filter-pill ${this.activeFilter === tab.key ? 'active' : ''}"
              @click=${() => { this.activeFilter = tab.key; }}
            >
              ${tab.label}
              ${count > 0 ? html`<span class="count">${count}</span>` : nothing}
            </button>
          `;
        })}
      </div>
    `;
  }

  private renderImageCard(a: ArtifactItem) {
    const isBroken = this.brokenImages.has(a.url);
    const origin = getOriginLabel(a.created_by);
    return html`
      <div class="image-card" tabindex="0" @click=${() => this.openPreview(a)} @keydown=${(e: KeyboardEvent) => { if (e.key === 'Enter') this.openPreview(a); }}>
        ${isBroken
          ? html`
            <div class="image-broken-placeholder">
              <span class="image-broken-icon">\u{1F5BC}</span>
              <span>Image unavailable</span>
            </div>
          `
          : html`
            <img
              src="${a.url}"
              alt="${a.name}"
              loading="lazy"
              @error=${() => {
                this.brokenImages = new Set([...this.brokenImages, a.url]);
              }}
            />
          `
        }
        <div class="image-overlay">
          <div class="overlay-name" title="${a.name}">${a.name}</div>
          <div class="overlay-meta">
            ${formatSize(a.size_bytes)} \u00B7 ${formatRelativeTime(a.created_at)}
            \u00B7 <span style="color:${origin.isAgent ? 'var(--accent)' : 'var(--success)'}">${origin.isAgent ? '\u{1F916}' : '\u{1F464}'} ${origin.label}</span>
          </div>
          ${a.task_text
            ? html`<div class="overlay-task" title="${a.task_text}">${truncate(a.task_text, 40)}</div>`
            : nothing}
          <div class="overlay-actions">
            <a href="javascript:void(0)" @click=${(e: Event) => { e.stopPropagation(); this.openPreview(a); }}>Open</a>
            <a href="${a.url}" download="${a.name}" @click=${(e: Event) => e.stopPropagation()}>Download</a>
            ${a.task_id
              ? html`<a href="#task/${a.task_id}" @click=${(e: Event) => e.stopPropagation()}>Task</a>`
              : nothing}
          </div>
        </div>
      </div>
    `;
  }

  private renderFileRow(a: ArtifactItem) {
    const ext = getFileExtension(a.name);
    const origin = getOriginLabel(a.created_by);
    return html`
      <div class="file-row" style="cursor:pointer" @click=${() => this.openPreview(a)}>
        <div class="file-icon">${getFileIcon(ext)}</div>
        <div class="file-info">
          <span class="file-name" title="${a.name}">${a.name}</span>
          <div class="file-meta">
            <span class="type-badge ${a.type}">${a.type}</span>
            <span>${formatSize(a.size_bytes)}</span>
            <span class="sep">\u00B7</span>
            <span style="color:${origin.isAgent ? 'var(--accent)' : 'var(--success)'}">
              ${origin.isAgent ? '\u{1F916}' : '\u{1F464}'} ${origin.label}
            </span>
            <span class="sep">\u00B7</span>
            <span>${formatRelativeTime(a.created_at)}</span>
            ${a.task_id ? html`
              <span class="sep">\u00B7</span>
              <a class="task-link" href="#task/${a.task_id}" title="${a.task_text || a.task_id}" @click=${(e: Event) => e.stopPropagation()}>${truncate(a.task_text || a.task_id, 30)}</a>
            ` : nothing}
          </div>
        </div>
        <div class="file-actions">
          <a href="javascript:void(0)" @click=${(e: Event) => { e.stopPropagation(); this.openPreview(a); }}>Open</a>
          <a href="${a.url}" download="${a.name}" @click=${(e: Event) => e.stopPropagation()}>Download</a>
        </div>
      </div>
    `;
  }

  private renderPreview() {
    const a = this.previewItem;
    if (!a) return nothing;

    const idx = this.previewIndex;
    const total = this.filteredArtifacts.length;
    const hasPrev = idx > 0;
    const hasNext = idx < total - 1;
    const origin = getOriginLabel(a.created_by);
    const ext = getFileExtension(a.name);

    return html`
      <div class="lightbox-backdrop" @click=${(e: Event) => {
        if (e.target === e.currentTarget) this.closePreview();
      }}>
        <button class="lightbox-close" @click=${() => this.closePreview()} title="Close (Esc)">\u2715</button>

        ${hasPrev ? html`<button class="lightbox-nav prev" @click=${() => this.navigatePreview(-1)} title="Previous (\u2190)">\u2039</button>` : nothing}
        ${hasNext ? html`<button class="lightbox-nav next" @click=${() => this.navigatePreview(1)} title="Next (\u2192)">\u203A</button>` : nothing}

        <div class="lightbox-content">
          ${this.renderPreviewContent(a, ext)}

          <div class="lightbox-info">
            <span class="lightbox-info-name" title="${a.name}">${a.name}</span>
            <span class="lightbox-info-meta">
              ${formatSize(a.size_bytes)}
              \u00A0\u00B7\u00A0
              <span style="color:${origin.isAgent ? 'var(--accent, #14A8C4)' : 'var(--success, #00b69b)'}">${origin.isAgent ? '\u{1F916}' : '\u{1F464}'} ${origin.label}</span>
              \u00A0\u00B7\u00A0
              ${formatRelativeTime(a.created_at)}
            </span>
            <div class="lightbox-info-actions">
              ${a.task_id ? html`<a href="#task/${a.task_id}">Task</a>` : nothing}
              <a href="${a.url}" download="${a.name}">Download</a>
              <a class="primary" href="${a.url}" target="_blank" rel="noopener">New Tab</a>
            </div>
          </div>
        </div>
      </div>
    `;
  }

  private renderPreviewContent(a: ArtifactItem, ext: string) {
    if (a.type === 'image') {
      return html`<img src="${a.url}" alt="${a.name}" />`;
    }

    if (a.type === 'video') {
      return html`<video src="${a.url}" controls autoplay></video>`;
    }

    if (a.type === 'audio') {
      return html`
        <div style="display:flex;flex-direction:column;align-items:center;gap:20px;padding:40px 0">
          <span style="font-size:64px;opacity:0.5">\u{1F3B5}</span>
          <audio src="${a.url}" controls autoplay></audio>
        </div>
      `;
    }

    if (ext === 'pdf') {
      return html`<embed class="lightbox-pdf" src="${a.url}" type="application/pdf" />`;
    }

    if (isTextFile(a.name)) {
      if (this.previewLoading) {
        return html`<div class="lightbox-loading"><span class="spinner"></span> Loading file...</div>`;
      }
      if (this.previewContent != null) {
        // Markdown: render as formatted HTML
        if (ext === 'md') {
          const rendered = renderMarkdownToHtml(this.previewContent);
          return html`
            <div class="lightbox-code-wrap lightbox-markdown">
              <div class="markdown-body">${unsafeHTML(rendered)}</div>
            </div>
          `;
        }

        // JSON: pretty-print first
        const content = ext === 'json' ? tryPrettyPrintJson(this.previewContent) : this.previewContent;
        const highlighted = highlightCode(content, ext);
        const lines = highlighted.split('\n');

        return html`
          <div class="lightbox-code-wrap">
            <div class="code-header">
              <span class="code-lang">${EXT_TO_LANG[ext] || ext || 'text'}</span>
              <span class="code-lines">${lines.length} lines</span>
            </div>
            <pre class="hljs"><code>${lines.map((line, i) => html`<span class="line"><span class="line-num">${i + 1}</span>${unsafeHTML(line || '\u00A0')}</span>\n`)}</code></pre>
          </div>
        `;
      }
    }

    // Unsupported — show a nice fallback
    const icon = getFileIcon(ext);
    return html`
      <div class="lightbox-unsupported">
        <span class="big-icon">${icon}</span>
        <span style="font-size:16px;font-weight:600;color:var(--text-bright)">${a.name}</span>
        <span class="hint">This file type can't be previewed inline. Download it or open in a new tab.</span>
      </div>
    `;
  }

  private renderSkeleton() {
    return html`
      <div class="page-title">Artifacts</div>
      <div style="margin-bottom:20px">
        ${[1, 2, 3, 4, 5, 6].map(() => html`<div class="skeleton skeleton-pill"></div>`)}
      </div>
      <div class="skeleton skeleton-stats"></div>
      <div class="skeleton-grid">
        ${[1, 2, 3, 4, 5, 6, 7, 8].map(() => html`<div class="skeleton skeleton-image"></div>`)}
      </div>
      <div>
        ${[1, 2, 3].map(() => html`<div class="skeleton skeleton-row"></div>`)}
      </div>
    `;
  }
}
