/**
 * Artifact validation for completed tasks.
 *
 * Checks that a task's artifacts are visible via the filesystem manifest
 * (artifacts.json) and in the UI's Artifacts tab (shadow DOM scraping).
 */

const fs = require('fs');
const path = require('path');

/**
 * Check artifacts for a completed task via filesystem manifest and UI.
 *
 * @param {import('playwright').Page} page - Playwright page object
 * @param {string} taskId - task ID to check
 * @param {string} serverUrl - server base URL (e.g. "http://localhost:8080")
 * @returns {Promise<{
 *   count: number,
 *   items: Array<{ type: string, name: string, url?: string, size?: number, createdBy?: string }>,
 *   uiTabVisible: boolean,
 *   uiArtifactCount: number,
 *   screenshotPath?: string,
 * }>}
 */
async function checkArtifacts(page, taskId, serverUrl) {
  const manifest = await checkViaManifest(taskId);
  const ui = await checkViaUi(page, taskId, serverUrl);

  return {
    count: manifest.count,
    items: manifest.items,
    uiTabVisible: ui.uiTabVisible,
    uiArtifactCount: ui.uiArtifactCount,
    screenshotPath: ui.screenshotPath,
  };
}

/**
 * Check artifacts by reading the artifacts.json manifest from disk.
 *
 * @param {string} taskId
 * @returns {Promise<{ count: number, items: Array<{ type: string, name: string, url: string, size: number, createdBy: string }> }>}
 */
async function checkViaManifest(taskId) {
  try {
    // Find the task directory
    const tasksDir = path.join(__dirname, '..', '..', '..', 'working', 'tasks');
    const dirs = fs.readdirSync(tasksDir).filter(d => d.startsWith(taskId.substring(0, 8)));
    if (dirs.length === 0) return { count: 0, items: [] };

    const taskDir = path.join(tasksDir, dirs[0]);
    const manifestPath = path.join(taskDir, 'artifacts.json');
    if (!fs.existsSync(manifestPath)) {
      // No manifest — fall back to scanning directory
      return scanDirectory(taskDir);
    }

    const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
    const items = manifest.map(a => ({
      type: a.type || 'file',
      name: a.name,
      url: a.url || '',
      size: a.size_bytes || 0,
      createdBy: a.created_by || '',
    }));

    return { count: items.length, items };
  } catch {
    return { count: 0, items: [] };
  }
}

/**
 * Fallback: scan task directory for non-system files.
 *
 * @param {string} taskDir - absolute path to the task directory
 * @returns {{ count: number, items: Array<{ type: string, name: string, url: string, size: number, createdBy: string }> }}
 */
function scanDirectory(taskDir) {
  const SYSTEM_FILES = new Set([
    'session.jsonl', 'budget.jsonl', 'artifacts.json', 'fork_context.json'
  ]);

  const items = [];
  try {
    for (const entry of fs.readdirSync(taskDir, { withFileTypes: true })) {
      if (entry.isDirectory()) continue; // skip all directories
      const name = entry.name;
      if (SYSTEM_FILES.has(name)) continue;
      if (name.startsWith('.') || name.startsWith('tool_output_')) continue;

      const ext = path.extname(name).toLowerCase().replace('.', '');
      const type = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp'].includes(ext) ? 'image'
        : ['mp4', 'webm', 'mov'].includes(ext) ? 'video'
        : ['mp3', 'wav', 'ogg', 'm4a', 'flac'].includes(ext) ? 'audio'
        : 'file';

      const stat = fs.statSync(path.join(taskDir, name));
      items.push({ type, name, size: stat.size, url: '', createdBy: '' });
    }
  } catch { /* ignore */ }

  return { count: items.length, items };
}

/**
 * Navigate to the task's audit page, click the Artifacts tab, and scrape
 * artifact cards from the shadow DOM.
 *
 * @param {import('playwright').Page} page
 * @param {string} taskId
 * @param {string} serverUrl
 * @returns {Promise<{ uiTabVisible: boolean, uiArtifactCount: number, screenshotPath?: string }>}
 */
async function checkViaUi(page, taskId, serverUrl) {
  try {
    await page.goto(`${serverUrl}/ui/#task/${taskId}`);
    await page.waitForTimeout(3000);

    // Find and click the Artifacts tab via shadow DOM
    const tabClicked = await page.evaluate(() => {
      const app = document.querySelector('dm-app');
      if (!app?.shadowRoot) return false;
      const audit = app.shadowRoot.querySelector('dm-audit');
      if (!audit?.shadowRoot) return false;

      const buttons = Array.from(audit.shadowRoot.querySelectorAll('button'));
      const artifactsBtn = buttons.find(b => b.textContent?.trim().toLowerCase().includes('artifact'));
      if (!artifactsBtn) return false;
      artifactsBtn.click();
      return true;
    }).catch(() => false);

    if (!tabClicked) {
      return { uiTabVisible: false, uiArtifactCount: 0 };
    }

    await page.waitForTimeout(2000);

    // Count artifact cards in the artifacts component
    const uiArtifactCount = await page.evaluate(() => {
      const app = document.querySelector('dm-app');
      if (!app?.shadowRoot) return 0;
      const audit = app.shadowRoot.querySelector('dm-audit');
      if (!audit?.shadowRoot) return 0;
      const artifacts = audit.shadowRoot.querySelector('dm-audit-artifacts');
      if (!artifacts?.shadowRoot) return 0;

      // New component uses .artifact-card class
      return artifacts.shadowRoot.querySelectorAll('.artifact-card').length;
    }).catch(() => 0);

    // Take a screenshot for visual evidence
    const resultsDir = path.join(__dirname, '..', 'results');
    fs.mkdirSync(resultsDir, { recursive: true });
    const screenshotPath = path.join(resultsDir, `artifacts-${taskId.substring(0, 8)}.png`);
    await page.screenshot({ path: screenshotPath, fullPage: true });

    return { uiTabVisible: true, uiArtifactCount, screenshotPath };
  } catch {
    return { uiTabVisible: false, uiArtifactCount: 0 };
  }
}

module.exports = { checkArtifacts };
