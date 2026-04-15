#!/usr/bin/env node
/**
 * Dolphin Milk Integration Test Runner
 *
 * Runs real-BSV test scenarios through the live Dolphin Milk UI via Playwright.
 * Each test sends a message, waits for a response, and captures metrics.
 *
 * Usage:
 *   node run.js                    # Run all non-skipped scenarios
 *   node run.js --tier trivial     # Run only trivial tier
 *   node run.js --id 1,2,3        # Run specific scenario IDs
 *   node run.js --canary           # Run just test 1 (health check)
 *   node run.js --dry-run          # Show what would run without executing
 *
 * Prerequisites:
 *   - dolphin-milk server running at localhost:8080
 *   - Funded wallet (check budget page first!)
 *   - npx playwright install chromium (first time only)
 *
 * Cost: ~$0.01 per trivial test, ~$0.05 per complex test
 */

const { chromium } = require('playwright');
const fs = require('fs');
const path = require('path');

const { captureLastAssistantMessage, getSatsBalance } = require('./lib/scraper');
const { evaluateResult, compareWithBaseline, evaluateArtifacts } = require('./lib/evaluator');
const { navigateToNewChat, waitForEnabled } = require('./lib/navigation');
const { getArg, formatCost, formatDuration } = require('./lib/helpers');
const { getTranscriptCost } = require('./lib/transcript-cost');

const SCENARIOS_FILE = path.join(__dirname, 'scenarios.json');
const RESULTS_DIR = path.join(__dirname, 'results');

/** Get the most recent task ID from the server status endpoint. */
/** Extract the most recent task ID from network requests the page made to /task/{id}/events. */
async function extractTaskIdFromPage(page) {
  // The chat UI polls /task/{id}/events — extract the task ID from those URLs
  const taskId = await page.evaluate(() => {
    // Check performance entries for task event polling URLs
    const entries = performance.getEntriesByType('resource');
    for (let i = entries.length - 1; i >= 0; i--) {
      const url = entries[i].name;
      const match = url.match(/\/task\/([a-f0-9-]+)\/events/);
      if (match) return match[1];
    }
    return null;
  });
  return taskId;
}

