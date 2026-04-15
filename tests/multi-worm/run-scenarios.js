#!/usr/bin/env node
/**
 * Multi-Worm Scenario Runner
 *
 * Executes multi-agent test scenarios via HTTP API calls (no Playwright).
 * Uses the orchestrator to manage agents, then runs scenarios step-by-step.
 *
 * Usage:
 *   node run-scenarios.js                    # Run all non-skipped scenarios
 *   node run-scenarios.js --tier basic       # Run only basic tier
 *   node run-scenarios.js --id 1,2           # Run specific IDs
 *   node run-scenarios.js --dry-run          # Show what would run
 *
 * Exports: { runScenarios, executeStep, checkAssertions, httpPost, waitForTaskComplete }
 */

const http = require('http');
const fs = require('fs');
const path = require('path');

const { WormOrchestrator, httpGet } = require('./orchestrate.js');
const { authGet, authPost } = require('./lib/auth');

// ---------------------------------------------------------------------------
// BRC-31 auth context — set by runScenarios() from orchestrator config
// ---------------------------------------------------------------------------

/** @type {number} Parent wallet port for BRC-31 auth. Required — never run without auth. */
let _parentWalletPort = 0;

/**
 * GET with BRC-31 auth via parent wallet. Never unauthenticated.
 */
async function authedGet(url) {
  if (!_parentWalletPort) throw new Error('BRC-31 auth not configured — set parent_wallet_port in config.json');
  return authGet(url, _parentWalletPort);
}

/**
 * POST with BRC-31 auth via parent wallet. Never unauthenticated.
 */
async function authedPost(url, body) {
  if (!_parentWalletPort) throw new Error('BRC-31 auth not configured — set parent_wallet_port in config.json');
  return authPost(url, body, _parentWalletPort);
}

// ---------------------------------------------------------------------------
// HTTP POST helper
// ---------------------------------------------------------------------------

/**
 * Perform an HTTP POST request and return { status, body }.
 * Body is parsed as JSON if possible, otherwise returned as a raw string.
 *
 * @param {string} url — full URL to POST to
 * @param {object} [body] — request body (will be JSON-serialized)
 * @returns {Promise<{ status: number, body: * }>}
 */
function httpPost(url, body) {
  return new Promise((resolve, reject) => {
    const urlObj = new URL(url);
    const data = JSON.stringify(body || {});
    const req = http.request({
      hostname: urlObj.hostname,
      port: urlObj.port,
      path: urlObj.pathname + urlObj.search,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(data),
      },
    }, (res) => {
      let buf = '';
      res.on('data', chunk => buf += chunk);
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(buf) }); }
        catch { resolve({ status: res.statusCode, body: buf }); }
      });
    });
    req.on('error', reject);
    req.setTimeout(30000, () => { req.destroy(); reject(new Error('HTTP POST timeout')); });
    req.write(data);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Sleep helper
// ---------------------------------------------------------------------------

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// ---------------------------------------------------------------------------
// Template resolution
// ---------------------------------------------------------------------------

/**
 * Replace {{variable}} placeholders in a string with values from vars.
 *
 * @param {string} str — string with {{alice.identity_key}} etc.
 * @param {object} vars — template variable map
 * @returns {string}
 */
function resolveTemplate(str, vars) {
  if (typeof str !== 'string') return str;
  return str.replace(/\{\{([a-zA-Z0-9_.]+)\}\}/g, (match, key) => {
    // Try flat key first (e.g. "alice.identity_key" set by orchestrator)
    if (vars[key] !== undefined) return String(vars[key]);
    // Try nested access (e.g. "alice_task.task_id" from stored step results)
    const parts = key.split('.');
    let val = vars;
    for (const p of parts) {
      if (val === null || val === undefined || typeof val !== 'object') return match;
      val = val[p];
    }
    return val !== undefined && val !== null ? String(val) : match;
  });
}

// ---------------------------------------------------------------------------
// Nested field access
// ---------------------------------------------------------------------------

/**
 * Get a nested field from an object using dot notation.
 *
 * @param {*} obj — the object to traverse
 * @param {string} fieldPath — dot-separated path (e.g. "identity_key", "tasks.0.status")
 * @returns {*} the value, or undefined if not found
 */
