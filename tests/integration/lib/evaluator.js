/**
 * Result evaluation and baseline comparison.
 *
 * Pure functions — no page interaction, no side effects.
 */

function evaluateResult(test, result) {
  if (test.expected_behavior === 'ui_rejects_send') {
    return result.response === 'UI_REJECTED';
  }

  const respLower = result.response.toLowerCase();

  if (test.expected_contains) {
    for (const term of test.expected_contains) {
      if (!respLower.includes(term.toLowerCase())) return false;
    }
  }

  if (test.expected_contains_any) {
    const found = test.expected_contains_any.some(t => respLower.includes(t.toLowerCase()));
    if (!found) return false;
  }

  if (test.expected_not_contains) {
    for (const term of test.expected_not_contains) {
      if (respLower.includes(term.toLowerCase())) return false;
    }
  }

  if (test.expected_pattern) {
    const regex = new RegExp(test.expected_pattern, 'i');
    if (!regex.test(result.response)) return false;
  }

  if (test.max_cost_sats && result.satsSpent > test.max_cost_sats) {
    return false;
  }

  if (result.response.length === 0 && !test.expected_behavior) {
    return false;
  }

  return true;
}

function compareWithBaseline(results, scenarios) {
  console.log(`\nBaseline Comparison:`);
  for (const result of results) {
    const scenario = scenarios.find(s => s.id === result.id);
    if (!scenario || !scenario.baseline_sats) continue;

    const costDelta = ((result.satsSpent - scenario.baseline_sats) / scenario.baseline_sats * 100).toFixed(0);
    const latDelta = scenario.baseline_latency_ms
      ? ((result.latencyMs - scenario.baseline_latency_ms) / scenario.baseline_latency_ms * 100).toFixed(0)
      : '?';

    const costFlag = Math.abs(costDelta) > 50 ? ' !!' : '';
    const latFlag = Math.abs(latDelta) > 100 ? ' !!' : '';

    console.log(`  #${result.id} ${result.name}: cost ${costDelta > 0 ? '+' : ''}${costDelta}%${costFlag}, latency ${latDelta > 0 ? '+' : ''}${latDelta}%${latFlag}`);
    if (result.artifacts && result.artifacts.count > 0) {
      console.log(`    artifacts: ${result.artifacts.count} found (${result.artifacts.items.map(i => i.type).join(', ')}), UI tab: ${result.artifacts.uiTabVisible ? 'visible' : 'missing'}`);
    }
  }
}

/**
 * Evaluate artifact check results against scenario expectations.
 *
 * @param {object} test — scenario from scenarios.json (has expected_artifacts field)
 * @param {object} artifacts — result from checkArtifacts() in artifact-checker.js
 * @returns {boolean} true if artifacts meet expectations
 */
function evaluateArtifacts(test, artifacts) {
  if (!test.expected_artifacts || !artifacts) return true; // no expectations = pass

  const exp = test.expected_artifacts;

  // Check minimum artifact count
  if (exp.min_count && artifacts.count < exp.min_count) {
    return false;
  }

  // Check that expected types are present
  if (exp.types && exp.types.length > 0) {
    for (const expectedType of exp.types) {
      const found = artifacts.items.some(item => item.type === expectedType);
      if (!found) return false;
    }
  }

  // Check UI tab was accessible (if we got artifacts via API, the tab should work)
  if (artifacts.count > 0 && !artifacts.uiTabVisible) {
    return false; // API found artifacts but UI tab wasn't accessible — UI bug
  }

  return true;
}

/**
 * Evaluate a task's session.jsonl transcript for structural issues.
 *
 * Reads the transcript from disk and checks:
 * 1. No unexpected `error` type events (session_end with non-empty error field counts)
 * 2. Cost within max_cost_sats bound (if specified)
 * 3. All think_request events have matching think_response (no orphaned requests)
 * 4. No more than maxConsecutiveThinks consecutive think_request events without
 *    a tool call in between (indicates discovery waste / stuck loop)
 *
 * @param {string} taskId - The task ID (or prefix) to look up
 * @param {string} workspacePath - Path to the workspace root (contains tasks/ subdir)
 * @param {object} [options] - Optional constraints
 * @param {number} [options.max_cost_sats] - Max allowed cost in sats
 * @param {number} [options.maxConsecutiveThinks] - Max consecutive think cycles without tool use (default: 5)
 * @param {string[]} [options.allowedErrors] - Error substrings that are expected and should not fail
 * @returns {{ pass: boolean, issues: string[], stats: { events: number, thinkRequests: number, thinkResponses: number, totalCostSats: number, iterations: number } }}
 */
