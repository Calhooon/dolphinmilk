#!/usr/bin/env node
/**
 * Two-Agent Handshake E2E Test (Issue #314)
 *
 * Two running dolphin-milk servers discover each other via the overlay,
 * exchange a task via MessageBox, process it, and reply.
 * Real sats, real overlay, real MessageBox, real LLM inference.
 *
 * Flow:
 *   1. Start Agent A on port 8081 (wallet 3322)
 *   2. Start Agent B on port 8082 (wallet 3323)
 *   3. Wait for both /health to return 200
 *   4. Get identity keys from /agent
 *   5. POST /task to Agent A: discover agents via overlay, send a message, report
 *   6. Poll Agent A until task completes
 *   7. Poll Agent B for a task to appear and complete
 *   8. Inspect both transcripts
 *   9. Print everything for human inspection
 *  10. Kill both servers
 *
 * Prerequisites:
 *   - cargo build --release (binary at ./target/release/dolphin-milk)
 *   - Wallet A on localhost:3322 (funded BSV wallet)
 *   - Wallet B on localhost:3323 (funded BSV wallet)
 *   - Live internet (overlay + messagebox)
 *
 * Usage:
 *   node test_two_agent_handshake.js
 */

const { spawn } = require('child_process');
const http = require('http');
const path = require('path');
const fs = require('fs');

// BRC-31 auth library for authenticated requests to worm servers
const { authGet, authPost, clearAuthCache } = require('./lib/auth');

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const AGENT_A_PORT = 8081;
const AGENT_B_PORT = 8082;
const WALLET_A_PORT = 3322;
const WALLET_B_PORT = 3323;
const PARENT_WALLET_PORT = 3321;

const PROJECT_ROOT = path.resolve(__dirname, '../../');
const BINARY = path.join(PROJECT_ROOT, 'target/release/dolphin-milk');
const WORKSPACE_A = path.join(PROJECT_ROOT, 'test-workspaces/handshake-agent-a');
const WORKSPACE_B = path.join(PROJECT_ROOT, 'test-workspaces/handshake-agent-b');

// Timeouts
const HEALTH_TIMEOUT_MS = 60000;
const TASK_A_COMPLETE_TIMEOUT_MS = 300000;  // 5 min for Agent A
const TASK_B_APPEAR_TIMEOUT_MS = 300000;    // 5 min for Agent B task to appear
const TASK_B_COMPLETE_TIMEOUT_MS = 300000;  // 5 min for Agent B task to complete

// Track server processes for cleanup
let serverA = null;
let serverB = null;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatSats(sats) {
  return sats ? sats.toLocaleString() + ' sats' : '0 sats';
}

function formatMs(ms) {
  return (ms / 1000).toFixed(1) + 's';
}

function sleep(ms) {
  return new Promise(r => setTimeout(r, ms));
}

function readLogTail(filepath, chars = 3000) {
  try {
    return fs.readFileSync(filepath, 'utf8').slice(-chars);
  } catch { return ''; }
}

// ---------------------------------------------------------------------------
// HTTP helpers (no BRC-31 auth — dev mode with no parent key)
// ---------------------------------------------------------------------------

function httpGet(url) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(data) }); }
        catch { resolve({ status: res.statusCode, body: data }); }
      });
    });
    req.on('error', reject);
    req.setTimeout(15000, () => { req.destroy(); reject(new Error('timeout')); });
  });
}

function httpPost(url, body) {
  return new Promise((resolve, reject) => {
    const urlObj = new URL(url);
    const data = JSON.stringify(body || {});
    const req = http.request({
      hostname: urlObj.hostname,
      port: urlObj.port || 80,
      path: urlObj.pathname + (urlObj.search || ''),
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(data),
      },
    }, (res) => {
      let raw = '';
      res.on('data', chunk => raw += chunk);
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(raw), headers: res.headers }); }
        catch { resolve({ status: res.statusCode, body: raw, headers: res.headers }); }
      });
    });
    req.on('error', reject);
    req.setTimeout(30000, () => { req.destroy(); reject(new Error('timeout')); });
    req.write(data);
    req.end();
  });
}