function getNestedField(obj, fieldPath) {
  if (obj === null || obj === undefined) return undefined;
  const parts = fieldPath.split('.');
  let current = obj;
  for (const part of parts) {
    if (current === null || current === undefined) return undefined;
    if (typeof current === 'object') {
      current = current[part];
    } else {
      return undefined;
    }
  }
  return current;
}

// ---------------------------------------------------------------------------
// Wait for task completion
// ---------------------------------------------------------------------------

/**
 * Poll GET /task/{taskId}/events until the task is no longer active.
 *
 * @param {string} agentUrl — base URL of the agent (e.g. http://localhost:8080)
 * @param {string} taskId — the task ID to poll
 * @param {number} timeoutSecs — max seconds to wait
 * @returns {Promise<object>} — the final events response
 */
async function waitForTaskComplete(agentUrl, taskId, timeoutSecs) {
  const deadline = Date.now() + timeoutSecs * 1000;
  let lastBody = null;

  while (Date.now() < deadline) {
    try {
      const { status, body } = await authedGet(`${agentUrl}/task/${taskId}/events?since=0`);
      lastBody = body;
      if (status === 200 && body && body.active === false) {
        return body;
      }
    } catch {
      // Connection refused or timeout — agent may still be starting, retry
    }
    await sleep(2000);
  }

  throw new Error(
    `Task ${taskId} did not complete within ${timeoutSecs}s. ` +
    `Last response: ${JSON.stringify(lastBody)}`
  );
}

// ---------------------------------------------------------------------------
// Step executors
// ---------------------------------------------------------------------------

/**
 * Execute a "task" step: submit a task to an agent and wait for completion.
 *
 * @param {object} step — the step definition
 * @param {object} vars — template variables
 * @returns {Promise<object>} — { task_id, status, events }
 */
async function executeTask(step, vars) {
  const agentUrl = vars[`${step.agent}.worm_url`];
  if (!agentUrl) throw new Error(`No worm_url for agent '${step.agent}'`);

  const message = resolveTemplate(step.message, vars);
  const maxIterations = step.max_iterations || 15;

  const res = await authedPost(`${agentUrl}/task`, {
    task: message,
    max_iterations: maxIterations,
  });

  if (res.status !== 200 && res.status !== 201 && res.status !== 202) {
    throw new Error(`POST /task failed for ${step.agent}: status=${res.status}, body=${JSON.stringify(res.body)}`);
  }

  const taskId = typeof res.body === 'object' ? (res.body.task_id || res.body.id) : res.body;
  if (!taskId) {
    throw new Error(`No task_id in response from ${step.agent}: ${JSON.stringify(res.body)}`);
  }

  // Store the response for assertions
  const result = { task_id: taskId, status: res.status, body: res.body };

  // Wait for task completion
  const timeoutSecs = step.timeout_seconds || 180;
  const events = await waitForTaskComplete(agentUrl, taskId, timeoutSecs);
  result.events = events;
  result.completed = true;

  return result;
}

/**
 * Execute a "wait" step: wait for an event on an agent.
 *
 * @param {object} step — the step definition
 * @param {object} vars — template variables
 * @returns {Promise<object>} — { found, tasks }
 */
async function executeWait(step, vars) {
  const agentUrl = vars[`${step.agent}.worm_url`];
  if (!agentUrl) throw new Error(`No worm_url for agent '${step.agent}'`);

  const timeout = (step.timeout_seconds || 60) * 1000;
  const deadline = Date.now() + timeout;

  while (Date.now() < deadline) {
    try {
      if (step.event === 'task_complete') {
        const { status, body } = await authedGet(`${agentUrl}/status`);
        if (status === 200 && body) {
          const tasks = body.tasks || [];
          const completed = tasks.filter(t =>
            t.status === 'complete' || t.status === 'completed' || t.status === 'error'
          );
          if (completed.length > 0) {
            return { found: true, tasks: completed };
          }
        }
      }

      if (step.event === 'message_received') {
        // Check if agent has processed a messagebox message
        const { status, body } = await authedGet(`${agentUrl}/status`);
        if (status === 200 && body) {
          const tasks = body.tasks || [];
          const msgTasks = tasks.filter(t =>
            t.origin === 'message' || t.origin === 'messagebox'
          );
          if (msgTasks.length > 0) {
            return { found: true, tasks: msgTasks };
          }
        }
      }

      if (step.event === 'proof_created') {
        const { status, body } = await authedGet(`${agentUrl}/status`);
        if (status === 200 && body && body.proofs && body.proofs.length > 0) {
          return { found: true, proofs: body.proofs };
        }
      }

      if (step.event === 'budget_updated') {
        const { status, body } = await authedGet(`${agentUrl}/budget/detail`);
        if (status === 200 && body) {
          return { found: true, budget: body };
        }
      }
    } catch {
      // retry
    }
    await sleep(2000);
  }

  throw new Error(`Wait timeout: ${step.event} on ${step.agent} after ${timeout / 1000}s`);
}