async function main() {
  const args = process.argv.slice(2);
  const tierFilter = getArg(args, '--tier');
  const idFilter = getArg(args, '--id');
  const canary = args.includes('--canary');
  const dryRun = args.includes('--dry-run');
  const compareBaselineFlag = args.includes('--compare');

  const scenarios = JSON.parse(fs.readFileSync(SCENARIOS_FILE, 'utf8'));
  let tests = scenarios.scenarios.filter(s => !s.skip);

  if (canary) tests = tests.filter(s => s.id === 1);
  if (tierFilter) tests = tests.filter(s => s.tier === tierFilter);
  if (idFilter) {
    const ids = idFilter.split(',').map(Number);
    tests = tests.filter(s => ids.includes(s.id));
  }

  console.log(`\n🧪 Dolphin Milk Integration Tests`);
  console.log(`   ${tests.length} scenarios selected`);
  console.log(`   Server: ${scenarios.server_url}`);
  console.log(`   Cooldown: ${scenarios.cooldown_between_tests_ms}ms between tests\n`);

  if (dryRun) {
    tests.forEach(t => console.log(`  [${t.tier}] #${t.id} ${t.name}: "${t.message.substring(0, 60)}..."`));
    console.log(`\nDry run — no tests executed.`);
    return;
  }

  // Launch browser
  const browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();

  // Connect to worm — initial load to verify connectivity
  console.log('Connecting to Dolphin Milk UI...');
  await page.goto(scenarios.server_url + '/ui/');
  try {
    await page.getByText('Connecting...').first().waitFor({ state: 'hidden', timeout: 15000 });
  } catch {
    console.error('ERROR: Could not connect to Dolphin Milk. Is the server running?');
    await browser.close();
    process.exit(1);
  }

  // Get initial wallet balance
  const walletStart = await getSatsBalance(page);
  console.log(`Wallet: ${walletStart.toLocaleString()} sats\n`);

  // Run tests
  const results = [];
  for (const test of tests) {
    process.stdout.write(`  [${test.tier}] #${test.id} ${test.name}... `);

    try {
      const result = await runScenario(page, test, scenarios);
      result.pass = evaluateResult(test, result);

      // Post-scenario artifact validation (if scenario defines expected_artifacts)
      if (test.expected_artifacts) {
        try {
          const { checkArtifacts } = require('./lib/artifact-checker');
          const taskId = await extractTaskIdFromPage(page);
          if (taskId) {
            result.artifacts = await checkArtifacts(page, taskId, scenarios.server_url);
            result.artifactPass = evaluateArtifacts(test, result.artifacts);
            if (!result.artifactPass) result.pass = false;
          }
        } catch (err) {
          result.artifactError = err.message;
          // Don't fail the test for artifact check errors — log and continue
          process.stdout.write(` [artifact check failed: ${err.message.substring(0, 50)}] `);
        }
      }

      results.push(result);

      const icon = result.pass ? '✓' : '✗';
      const artifactInfo = result.artifacts ? ` [${result.artifacts.count} artifact${result.artifacts.count !== 1 ? 's' : ''}]` : '';
      const costInfo = result.costBreakdown
        ? `${formatCost(result.satsSpent)} (paid ${formatCost(result.costBreakdown.paid)}, refund ${result.costBreakdown.refundRate})`
        : formatCost(result.satsSpent);
      console.log(`${icon} ${costInfo} ${formatDuration(result.latencyMs)}${artifactInfo} — "${result.response.substring(0, 60)}"`);
    } catch (err) {
      console.log(`✗ ERROR: ${err.message.split('\n')[0]}`);
      results.push({
        id: test.id, name: test.name, tier: test.tier,
        pass: false, error: err.message,
        response: '', satsSpent: 0, latencyMs: 0, tokens: ''
      });
    }

    // Cooldown between tests
    if (test !== tests[tests.length - 1]) {
      await page.waitForTimeout(scenarios.cooldown_between_tests_ms);
    }
  }

  // Get final wallet balance
  const walletEnd = await getSatsBalance(page);
  await browser.close();

  // Summary — use transcript-derived effective cost, not wallet diff
  const passed = results.filter(r => r.pass).length;
  const failed = results.filter(r => !r.pass).length;
  const totalEffective = results.reduce((sum, r) => sum + (r.satsSpent || 0), 0);
  const totalPaid = results.reduce((sum, r) => sum + (r.costBreakdown?.paid || 0), 0);
  const totalRefunded = results.reduce((sum, r) => sum + (r.costBreakdown?.refunded || 0), 0);
  const walletDiff = walletStart - walletEnd;

  console.log(`\n${'─'.repeat(60)}`);
  console.log(`Results: ${passed} passed, ${failed} failed`);
  console.log(`Cost (effective): ${totalEffective.toLocaleString()} sats ($${(totalEffective * 0.0000001439).toFixed(4)})`);
  if (totalPaid > 0) {
    console.log(`Cost (gross paid): ${totalPaid.toLocaleString()} sats → refunded ${totalRefunded.toLocaleString()} sats (${((totalRefunded / totalPaid) * 100).toFixed(1)}%)`);
  }
  console.log(`Wallet: ${walletStart.toLocaleString()} → ${walletEnd.toLocaleString()} sats (diff: ${walletDiff.toLocaleString()})`);

  // Save results
  const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
  const resultsFile = path.join(RESULTS_DIR, `run-${timestamp}.json`);
  fs.mkdirSync(RESULTS_DIR, { recursive: true });
  fs.writeFileSync(resultsFile, JSON.stringify({
    timestamp: new Date().toISOString(),
    walletStart, walletEnd,
    totalEffective, totalPaid, totalRefunded,
    walletDiff,
    passed, failed, total: results.length,
    results
  }, null, 2));
  console.log(`Results saved: ${resultsFile}`);

  // Compare against baseline if requested
  if (compareBaselineFlag) {
    compareWithBaseline(results, tests);
  }

  // Exit with failure code if any test failed
  process.exit(failed > 0 ? 1 : 0);
}