function walletPost(walletPort, endpoint, body) {
  return new Promise((resolve, reject) => {
    const data = JSON.stringify(body);
    const req = http.request({
      hostname: '127.0.0.1',
      port: walletPort,
      path: `/${endpoint}`,
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Origin': `http://localhost:${walletPort}`,
        'Content-Length': Buffer.byteLength(data),
      },
    }, (res) => {
      let raw = '';
      res.on('data', chunk => raw += chunk);
      res.on('end', () => {
        try {
          const parsed = JSON.parse(raw);
          if (parsed.error) reject(new Error(`Wallet ${endpoint}: ${JSON.stringify(parsed.error)}`));
          else resolve(parsed);
        } catch {
          reject(new Error(`Wallet ${endpoint}: bad response: ${raw.substring(0, 200)}`));
        }
      });
    });
    req.on('error', reject);
    req.setTimeout(15000, () => { req.destroy(); reject(new Error(`Wallet ${endpoint}: timeout`)); });
    req.write(data);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

function startAgent(name, port, walletPort, workspace, parentKey, agentName) {
  return new Promise(async (resolve, reject) => {
    if (!fs.existsSync(BINARY)) {
      return reject(new Error(`Binary not found at ${BINARY}. Run \`cargo build --release\` first.`));
    }

    fs.mkdirSync(workspace, { recursive: true });

    const stdoutLog = fs.openSync(path.join(workspace, 'server-stdout.log'), 'w');
    const stderrLog = fs.openSync(path.join(workspace, 'server-stderr.log'), 'w');

    console.log(`  Starting ${name} on port ${port} (wallet ${walletPort})...`);

    const proc = spawn(
      BINARY,
      ['serve', '--port', String(port), '--workspace', workspace],
      {
        cwd: PROJECT_ROOT,
        env: {
          ...process.env,
          DOLPHIN_MILK_WALLET_URL: `http://localhost:${walletPort}`,
          DOLPHIN_MILK_HEARTBEAT_ENABLED: 'true',
          DOLPHIN_MILK_HEARTBEAT_POLL_SECS: '15',
          DOLPHIN_MILK_OVERLAY_ENABLED: 'true',
          DOLPHIN_MILK_AGENT_NAME: agentName,
          DOLPHIN_MILK_LOG_LEVEL: 'INFO',
          DOLPHIN_MILK_PARENT_KEY: parentKey,
          DOLPHIN_MILK_PARENT_WALLET_URL: `http://localhost:${PARENT_WALLET_PORT}`,
        },
        stdio: ['ignore', stdoutLog, stderrLog],
        detached: false,
      }
    );

    // Wait for health check
    const deadline = Date.now() + HEALTH_TIMEOUT_MS;
    let lastError = null;

    while (Date.now() < deadline) {
      await sleep(1000);
      try {
        const { status, body } = await httpGet(`http://localhost:${port}/health`);
        if (status === 200 && body?.status === 'ok' && body?.wallet_connected === true) {
          console.log(`  ${name} is healthy and wallet connected.`);
          return resolve(proc);
        }
        lastError = `status=${status}, body=${JSON.stringify(body)}`;
      } catch (e) {
        lastError = e.message;
      }
    }

    const stderr = readLogTail(path.join(workspace, 'server-stderr.log'));
    reject(new Error(`${name} health check failed after ${HEALTH_TIMEOUT_MS}ms. Last: ${lastError}\nStderr tail: ${stderr}`));
  });
}

function stopAgent(proc, name) {
  if (!proc) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      try { proc.kill('SIGKILL'); } catch {}
    }, 5000);
    proc.on('exit', () => {
      clearTimeout(timer);
      console.log(`  ${name} stopped.`);
      resolve();
    });
    try { proc.kill('SIGTERM'); } catch {}
  });
}

async function stopAll() {
  await Promise.all([
    stopAgent(serverA, 'Agent A'),
    stopAgent(serverB, 'Agent B'),
  ]);
}

// ---------------------------------------------------------------------------
// Polling helpers
// ---------------------------------------------------------------------------

// Auth uses the parent wallet (MetaNet Client on 3321) for all agent requests.
// This matches how the production UI works — the wallet operator signs requests.