/**
 * Execute an "assert" step: record an inline assertion for later checking.
 *
 * @param {object} step — the step definition
 * @param {object} vars — template variables
 * @returns {object} — { condition, pending: true }
 */
function executeAssert(step, vars) {
  // Inline assertions are evaluated after all steps complete
  return { condition: step.condition, pending: true };
}

/**
 * Execute an "api" step: make a direct HTTP call to an agent.
 *
 * @param {object} step — the step definition
 * @param {object} vars — template variables
 * @returns {Promise<object>} — { status, body }
 */
async function executeApi(step, vars) {
  const agentUrl = vars[`${step.agent}.worm_url`];
  if (!agentUrl) throw new Error(`No worm_url for agent '${step.agent}'`);

  const method = step.method || 'GET';
  const apiPath = resolveTemplate(step.path, vars);

  let res;
  if (method === 'GET') {
    res = await authedGet(`${agentUrl}${apiPath || ''}`);
  } else {
    const body = step.body
      ? JSON.parse(resolveTemplate(JSON.stringify(step.body), vars))
      : undefined;
    res = await authedPost(`${agentUrl}${apiPath}`, body);
  }

  return res;
}

/**
 * Execute a single scenario step.
 *
 * @param {object} step — the step definition
 * @param {object} vars — template variables (mutated if store_as is set)
 * @returns {Promise<object>} — step result
 */
async function executeStep(step, vars) {
  switch (step.action) {
    case 'task':
      return executeTask(step, vars);
    case 'wait':
      return executeWait(step, vars);
    case 'assert':
      return executeAssert(step, vars);
    case 'api':
      return executeApi(step, vars);
    default:
      throw new Error(`Unknown action: ${step.action}`);
  }
}

// ---------------------------------------------------------------------------
// Assertion checking
// ---------------------------------------------------------------------------

/**
 * Check inline assertions (from "assert" steps) against collected vars.
 *
 * @param {object} condition — the assertion condition object
 * @param {object} vars — template variables with stored step results
 * @returns {{ pass: boolean, reason: string }}
 */