function evaluateTranscript(taskId, workspacePath, options = {}) {
  const fs = require('fs');
  const path = require('path');

  const maxConsecutiveThinks = options.maxConsecutiveThinks || 5;
  const allowedErrors = options.allowedErrors || [];
  const issues = [];

  // Find the task directory
  const tasksDir = path.join(workspacePath, 'tasks');
  if (!fs.existsSync(tasksDir)) {
    return { pass: false, issues: ['tasks directory not found'], stats: { events: 0, thinkRequests: 0, thinkResponses: 0, totalCostSats: 0, iterations: 0 } };
  }

  const dirs = fs.readdirSync(tasksDir).filter(d => d.includes(taskId.substring(0, 8)));
  if (dirs.length === 0) {
    return { pass: false, issues: [`no task directory matching ${taskId.substring(0, 8)}`], stats: { events: 0, thinkRequests: 0, thinkResponses: 0, totalCostSats: 0, iterations: 0 } };
  }

  const sessionPath = path.join(tasksDir, dirs[0], 'session.jsonl');
  if (!fs.existsSync(sessionPath)) {
    return { pass: false, issues: ['session.jsonl not found'], stats: { events: 0, thinkRequests: 0, thinkResponses: 0, totalCostSats: 0, iterations: 0 } };
  }

  // Parse events
  const lines = fs.readFileSync(sessionPath, 'utf8').trim().split('\n').filter(Boolean);
  const events = [];
  for (const line of lines) {
    try {
      events.push(JSON.parse(line));
    } catch {
      issues.push(`malformed JSONL line: ${line.substring(0, 80)}`);
    }
  }

  if (events.length === 0) {
    return { pass: false, issues: ['empty transcript'], stats: { events: 0, thinkRequests: 0, thinkResponses: 0, totalCostSats: 0, iterations: 0 } };
  }

  // Collect stats
  let thinkRequests = 0;
  let thinkResponses = 0;
  let totalCostSats = 0;
  let iterations = 0;
  let consecutiveThinks = 0;
  let maxConsecutiveFound = 0;
  const thinkRequestIds = new Set();
  const thinkResponseIds = new Set();

  for (const event of events) {
    switch (event.type) {
      case 'think_request':
        thinkRequests++;
        if (event.id) thinkRequestIds.add(event.id);
        consecutiveThinks++;
        maxConsecutiveFound = Math.max(maxConsecutiveFound, consecutiveThinks);
        break;

      case 'think_response':
        thinkResponses++;
        if (event.id) thinkResponseIds.add(event.id);
        if (event.sats_effective) totalCostSats += event.sats_effective;
        // A think_response resets the consecutive counter only if it contains
        // tool_calls (indicates the agent is actually doing something, not just thinking)
        if (event.tool_calls && event.tool_calls.length > 0) {
          consecutiveThinks = 0;
        }
        break;

      case 'session_end':
        if (event.iterations) iterations = event.iterations;
        if (event.sats_spent) totalCostSats = event.sats_spent; // prefer session_end total
        if (event.error && event.error.length > 0) {
          const isAllowed = allowedErrors.some(ae => event.error.includes(ae));
          if (!isAllowed) {
            issues.push(`session ended with error: ${event.error.substring(0, 200)}`);
          }
        }
        break;

      case 'error':
        const errorMsg = event.message || event.error || JSON.stringify(event);
        const isAllowed = allowedErrors.some(ae => errorMsg.includes(ae));
        if (!isAllowed) {
          issues.push(`error event: ${errorMsg.substring(0, 200)}`);
        }
        break;
    }
  }

  // Check 1: Orphaned think_requests (more requests than responses)
  if (thinkRequests > thinkResponses + 1) {
    // Allow at most 1 unmatched request (task may have been killed mid-think)
    issues.push(`orphaned think_requests: ${thinkRequests} requests vs ${thinkResponses} responses`);
  }

  // Check 2: Cost bound
  if (options.max_cost_sats && totalCostSats > options.max_cost_sats) {
    issues.push(`cost ${totalCostSats} sats exceeds limit ${options.max_cost_sats} sats`);
  }

  // Check 3: Excessive consecutive thinks without tool use
  if (maxConsecutiveFound > maxConsecutiveThinks) {
    issues.push(`${maxConsecutiveFound} consecutive think cycles without tool use (limit: ${maxConsecutiveThinks})`);
  }

  const stats = {
    events: events.length,
    thinkRequests,
    thinkResponses,
    totalCostSats,
    iterations,
  };

  return { pass: issues.length === 0, issues, stats };
}

module.exports = { evaluateResult, evaluateArtifacts, compareWithBaseline, evaluateTranscript };
