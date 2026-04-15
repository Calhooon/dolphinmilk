/**
 * Response extraction from the Dolphin Milk UI.
 *
 * 4-tier fallback strategy for maximum reliability:
 *   1. Shadow DOM traversal (dm-app → dm-chat → dm-message)
 *   2. Tool card output (dm-tool-card .output)
 *   3. Broad page text scraping (all p/li elements)
 *   4. Transcript fallback (session.jsonl on disk — source of truth)
 *
 * The UI-based tiers (1-3) can fail due to Lit component lifecycle timing,
 * shadow DOM hierarchy changes, or rendering delays. Tier 4 reads the
 * transcript directly and always succeeds if the task completed.
 */

const fs = require('fs');
const path = require('path');

// Workspace candidates for transcript fallback (same as transcript-cost.js)
const WORKSPACE_CANDIDATES = [
  path.join(__dirname, '..', '..', '..', 'working', 'tasks'),
  path.join(process.env.HOME || '', '.dolphin-milk', 'workspace', 'tasks'),
  path.join(process.env.HOME || '', '.bsv-worm', 'workspace', 'tasks'),
];

/**
 * Read the last assistant response from a task's session.jsonl transcript.
 * Returns the content of the last think_response event, or session_end result.
 */
function getResponseFromTranscript(taskId) {
  if (!taskId) return null;
  for (const tasksDir of WORKSPACE_CANDIDATES) {
    try {
      if (!fs.existsSync(tasksDir)) continue;
      const prefix = taskId.substring(0, 8);
      const dirs = fs.readdirSync(tasksDir).filter(d => d === taskId || d.startsWith(prefix));
      if (dirs.length === 0) continue;

      const sessionPath = path.join(tasksDir, dirs[0], 'session.jsonl');
      if (!fs.existsSync(sessionPath)) continue;

      const lines = fs.readFileSync(sessionPath, 'utf8').trim().split('\n');
      let lastResponse = null;
      for (const line of lines) {
        try {
          const event = JSON.parse(line);
          if (event.type === 'think_response' && event.content) {
            lastResponse = event.content;
          }
          if (event.type === 'session_end' && event.result) {
            lastResponse = event.result;
          }
        } catch { /* skip malformed */ }
      }
      if (lastResponse) return lastResponse;
    } catch { /* try next candidate */ }
  }
  return null;
}

/** Capture the last assistant message text from the chat shadow DOM. */
async function captureLastAssistantMessage(page, taskId) {
  // Try shadow DOM approach first — most reliable
  const shadowResult = await page.evaluate(() => {
    const app = document.querySelector('dm-app');
    if (!app?.shadowRoot) return null;
    const chat = app.shadowRoot.querySelector('dm-chat');
    if (!chat?.shadowRoot) return null;

    // Get all message elements, filter to non-user (assistant/system)
    const msgs = Array.from(chat.shadowRoot.querySelectorAll('dm-message'));
    const assistantMsgs = msgs.filter(m => m.role && m.role !== 'user');
    if (assistantMsgs.length === 0) return null;

    const lastMsg = assistantMsgs[assistantMsgs.length - 1];
    if (!lastMsg.shadowRoot) return lastMsg.textContent?.trim() || null;

    // Extract deduplicated text from the message's shadow DOM
    const seen = new Set();
    const parts = [];
    for (const el of lastMsg.shadowRoot.querySelectorAll('p, li, code, td, pre')) {
      const text = el.textContent?.trim();
      if (text && text.length > 0 && !seen.has(text)) {
        seen.add(text);
        parts.push(text);
      }
    }
    return parts.length > 0 ? parts.join(' | ') : (lastMsg.shadowRoot.textContent?.trim() || null);
  }).catch(() => null);

  if (shadowResult) return shadowResult;

  // If no assistant message text found, try tool card output
  const toolCardResult = await page.evaluate(() => {
    const app = document.querySelector('dm-app');
    if (!app?.shadowRoot) return null;
    const chat = app.shadowRoot.querySelector('dm-chat');
    if (!chat?.shadowRoot) return null;

    // Get all tool cards
    const cards = Array.from(chat.shadowRoot.querySelectorAll('dm-tool-card'));
    if (cards.length === 0) return null;

    // Get the last tool card's output
    const lastCard = cards[cards.length - 1];
    const output = lastCard.output;
    if (output && output.trim().length > 0) return output.trim();

    // Fallback: try reading from shadow DOM
    if (lastCard.shadowRoot) {
      const outputEl = lastCard.shadowRoot.querySelector('.output, pre, code');
      if (outputEl) return outputEl.textContent?.trim() || null;
    }
    return null;
  }).catch(() => null);

  if (toolCardResult) return toolCardResult;

  // Tier 3: Broad page scraping (less precise)
  const pText = await page.locator('p').allTextContents().catch(() => []);
  const liText = await page.locator('li').allTextContents().catch(() => []);
  const broadResult = [...pText, ...liText].filter(t => t.length > 0).join(' | ');
  if (broadResult) return broadResult;

  // Tier 4: Transcript fallback — read from session.jsonl on disk (source of truth)
  const transcriptResult = getResponseFromTranscript(taskId);
  if (transcriptResult) return transcriptResult;

  return '';
}

async function getSatsBalance(page) {
  // Wait a moment for balance to load from /agent
  await page.waitForTimeout(2000);

  // Pierce shadow DOM to read balance from dm-app sidebar
  const balance = await page.evaluate(() => {
    const app = document.querySelector('dm-app');
    if (!app?.shadowRoot) return 0;

    // Look for sats text in the sidebar balance area
    const balEl = app.shadowRoot.querySelector('.balance-primary');
    if (balEl) {
      const text = balEl.textContent || '';
      // Match patterns like "195,022,259 sats" or "195,022,259"
      const satsMatch = text.match(/([\d,]+)\s*sats/);
      if (satsMatch) return parseInt(satsMatch[1].replace(/,/g, '')) || 0;
      // If showing USD primary, check the secondary for sats
      const secEl = app.shadowRoot.querySelector('.balance-secondary');
      if (secEl) {
        const secText = secEl.textContent || '';
        const secMatch = secText.match(/([\d,]+)\s*sats/);
        if (secMatch) return parseInt(secMatch[1].replace(/,/g, '')) || 0;
      }
      // Try parsing the primary as a plain number (no "sats" suffix)
      const numMatch = text.match(/([\d,]+)/);
      if (numMatch) return parseInt(numMatch[1].replace(/,/g, '')) || 0;
    }
    return 0;
  }).catch(() => 0);

  // Fallback to page-level text search
  if (balance > 0) return balance;
  const text = await page.getByText(/[\d,]+ sats/).first().textContent().catch(() => '0 sats');
  return parseInt(text.replace(/[, sats]/g, '')) || 0;
}

module.exports = { captureLastAssistantMessage, getSatsBalance, getResponseFromTranscript };