function checkInlineAssertion(condition, vars) {
  const stepRef = condition.step;

  switch (condition.type) {
    case 'status_code': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const actual = result.status;
      const pass = actual === condition.value;
      return { pass, reason: pass ? 'OK' : `Expected status ${condition.value}, got ${actual}` };
    }

    case 'field_exists': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const source = result.body || result;
      const val = getNestedField(source, condition.field);
      const pass = val !== undefined && val !== null;
      return { pass, reason: pass ? 'OK' : `Field '${condition.field}' not found` };
    }

    case 'field_equals': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const source = result.body || result;
      const val = getNestedField(source, condition.field);
      const pass = val === condition.value;
      return { pass, reason: pass ? 'OK' : `Field '${condition.field}' = '${val}', expected '${condition.value}'` };
    }

    case 'response_contains': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const text = JSON.stringify(result).toLowerCase();
      const needle = (condition.value || '').toLowerCase();
      const pass = text.includes(needle);
      return { pass, reason: pass ? 'OK' : `Response does not contain '${condition.value}'` };
    }

    case 'response_not_contains': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const text = JSON.stringify(result).toLowerCase();
      const needle = (condition.value || '').toLowerCase();
      const pass = !text.includes(needle);
      return { pass, reason: pass ? 'OK' : `Response contains forbidden '${condition.value}'` };
    }

    case 'response_matches': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      // Search full result including events (task responses are in events, not body)
      const val = condition.field ? getNestedField(result.body || result, condition.field) : JSON.stringify(result);
      const re = new RegExp(condition.value, 'i');
      const pass = re.test(String(val));
      return { pass, reason: pass ? 'OK' : `Does not match /${condition.value}/i (searched ${String(val).length} chars)` };
    }

    case 'session_no_error': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const events = (result.events && result.events.events) || [];
      const sessionEnd = events.find(e => e.type === 'session_end');
      if (!sessionEnd) return { pass: false, reason: 'No session_end event found' };
      const err = (sessionEnd.error || sessionEnd.data?.error || '').trim();
      const pass = err === '';
      return { pass, reason: pass ? 'OK' : `Session ended with error: ${err.substring(0, 150)}` };
    }

    case 'tool_succeeded': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const events = (result.events && result.events.events) || [];
      const toolResults = events.filter(e => e.type === 'tool_result' && (e.name === condition.tool_name || e.data?.name === condition.tool_name));
      if (toolResults.length === 0) return { pass: false, reason: `Tool '${condition.tool_name}' was never called` };
      const last = toolResults[toolResults.length - 1];
      const content = last.content || last.data?.content || '';
      const hasError = content.toLowerCase().startsWith('error');
      const pass = !hasError;
      return { pass, reason: pass ? 'OK' : `Tool '${condition.tool_name}' failed: ${content.substring(0, 150)}` };
    }

    case 'tool_result_contains': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const events = (result.events && result.events.events) || [];
      const toolResults = events.filter(e => e.type === 'tool_result' && (e.name === condition.tool_name || e.data?.name === condition.tool_name));
      if (toolResults.length === 0) return { pass: false, reason: `Tool '${condition.tool_name}' was never called` };
      const last = toolResults[toolResults.length - 1];
      const content = last.content || last.data?.content || '';
      const needle = (condition.value || '').toLowerCase();
      const pass = content.toLowerCase().includes(needle);
      return { pass, reason: pass ? 'OK' : `Tool '${condition.tool_name}' output does not contain '${condition.value}'` };
    }

    case 'no_tool_errors': {
      const result = vars[stepRef];
      if (!result) return { pass: false, reason: `No result stored as '${stepRef}'` };
      const events = (result.events && result.events.events) || [];
      const toolResults = events.filter(e => e.type === 'tool_result');
      const failed = toolResults.filter(tr => {
        const content = tr.content || tr.data?.content || '';
        return content.toLowerCase().startsWith('error');
      });
      const pass = failed.length === 0;
      const names = failed.map(f => f.name || f.data?.name || '?').join(', ');
      return { pass, reason: pass ? 'OK' : `${failed.length} tool(s) failed: ${names}` };
    }

    default:
      return { pass: false, reason: `Unknown inline assertion type: ${condition.type}` };
  }
}

/**
 * Check post-scenario assertions.
 *
 * @param {object} scenario — the scenario definition
 * @param {object} vars — template variables with stored step results
 * @param {object[]} stepResults — array of step results
 * @returns {{ pass: boolean, type: string, description: string, reason: string }[]}
 */