async function runScenario(page, test, scenarios) {
  const start = Date.now();

  // Handle UI rejection tests (empty input)
  if (test.expected_behavior === 'ui_rejects_send') {
    await navigateToNewChat(page, scenarios.server_url);
    const input = page.getByRole('textbox', { name: 'Message Dolphin Milk...' });
    await waitForEnabled(input, page);
    await input.fill(test.message);
    await page.waitForTimeout(300);
    const isDisabled = await page.getByRole('button', { name: 'Send' }).isDisabled();
    return {
      id: test.id, name: test.name, tier: test.tier,
      response: isDisabled ? 'UI_REJECTED' : 'UI_ALLOWED_UNEXPECTEDLY',
      satsSpent: 0, tokens: '0', latencyMs: Date.now() - start
    };
  }

  // Determine model: CLI --model flag > scenario model field > null (use default)
  const modelOverride = getArg(process.argv.slice(2), '--model') || test.model || null;

  await navigateToNewChat(page, scenarios.server_url);

  // Select model via the UI model picker if override is specified
  if (modelOverride) {
    await selectModelInUI(page, modelOverride);
  }

  const input = page.getByRole('textbox', { name: 'Message Dolphin Milk...' });
  await waitForEnabled(input, page);
  await input.fill(test.message);
  await page.waitForTimeout(300);
  await page.getByRole('button', { name: 'Send' }).click();

  // Wait for response — look for the task to complete
  await page.waitForTimeout(3000);
  try {
    const timeout = test.timeout_ms || scenarios.default_timeout_ms;
    await page.getByRole('button', { name: 'Stop' }).waitFor({ state: 'hidden', timeout });
  } catch { /* might complete before Stop appears */ }
  // Settle delay: the UI may still be rendering the final response after the
  // Stop button disappears. Wait long enough for the last dm-message to land.
  await page.waitForTimeout(3000);

  const latencyMs = Date.now() - start;
  const tokens = await page.getByText(/\d+ tok/).first().textContent().catch(() => '0');

  // Extract task ID first — needed by both response capture and cost reading
  const taskId = await extractTaskIdFromPage(page);

  // Capture the LAST assistant message — 4-tier fallback (shadow DOM → tool cards → page text → transcript)
  const response = await captureLastAssistantMessage(page, taskId);
  let satsSpent = 0;
  let costBreakdown = null;
  if (taskId) {
    const tc = getTranscriptCost(taskId);
    if (tc.found) {
      satsSpent = tc.effectiveSats;
      costBreakdown = {
        effective: tc.effectiveSats,
        paid: tc.paidSats,
        refunded: tc.refundedSats,
        thinkCalls: tc.thinkCalls,
        toolCost: tc.toolCostSats,
        proofCost: tc.proofCostSats,
        refundRate: tc.paidSats > 0 ? ((tc.refundedSats / tc.paidSats) * 100).toFixed(1) + '%' : 'n/a',
      };
    }
  }

  return {
    id: test.id, name: test.name, tier: test.tier,
    response: response.substring(0, 4000),
    satsSpent, tokens, latencyMs, taskId,
    costBreakdown,
  };
}

/**
 * Select a model via the UI model picker (shadow DOM traversal).
 * Path: dm-app → dm-chat → dm-chat-input → .model-trigger → .model-option
 */
async function selectModelInUI(page, modelId) {
  // Map model IDs to display labels used in the UI
  const MODEL_LABELS = {
    'gpt-5-nano': 'GPT-5 Nano',
    'gpt-5-mini': 'GPT-5 Mini',
    'gpt-5': 'GPT-5',
    'gpt-5.2': 'GPT-5.2',
    'o4-mini': 'o4 Mini',
    'gpt-5.2-pro': 'GPT-5.2 Pro',
    'claude-haiku-4-5': 'Haiku 4.5',
    'claude-sonnet-4-6': 'Sonnet 4.6',
    'claude-opus-4-6': 'Opus 4.6',
  };

  const targetLabel = MODEL_LABELS[modelId] || modelId;

  try {
    // Step 1: Open the flyout by clicking the model trigger
    await page.evaluate(() => {
      const app = document.querySelector('dm-app');
      const chat = app?.shadowRoot?.querySelector('dm-chat');
      const chatInput = chat?.shadowRoot?.querySelector('dm-chat-input');
      const trigger = chatInput?.shadowRoot?.querySelector('.model-trigger');
      if (trigger) trigger.click();
    });

    // Step 2: Wait for the flyout to render
    await page.waitForTimeout(500);

    // Step 3: Find and click the matching model option
    const selected = await page.evaluate((label) => {
      const app = document.querySelector('dm-app');
      const chat = app?.shadowRoot?.querySelector('dm-chat');
      const chatInput = chat?.shadowRoot?.querySelector('dm-chat-input');
      const sr = chatInput?.shadowRoot;
      if (!sr) return 'no shadow root';

      const options = sr.querySelectorAll('.model-option');
      for (const opt of options) {
        const nameEl = opt.querySelector('.model-name');
        if (nameEl && nameEl.textContent.trim() === label) {
          opt.click();
          return 'ok';
        }
      }
      const names = [...options].map(o => o.querySelector('.model-name')?.textContent?.trim());
      return `model "${label}" not found in [${names.join(', ')}]`;
    }, targetLabel);

    if (selected !== 'ok') {
      console.warn(`  [model-picker] ${selected} — using default model`);
    }
    await page.waitForTimeout(300);
  } catch (err) {
    console.warn(`  [model-picker] failed: ${err.message} — using default model`);
  }
}

main().catch(err => {
  console.error('Fatal:', err);
  process.exit(1);
});