async function getAgentInfo(port) {
  const { status, body } = await authGet(`http://localhost:${port}/agent`, PARENT_WALLET_PORT);
  if (status !== 200) throw new Error(`/agent returned ${status}: ${JSON.stringify(body)}`);
  return body;
}

async function getTaskList(port) {
  const { status, body } = await authGet(`http://localhost:${port}/tasks`, PARENT_WALLET_PORT);
  if (status === 200 && body?.tasks) return body.tasks;
  return [];
}

async function getTaskDetail(port, taskId) {
  try {
    const { status, body } = await authGet(`http://localhost:${port}/task/${taskId}`, PARENT_WALLET_PORT);
    if (status === 200) return body;
    return null;
  } catch { return null; }
}

async function getTaskAudit(port, taskId) {
  try {
    const { status, body } = await authGet(`http://localhost:${port}/task/${taskId}/audit`, PARENT_WALLET_PORT);
    if (status === 200) return body;
    return null;
  } catch { return null; }
}

async function submitTask(port, message) {
  const { status, body } = await authPost(`http://localhost:${port}/task`, { task: message }, PARENT_WALLET_PORT);
  if (status !== 200 && status !== 201 && status !== 202) {
    throw new Error(`POST /task returned ${status}: ${JSON.stringify(body)}`);
  }
  return body;
}