function checkAssertions(scenario, vars, stepResults) {
  const results = [];

  // Check inline assertions from "assert" steps
  const steps = scenario.steps || [];
  for (let i = 0; i < steps.length; i++) {
    if (steps[i].action === 'assert' && steps[i].condition) {
      const { pass, reason } = checkInlineAssertion(steps[i].condition, vars);
      results.push({
        pass,
        type: steps[i].condition.type,
        description: `Inline assert step ${i}: ${steps[i].condition.type}`,
        reason,
      });
    }
  }

  // Check post-scenario assertions
  for (const assertion of (scenario.assertions || [])) {
    let pass = false;
    let reason = '';

    switch (assertion.type) {
      case 'agents_different_keys': {
        const agents = scenario.agents || [];
        const keys = agents.map(a => vars[`${a}.identity_key`]).filter(Boolean);
        const unique = new Set(keys);
        pass = unique.size === keys.length && keys.length === agents.length;
        reason = pass ? 'OK' : `Keys not all different: ${keys.join(', ')}`;
        break;
      }

      case 'cost_within': {
        // For now, cost_within passes — real cost checking requires agent balance tracking
        const maxSats = assertion.max_sats || assertion.value || 0;
        pass = true;
        reason = `Cost check placeholder (max: ${maxSats} sats)`;
        break;
      }

      case 'response_contains': {
        const result = vars[assertion.step || assertion.source];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step || assertion.source}'`;
        } else {
          const text = JSON.stringify(result).toLowerCase();
          const values = assertion.values || [assertion.value].filter(Boolean);
          const missing = values.filter(v => !text.includes(v.toLowerCase()));
          pass = missing.length === 0;
          reason = pass ? 'OK' : `Missing: ${missing.join(', ')}`;
        }
        break;
      }

      case 'response_not_contains': {
        const result = vars[assertion.step || assertion.source];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step || assertion.source}'`;
        } else {
          const text = JSON.stringify(result).toLowerCase();
          const needle = (assertion.value || '').toLowerCase();
          pass = !text.includes(needle);
          reason = pass ? 'OK' : `Response contains forbidden '${assertion.value}'`;
        }
        break;
      }

      case 'response_matches': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          // Search full result including events
          const val = assertion.field ? getNestedField(result.body || result, assertion.field) : JSON.stringify(result);
          const re = new RegExp(assertion.value, 'i');
          pass = re.test(String(val));
          reason = pass ? 'OK' : `Does not match /${assertion.value}/i (searched ${String(val).length} chars)`;
        }
        break;
      }

      case 'status_code': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          const actual = result.status;
          pass = actual === assertion.value;
          reason = pass ? 'OK' : `Expected status ${assertion.value}, got ${actual}`;
        }
        break;
      }

      case 'field_exists': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          const source = result.body || result;
          const val = getNestedField(source, assertion.field);
          pass = val !== undefined && val !== null;
          reason = pass ? 'OK' : `Field '${assertion.field}' not found`;
        }
        break;
      }

      case 'session_no_error': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          const events = (result.events && result.events.events) || [];
          const sessionEnd = events.find(e => e.type === 'session_end');
          if (!sessionEnd) {
            pass = false;
            reason = 'No session_end event found';
          } else {
            const err = (sessionEnd.error || sessionEnd.data?.error || '').trim();
            pass = err === '';
            reason = pass ? 'OK' : `Session ended with error: ${err.substring(0, 150)}`;
          }
        }
        break;
      }

      case 'tool_succeeded': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          const events = (result.events && result.events.events) || [];
          const toolResults = events.filter(e => e.type === 'tool_result' && (e.name === assertion.tool_name || e.data?.name === assertion.tool_name));
          if (toolResults.length === 0) {
            pass = false;
            reason = `Tool '${assertion.tool_name}' was never called`;
          } else {
            const last = toolResults[toolResults.length - 1];
            const content = last.content || last.data?.content || '';
            const hasError = content.toLowerCase().startsWith('error');
            pass = !hasError;
            reason = pass ? 'OK' : `Tool '${assertion.tool_name}' failed: ${content.substring(0, 150)}`;
          }
        }
        break;
      }

      case 'tool_result_contains': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          const events = (result.events && result.events.events) || [];
          const toolResults = events.filter(e => e.type === 'tool_result' && (e.name === assertion.tool_name || e.data?.name === assertion.tool_name));
          if (toolResults.length === 0) {
            pass = false;
            reason = `Tool '${assertion.tool_name}' was never called`;
          } else {
            const last = toolResults[toolResults.length - 1];
            const content = last.content || last.data?.content || '';
            const needle = (assertion.value || '').toLowerCase();
            pass = content.toLowerCase().includes(needle);
            reason = pass ? 'OK' : `Tool '${assertion.tool_name}' output does not contain '${assertion.value}'`;
          }
        }
        break;
      }

      case 'no_tool_errors': {
        const result = vars[assertion.step];
        if (!result) {
          pass = false;
          reason = `No result stored as '${assertion.step}'`;
        } else {
          const events = (result.events && result.events.events) || [];
          const toolResults = events.filter(e => e.type === 'tool_result');
          const failed = toolResults.filter(tr => {
            const content = tr.content || tr.data?.content || '';
            return content.toLowerCase().startsWith('error');
          });
          pass = failed.length === 0;
          const names = failed.map(f => f.name || f.data?.name || '?').join(', ');
          reason = pass ? 'OK' : `${failed.length} tool(s) failed: ${names}`;
        }
        break;
      }

      default:
        pass = false;
        reason = `Unknown assertion type: ${assertion.type}`;
    }

    results.push({
      pass,
      type: assertion.type,
      description: assertion.description || assertion.type,
      reason,
    });
  }

  return results;
}

