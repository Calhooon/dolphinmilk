/**
 * Page navigation and input readiness helpers.
 *
 * Handles fresh page loads, "New Chat" clicks, and input polling.
 */

/** Navigate to the chat UI and start a fresh conversation. */
async function navigateToNewChat(page, serverUrl) {
  // Navigate to chat view — each test gets a totally fresh page load
  await page.goto(serverUrl + '/ui/');
  await page.getByText('Connecting...').first().waitFor({ state: 'hidden', timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(1500);

  // Click "New Chat" to get a clean conversation with no prior history
  try {
    await page.getByText('New Chat').click({ timeout: 3000 });
    await page.waitForTimeout(800);
  } catch {
    // If New Chat button not found, the fresh page load is OK
  }
}

/** Wait for an input element to become enabled (heartbeat tasks may keep it busy). */
async function waitForEnabled(input, page, maxWaitMs = 60000) {
  await input.waitFor({ state: 'visible', timeout: maxWaitMs });
  const deadline = Date.now() + maxWaitMs;
  while (Date.now() < deadline) {
    if (!(await input.isDisabled())) return;
    await page.waitForTimeout(500);
  }
}

module.exports = { navigateToNewChat, waitForEnabled };