async function pollTaskComplete(port, taskId, timeoutMs, label) {
  const startMs = Date.now();
  const deadline = startMs + timeoutMs;
  let pollCount = 0;
  while (Date.now() < deadline) {
    pollCount++;
    try {
      const task = await getTaskDetail(port, taskId);
      if (task) {
        const status = (task.status || '').toLowerCase();
        if (status === 'complete' || status === 'error' || status === 'cancelled') {
          console.log(`  ${label}: completed after ${pollCount} polls (${formatMs(Date.now() - startMs)})`);
          return task;
        }
      }
    } catch (e) {
      if (pollCount <= 2) console.log(`  ${label} poll error: ${e.message}`);
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  return null;
}

async function pollForNewTask(port, baselineIds, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  let pollCount = 0;
  while (Date.now() < deadline) {
    pollCount++;
    try {
      const tasks = await getTaskList(port);
      const newTask = tasks.find(t => !baselineIds.has(t.id));
      if (newTask) {
        console.log(`  ${label}: new task found after ${pollCount} polls`);
        return newTask;
      }
    } catch (e) {
      if (pollCount <= 2) console.log(`  ${label} poll error: ${e.message}`);
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  return null;
}

// ---------------------------------------------------------------------------
// Transcript inspector
// ---------------------------------------------------------------------------

function inspectAudit(audit, label) {
  if (!audit) {
    console.log(`  ${label}: No audit data available`);
    return {};
  }

  const events = audit.events || [];
  console.log(`  ${label}: ${events.length} events`);

  const eventTypes = {};
  for (const ev of events) {
    const type = ev.event_type || ev.type || 'unknown';
    eventTypes[type] = (eventTypes[type] || 0) + 1;
  }
  console.log(`  Event types:`);
  for (const [type, count] of Object.entries(eventTypes).sort((a, b) => b[1] - a[1])) {
    console.log(`    ${type}: ${count}`);
  }

  const thinkEvents = events.filter(e => (e.event_type || e.type) === 'think_response');
  console.log(`  LLM calls:  ${thinkEvents.length}`);

  const toolEvents = events.filter(e => (e.event_type || e.type) === 'tool_call');
  const toolNames = [];
  if (toolEvents.length > 0) {
    console.log(`  Tool calls: ${toolEvents.length}`);
    for (const tc of toolEvents) {
      const name = tc.tool_name || tc.data?.name || tc.data?.tool_name || tc.name || 'unknown';
      toolNames.push(name);
      const args = JSON.stringify(tc.data?.arguments || tc.arguments || {}).substring(0, 150);
      console.log(`    - ${name} ${args}`);
    }
  }

  const proofEvents = events.filter(e =>
    ['checkpoint_created', 'proof_created'].includes(e.event_type || e.type)
  );
  console.log(`  Proofs:     ${proofEvents.length}`);

  if (audit.summary) {
    console.log(`  Summary:`);
    console.log(`    Iterations: ${audit.summary.iterations || '?'}`);
    console.log(`    Sats:       ${formatSats(audit.summary.total_sats_spent || audit.summary.sats_spent)}`);
    console.log(`    Model:      ${audit.summary.model || '?'}`);
  }

  return { thinkEvents, toolEvents, proofEvents, eventTypes, toolNames: toolNames || [] };
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

async function main() {
  const startTime = Date.now();
  console.log('='.repeat(80));
  console.log('  TWO-AGENT HANDSHAKE E2E TEST (Issue #314)');
  console.log('  Real sats. Real overlay. Real MessageBox. Real LLM inference.');
  console.log('='.repeat(80));
  console.log();

  // -----------------------------------------------------------------------
  // Gate 1: Verify prerequisites
  // -----------------------------------------------------------------------
  console.log('[Gate 1] Verifying prerequisites...');

  if (!fs.existsSync(BINARY)) {
    console.error(`FATAL: Binary not found at ${BINARY}. Run \`cargo build --release\` first.`);
    process.exit(1);
  }
  console.log(`  Binary: ${BINARY}`);

  // Check wallet A
  try {
    const resp = await walletPost(WALLET_A_PORT, 'getPublicKey', { identityKey: true });
    console.log(`  Wallet A (:${WALLET_A_PORT}): ${resp.publicKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet A not reachable on port ${WALLET_A_PORT}: ${e.message}`);
    process.exit(1);
  }

  // Check wallet B
  try {
    const resp = await walletPost(WALLET_B_PORT, 'getPublicKey', { identityKey: true });
    console.log(`  Wallet B (:${WALLET_B_PORT}): ${resp.publicKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet B not reachable on port ${WALLET_B_PORT}: ${e.message}`);
    process.exit(1);
  }

  // Get parent key from MetaNet Client for BRC-31 auth
  let parentKey;
  try {
    const resp = await walletPost(PARENT_WALLET_PORT, 'getPublicKey', { identityKey: true });
    parentKey = resp.publicKey;
    console.log(`  Parent  (:${PARENT_WALLET_PORT}): ${parentKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Parent wallet not reachable on port ${PARENT_WALLET_PORT}: ${e.message}`);
    process.exit(1);
  }

  console.log('  All prerequisites met.\n');

  // -----------------------------------------------------------------------
  // Gate 2: Start both agents
  // -----------------------------------------------------------------------
  console.log('[Gate 2] Starting both agents...');

  clearAuthCache(); // Fresh BRC-31 sessions

  try {
    serverA = await startAgent('Agent A', AGENT_A_PORT, WALLET_A_PORT, WORKSPACE_A, parentKey, 'handshake-alpha');
  } catch (e) {
    console.error(`FATAL: Could not start Agent A: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  try {
    serverB = await startAgent('Agent B', AGENT_B_PORT, WALLET_B_PORT, WORKSPACE_B, parentKey, 'handshake-bravo');
  } catch (e) {
    console.error(`FATAL: Could not start Agent B: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  console.log('  Both agents running.\n');

  // -----------------------------------------------------------------------
  // Gate 3: Get identity keys
  // -----------------------------------------------------------------------
  console.log('[Gate 3] Getting agent identities...');

  let agentAInfo, agentBInfo;
  try {
    agentAInfo = await getAgentInfo(AGENT_A_PORT);
    console.log(`  Agent A identity: ${agentAInfo.identity_key?.substring(0, 24)}...`);
    console.log(`  Agent A balance:  ${formatSats(agentAInfo.balance)}`);
    console.log(`  Agent A model:    ${agentAInfo.default_model || 'unknown'}`);
  } catch (e) {
    console.error(`FATAL: Cannot get Agent A identity: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  try {
    agentBInfo = await getAgentInfo(AGENT_B_PORT);
    console.log(`  Agent B identity: ${agentBInfo.identity_key?.substring(0, 24)}...`);
    console.log(`  Agent B balance:  ${formatSats(agentBInfo.balance)}`);
    console.log(`  Agent B model:    ${agentBInfo.default_model || 'unknown'}`);
  } catch (e) {
    console.error(`FATAL: Cannot get Agent B identity: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  if (agentAInfo.identity_key === agentBInfo.identity_key) {
    console.error('FATAL: Both agents have the same identity key! They must use different wallets.');
    await stopAll();
    process.exit(1);
  }
  console.log('  Identity keys are different (good).\n');

  // -----------------------------------------------------------------------
  // Gate 4: Get baseline task lists
  // -----------------------------------------------------------------------
  console.log('[Gate 4] Getting baseline task lists...');

  const baselineA = new Set();
  const baselineB = new Set();
  try {
    (await getTaskList(AGENT_A_PORT)).forEach(t => baselineA.add(t.id));
    console.log(`  Agent A baseline: ${baselineA.size} tasks`);
  } catch (e) {
    console.log(`  Agent A baseline: error (${e.message}), assuming 0`);
  }
  try {
    (await getTaskList(AGENT_B_PORT)).forEach(t => baselineB.add(t.id));
    console.log(`  Agent B baseline: ${baselineB.size} tasks`);
  } catch (e) {
    console.log(`  Agent B baseline: error (${e.message}), assuming 0`);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 5: Submit task to Agent A
  // -----------------------------------------------------------------------
  console.log('[Gate 5] Submitting task to Agent A...');

  const taskMessage = [
    'Use overlay_lookup with service "ls_agent" and query {"findByCapability": "llm"} to discover agents that have LLM capabilities.',
    `From the results, pick an agent whose identity_key is DIFFERENT from your own identity key. You can find your own identity key using wallet_identity. Do NOT message yourself.`,
    'Use the discovered identity_key from the overlay results as the recipient in send_message. CRITICAL: use message_box "task_inbox" (this is for actionable work — status_inbox and results_inbox are for metadata only and will NOT be processed by the recipient). Use body {"question": "What is the capital of France? Reply with just the city name."}.',
    'Report the overlay lookup results (including the discovered identity key) and confirmation that the message was sent.',
  ].join(' ');

  console.log(`  Task: "${taskMessage.substring(0, 100)}..."`);

  let taskAResponse;
  try {
    taskAResponse = await submitTask(AGENT_A_PORT, taskMessage);
    console.log(`  Task submitted: ${JSON.stringify(taskAResponse)}`);
  } catch (e) {
    console.error(`FATAL: Failed to submit task to Agent A: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  const taskAId = taskAResponse.task_id || taskAResponse.id;
  if (!taskAId) {
    console.error('FATAL: No task_id returned from POST /task');
    console.error(`  Response: ${JSON.stringify(taskAResponse)}`);
    await stopAll();
    process.exit(1);
  }
  console.log(`  Task A ID: ${taskAId}\n`);

  const taskSubmitTime = Date.now();

  // -----------------------------------------------------------------------
  // Gate 6: Wait for Agent A to complete
  // -----------------------------------------------------------------------
  console.log('[Gate 6] Waiting for Agent A task to complete...');
  console.log(`  Polling /task/${taskAId} every 5s (timeout ${TASK_A_COMPLETE_TIMEOUT_MS / 1000}s)...`);

  const completedA = await pollTaskComplete(AGENT_A_PORT, taskAId, TASK_A_COMPLETE_TIMEOUT_MS, 'Agent A');

  if (!completedA) {
    console.error('\nFAIL: Agent A task did not complete within timeout.');
    console.error('  Agent A stderr tail:');
    console.error(readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 2000));
    await stopAll();
    process.exit(1);
  }

  const taskACompleteMs = Date.now() - taskSubmitTime;
  console.log(`  Agent A completed in ${formatMs(taskACompleteMs)}`);
  console.log(`  Status:     ${completedA.status}`);
  console.log(`  Iterations: ${completedA.iterations}`);
  console.log(`  Sats spent: ${formatSats(completedA.sats_spent)}`);
  console.log(`  Result:     ${(completedA.result || completedA.error || 'none').substring(0, 300)}`);
  console.log();

  // -----------------------------------------------------------------------
  // Gate 7: Wait for Agent B to receive and process a task
  // -----------------------------------------------------------------------
  console.log('[Gate 7] Waiting for Agent B to pick up message and spawn a task...');
  console.log(`  Polling /tasks every 5s (timeout ${TASK_B_APPEAR_TIMEOUT_MS / 1000}s)...`);

  const newTaskB = await pollForNewTask(AGENT_B_PORT, baselineB, TASK_B_APPEAR_TIMEOUT_MS, 'Agent B');

  let completedB = null;
  if (newTaskB) {
    const taskBId = newTaskB.id;
    console.log(`  Agent B task ID: ${taskBId}`);
    console.log(`  Description:     ${(newTaskB.task || '').substring(0, 200)}`);
    console.log(`  Status:          ${newTaskB.status}`);
    console.log(`  Origin:          ${newTaskB.origin || 'unknown'}`);

    const taskBStatus = (newTaskB.status || '').toLowerCase();
    if (taskBStatus === 'complete' || taskBStatus === 'error' || taskBStatus === 'cancelled') {
      completedB = newTaskB;
    } else {
      console.log(`\n  Waiting for Agent B task to complete...`);
      console.log(`  Polling /task/${taskBId} every 5s (timeout ${TASK_B_COMPLETE_TIMEOUT_MS / 1000}s)...`);

      completedB = await pollTaskComplete(AGENT_B_PORT, taskBId, TASK_B_COMPLETE_TIMEOUT_MS, 'Agent B');
    }

    if (completedB) {
      console.log(`  Agent B task completed.`);
      console.log(`  Status:     ${completedB.status}`);
      console.log(`  Iterations: ${completedB.iterations}`);
      console.log(`  Sats spent: ${formatSats(completedB.sats_spent)}`);
      console.log(`  Result:     ${(completedB.result || completedB.error || 'none').substring(0, 300)}`);
    } else {
      console.log('  Agent B task did not complete within timeout.');
    }
  } else {
    console.log('  No new task appeared on Agent B within timeout.');
    console.log('  This may mean Agent A used a different delivery method or the overlay/messagebox was unreachable.');
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 8: Inspect both transcripts
  // -----------------------------------------------------------------------
  console.log('[Gate 8] Inspecting transcripts...');
  console.log();

  console.log('--- Agent A Audit ---');
  let auditA = null;
  try {
    auditA = await getTaskAudit(AGENT_A_PORT, taskAId);
  } catch (e) {
    console.log(`  Could not fetch audit: ${e.message}`);
  }
  const infoA = inspectAudit(auditA, 'Agent A');
  console.log();

  let auditB = null;
  let infoB = {};
  if (completedB) {
    console.log('--- Agent B Audit ---');
    try {
      auditB = await getTaskAudit(AGENT_B_PORT, completedB.id || newTaskB?.id);
    } catch (e) {
      console.log(`  Could not fetch audit: ${e.message}`);
    }
    infoB = inspectAudit(auditB, 'Agent B');
    console.log();
  }

  // -----------------------------------------------------------------------
  // Gate 9: Print server logs (tail)
  // -----------------------------------------------------------------------
  console.log('[Gate 9] Server log tails...');
  console.log();

  console.log('--- Agent A stderr (last 1500 chars) ---');
  console.log(readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 1500));
  console.log();

  console.log('--- Agent B stderr (last 1500 chars) ---');
  console.log(readLogTail(path.join(WORKSPACE_B, 'server-stderr.log'), 1500));
  console.log();

  // -----------------------------------------------------------------------
  // Gate 10: Cleanup
  // -----------------------------------------------------------------------
  console.log('[Gate 10] Stopping servers...');
  await stopAll();
  console.log();

  // -----------------------------------------------------------------------
  // Final Report
  // -----------------------------------------------------------------------
  const totalMs = Date.now() - startTime;

  console.log('='.repeat(80));
  console.log('  FINAL REPORT');
  console.log('='.repeat(80));
  console.log();
  console.log(`  Test:                Two-Agent Handshake E2E (#314)`);
  console.log(`  Total time:          ${formatMs(totalMs)}`);
  console.log(`  Agent A identity:    ${agentAInfo.identity_key?.substring(0, 24)}...`);
  console.log(`  Agent B identity:    ${agentBInfo.identity_key?.substring(0, 24)}...`);
  console.log();

  // Agent A results
  const aStatus = (completedA.status || '').toLowerCase();
  const aComplete = aStatus === 'complete';
  const aHadLLM = (infoA.thinkEvents || []).length > 0;
  const aHadTools = (infoA.toolEvents || []).length > 0;
  const aToolNames = infoA.toolNames || [];
  const aUsedOverlay = aToolNames.includes('overlay_lookup');
  const aUsedSendMessage = aToolNames.includes('send_message');
  const aHadProofs = (completedA.proof_txids?.length || 0) > 0 || (infoA.proofEvents || []).length > 0;

  console.log('  Agent A:');
  console.log(`    Status:      ${completedA.status}`);
  console.log(`    Iterations:  ${completedA.iterations}`);
  console.log(`    Sats spent:  ${formatSats(completedA.sats_spent)}`);
  console.log(`    LLM calls:   ${(infoA.thinkEvents || []).length}`);
  console.log(`    Tool calls:  ${(infoA.toolEvents || []).length}`);
  console.log(`    Proofs:      ${(completedA.proof_txids?.length || 0)}`);
  console.log();

  // Agent B results
  const bAppeared = !!newTaskB;
  const bComplete = completedB && (completedB.status || '').toLowerCase() === 'complete';
  const bHadLLM = (infoB.thinkEvents || []).length > 0;

  if (completedB) {
    console.log('  Agent B:');
    console.log(`    Status:      ${completedB.status}`);
    console.log(`    Iterations:  ${completedB.iterations}`);
    console.log(`    Sats spent:  ${formatSats(completedB.sats_spent)}`);
    console.log(`    LLM calls:   ${(infoB.thinkEvents || []).length}`);
    console.log(`    Tool calls:  ${(infoB.toolEvents || []).length}`);
    console.log();
  } else {
    console.log('  Agent B: no task completed');
    console.log();
  }

  // Verification checks
  const checks = [
    { name: 'Both agents started with different identity keys', pass: agentAInfo.identity_key !== agentBInfo.identity_key },
    { name: 'Agent A task completed', pass: aComplete },
    { name: 'Agent A made LLM calls', pass: aHadLLM },
    { name: 'Agent A used tools', pass: aHadTools },
    { name: 'Agent A used overlay_lookup', pass: aUsedOverlay },
    { name: 'Agent A used send_message', pass: aUsedSendMessage },
    { name: 'Agent A created proofs', pass: aHadProofs },
    { name: 'Agent B received a task', pass: bAppeared },
    { name: 'Agent B task completed', pass: bComplete },
    { name: 'Agent B made LLM calls', pass: bHadLLM },
  ];

  console.log('  Verification:');
  let allPass = true;
  for (const check of checks) {
    const icon = check.pass ? 'PASS' : 'FAIL';
    console.log(`    [${icon}] ${check.name}`);
    if (!check.pass) allPass = false;
  }
  console.log();

  if (allPass) {
    console.log('  RESULT: ALL CHECKS PASSED');
  } else {
    console.log('  RESULT: SOME CHECKS FAILED (see details above)');
    console.log();
    console.log('  Note: Agent B receiving a task depends on Agent A successfully');
    console.log('  discovering Agent B via overlay and sending a MessageBox message.');
    console.log('  Check Agent A tool calls above for overlay_lookup and send_message results.');
  }
  console.log('='.repeat(80));

  process.exit(allPass ? 0 : 1);
}

// ---------------------------------------------------------------------------
// Entry point with cleanup
// ---------------------------------------------------------------------------

process.on('SIGINT', async () => {
  console.log('\nReceived SIGINT. Stopping servers...');
  await stopAll();
  process.exit(1);
});

process.on('SIGTERM', async () => {
  console.log('\nReceived SIGTERM. Stopping servers...');
  await stopAll();
  process.exit(1);
});

main().catch(async (err) => {
  console.error(`\nFATAL: ${err.message}`);
  if (err.stack) console.error(err.stack);
  await stopAll();
  process.exit(1);
});