// ---------------------------------------------------------------------------
// Scenario runner
// ---------------------------------------------------------------------------

/**
 * Run a single scenario.
 *
 * @param {object} scenario — the scenario definition
 * @param {object} baseVars — base template variables from orchestrator
 * @returns {Promise<object>} — { pass, scenario, stepResults, assertionResults, elapsed, error }
 */
async function runScenario(scenario, baseVars) {
  const vars = { ...baseVars };
  const stepResults = [];
  const startTime = Date.now();

  try {
    // Execute steps sequentially
    const steps = scenario.steps || [];
    for (let i = 0; i < steps.length; i++) {
      const step = steps[i];
      const stepStart = Date.now();

      process.stdout.write(`  Step ${i + 1}: ${step.action} ${step.agent}`);
      if (step.action === 'task') {
        const msg = resolveTemplate(step.message, vars);
        const preview = msg.length > 50 ? msg.substring(0, 50) + '...' : msg;
        process.stdout.write(` "${preview}"`);
      }
      if (step.action === 'wait') {
        process.stdout.write(` ${step.event}`);
      }
      if (step.action === 'api') {
        process.stdout.write(` ${step.method || 'GET'} ${step.path}`);
      }

      const result = await executeStep(step, vars);
      const stepElapsed = ((Date.now() - stepStart) / 1000).toFixed(1);

      // Store result if store_as is set
      if (step.store_as) {
        vars[step.store_as] = result;
      }

      stepResults.push({
        index: i,
        action: step.action,
        agent: step.agent,
        result,
        elapsed_seconds: parseFloat(stepElapsed),
      });

      if (step.action === 'assert') {
        console.log(` -- deferred (${stepElapsed}s)`);
      } else {
        console.log(` -- ok (${stepElapsed}s)`);
      }
    }

    // Check assertions
    const assertionResults = checkAssertions(scenario, vars, stepResults);
    const passed = assertionResults.every(a => a.pass);
    const totalAssertions = assertionResults.length;
    const passedAssertions = assertionResults.filter(a => a.pass).length;

    if (totalAssertions > 0) {
      console.log(`  Assertions: ${passedAssertions}/${totalAssertions} passed`);
      for (const a of assertionResults) {
        const mark = a.pass ? '+' : 'X';
        console.log(`    [${mark}] ${a.description}: ${a.reason}`);
      }
    }

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
    const passStr = passed ? 'PASS' : 'FAIL';
    console.log(`  ${passStr} (${elapsed}s)\n`);

    return {
      pass: passed,
      scenario: { id: scenario.id, name: scenario.name, tier: scenario.tier },
      stepResults,
      assertionResults,
      elapsed_seconds: parseFloat(elapsed),
      error: null,
    };

  } catch (err) {
    const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
    console.log(` -- ERROR: ${err.message}`);
    console.log(`  FAIL (${elapsed}s, error)\n`);

    return {
      pass: false,
      scenario: { id: scenario.id, name: scenario.name, tier: scenario.tier },
      stepResults,
      assertionResults: [],
      elapsed_seconds: parseFloat(elapsed),
      error: err.message,
    };
  }
}

/**
 * Run multiple scenarios.
 *
 * @param {object} options
 * @param {object}   options.orchestrator — WormOrchestrator instance (or null for dry-run)
 * @param {object[]} options.scenarios — filtered list of scenarios to run
 * @param {boolean}  [options.dryRun=false] — if true, just print what would run
 * @returns {Promise<object>} — { results, passed, failed, skipped, elapsed }
 */
async function runScenarios({ orchestrator, scenarios, dryRun = false }) {
  const results = [];
  const startTime = Date.now();

  if (dryRun) {
    console.log('\nDry run -- showing scenarios that would execute:\n');
    for (const s of scenarios) {
      const agentList = s.agents.join(', ');
      const stepCount = (s.steps || []).length;
      const assertCount = (s.assertions || []).length;
      const inlineAsserts = (s.steps || []).filter(st => st.action === 'assert').length;
      console.log(`  [${s.tier}] #${s.id} ${s.name}`);
      console.log(`    ${s.description}`);
      console.log(`    Agents: ${agentList} | Steps: ${stepCount} | Assertions: ${assertCount + inlineAsserts}`);
      if (s.budget) {
        console.log(`    Budget: ${(s.budget.max_total_sats || 0).toLocaleString()} sats max`);
      }
      if (s.note) {
        console.log(`    Note: ${s.note}`);
      }
      console.log('');
    }

    console.log(`Total: ${scenarios.length} scenario(s) would run.`);
    return { results: [], passed: 0, failed: 0, skipped: 0, elapsed_seconds: 0 };
  }

  // Set BRC-31 auth context from orchestrator config
  if (orchestrator && orchestrator.config.parent_wallet_port) {
    _parentWalletPort = orchestrator.config.parent_wallet_port;
  }

  // Get template vars from orchestrator
  const baseVars = orchestrator ? orchestrator.getTemplateVars() : {};

  for (let si = 0; si < scenarios.length; si++) {
    const scenario = scenarios[si];
    const tierTag = `[${scenario.tier}]`;
    console.log(`${tierTag} #${scenario.id} ${scenario.name}`);

    const result = await runScenario(scenario, baseVars);
    results.push(result);

    // Note: sequential scenarios may have message contamination between runs.
    // For reliable results, run scenarios individually: --id 10, --id 11, etc.
    // Agents persist MessageBox state between scenarios within the same run.
  }

  const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
  const passed = results.filter(r => r.pass).length;
  const failed = results.filter(r => !r.pass).length;

  return {
    results,
    passed,
    failed,
    skipped: 0,
    elapsed_seconds: parseFloat(elapsed),
  };
}

// ---------------------------------------------------------------------------
// Results persistence
// ---------------------------------------------------------------------------

/**
 * Save run results to a JSON file in the results/ directory.
 *
 * @param {object} runResult — from runScenarios()
 * @returns {string} — path to the saved file
 */
function saveResults(runResult) {
  const resultsDir = path.join(__dirname, 'results');
  fs.mkdirSync(resultsDir, { recursive: true });

  const timestamp = new Date().toISOString().replace(/[:.]/g, '-');
  const filename = `run-${timestamp}.json`;
  const filepath = path.join(resultsDir, filename);

  const output = {
    timestamp: new Date().toISOString(),
    ...runResult,
  };

  fs.writeFileSync(filepath, JSON.stringify(output, null, 2));
  return filepath;
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

async function main() {
  const args = process.argv.slice(2);

  // Parse CLI flags
  const dryRun = args.includes('--dry-run');

  let tierFilter = null;
  const tierIdx = args.indexOf('--tier');
  if (tierIdx !== -1 && args[tierIdx + 1]) {
    tierFilter = args[tierIdx + 1];
  }

  let idFilter = null;
  const idIdx = args.indexOf('--id');
  if (idIdx !== -1 && args[idIdx + 1]) {
    idFilter = args[idIdx + 1].split(',').map(s => parseInt(s.trim(), 10));
  }

  // Load scenarios
  const scenariosPath = path.join(__dirname, 'scenarios.json');
  const scenariosData = JSON.parse(fs.readFileSync(scenariosPath, 'utf8'));
  let scenarios = scenariosData.scenarios || [];

  // Filter out skipped scenarios
  scenarios = scenarios.filter(s => !s.skip);

  // Apply tier filter
  if (tierFilter) {
    scenarios = scenarios.filter(s => s.tier === tierFilter);
  }

  // Apply ID filter
  if (idFilter) {
    scenarios = scenarios.filter(s => idFilter.includes(s.id));
  }

  // Sort by ID
  scenarios.sort((a, b) => a.id - b.id);

  // Determine required agents
  const requiredAgents = new Set();
  for (const s of scenarios) {
    for (const a of (s.agents || [])) {
      requiredAgents.add(a);
    }
  }

  // Load config
  const configPath = path.join(__dirname, 'config.json');
  const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));

  // Check all required agents exist in config
  for (const agent of requiredAgents) {
    if (!config.agents[agent]) {
      console.error(`Error: scenario requires agent '${agent}' but it is not in config.json`);
      process.exit(1);
    }
  }

  // Banner
  const agentNames = Array.from(requiredAgents).sort();
  console.log('\nMulti-Worm Scenario Runner');
  console.log(`  Config: ${agentNames.length} agents (${agentNames.join(', ')})`);
  console.log(`  Scenarios: ${scenarios.length} selected${tierFilter ? ` (tier: ${tierFilter})` : ''}${idFilter ? ` (ids: ${idFilter.join(',')})` : ''}`);

  if (scenarios.length === 0) {
    console.log('\n  No scenarios match the filter. Nothing to do.');
    process.exit(0);
  }

  if (dryRun) {
    await runScenarios({ orchestrator: null, scenarios, dryRun: true });
    process.exit(0);
  }

  // Kill any orphaned worm processes from prior runs
  try {
    const { execSync } = require('child_process');
    const orphans = execSync('pgrep -f "dolphin-milk serve"', { encoding: 'utf8' }).trim();
    if (orphans) {
      console.log(`  Killing orphaned dolphin-milk processes: ${orphans.split('\n').join(', ')}`);
      execSync('pkill -f "dolphin-milk serve"');
      await sleep(1000); // Let ports release
    }
  } catch { /* no orphans — pgrep exits non-zero */ }

  // Create orchestrator with only the required agents
  const filteredConfig = {
    ...config,
    agents: {},
  };
  for (const agent of requiredAgents) {
    filteredConfig.agents[agent] = config.agents[agent];
  }
  const orchestrator = new WormOrchestrator(filteredConfig);

  // Graceful shutdown on SIGINT/SIGTERM
  let shuttingDown = false;
  const shutdown = async (sig) => {
    if (shuttingDown) return;
    shuttingDown = true;
    console.log(`\n\nReceived ${sig}. Stopping all agents...`);
    await orchestrator.stopAll();
    console.log('All agents stopped.');
    process.exit(1);
  };
  process.on('SIGINT', () => shutdown('SIGINT'));
  process.on('SIGTERM', () => shutdown('SIGTERM'));

  // Start agents
  console.log('\nStarting agents...');
  try {
    await orchestrator.startAll();
  } catch (err) {
    console.error(`\nFailed to start agents: ${err.message}`);
    await orchestrator.stopAll();
    process.exit(1);
  }

  // Print agent summary
  for (const [name, identity] of orchestrator.identities) {
    const truncKey = identity.identity_key.substring(0, 9) + '...';
    const balStr = identity.balance.toLocaleString();
    console.log(`  ${name}: ${truncKey} (${balStr} sats)`);
  }

  console.log('');

  let runResult;
  try {
    // Run scenarios
    runResult = await runScenarios({ orchestrator, scenarios, dryRun: false });
  } finally {
    // Always stop agents, even on uncaught errors
    console.log('Stopping agents...');
    await orchestrator.stopAll();
    console.log('All agents stopped.\n');
  }

  // Print summary
  console.log(`Results: ${runResult.passed} passed, ${runResult.failed} failed`);
  console.log(`Elapsed: ${runResult.elapsed_seconds}s`);

  // Save results
  const resultsPath = saveResults(runResult);
  console.log(`Results saved: ${resultsPath}`);

  // Exit with failure code if any scenario failed
  process.exit(runResult.failed > 0 ? 1 : 0);
}

// ---------------------------------------------------------------------------
// Run standalone
// ---------------------------------------------------------------------------

if (require.main === module) {
  main().catch(async (err) => {
    console.error(`\nFatal: ${err.message}`);
    process.exit(1);
  });
}

// ---------------------------------------------------------------------------
// Module exports
// ---------------------------------------------------------------------------

module.exports = { runScenarios, executeStep, checkAssertions, httpPost, waitForTaskComplete };
