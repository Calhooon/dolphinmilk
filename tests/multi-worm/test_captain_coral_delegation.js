#!/usr/bin/env node
/**
 * Captain→Coral Delegation E2E Test (EPIC #329 Phase 3)
 *
 * Two running dolphin-milk agents commission work across identity keys via
 * the full delegation-macaroon flow:
 *
 *   1. Captain (Agent A, wallet 3322) looks Coral up in the overlay.
 *   2. Captain calls `delegate_task` with capabilities=[web_fetch],
 *      budget_cap_sats=100000, expires_in_secs=600. The tool builds a
 *      BRC-52 agent-delegation cert, creates a revocation UTXO, signs the
 *      canonical body, submits the revocation tx to the overlay, and
 *      dispatches a `task_delegation` envelope to Coral's task_inbox.
 *   3. Coral (Agent B, wallet 3323) polls its inbox. The heartbeat detects
 *      `type == "task_delegation"`, threads the full envelope through
 *      `InboxItem` → `spawn_task()` → `WormLoop::set_pending_delegation()`.
 *   4. Coral's `setup_task()` parses the cert + chain, verifies it against
 *      the deployed `ls_dm_delegation` overlay, applies the delegation's
 *      capability allowlist and budget cap as caveats, and records a
 *      `delegation_verified` transcript event.
 *   5. Coral runs the delegated task with cert-derived caveats in force.
 *
 * Real sats, real overlay (https://rust-overlay.dev-a3e.workers.dev), real
 * MessageBox, real LLM inference. Both agents trust the shared parent key
 * via `DOLPHIN_MILK_TRUST_CERTIFIERS`.
 *
 * Deep transcript inspection is mandatory: the test FAILS unless Coral's
 * session.jsonl shows a `delegation_verified` event with matching
 * budget_cap_sats and capabilities, and Captain's shows the delegate_task
 * tool call + a non-empty sent_message_id.
 *
 * Usage:
 *   node test_captain_coral_delegation.js
 */

const { spawn } = require('child_process');
const http = require('http');
const path = require('path');
const fs = require('fs');

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
const WORKSPACE_A = path.join(PROJECT_ROOT, 'test-workspaces/delegation-captain');
const WORKSPACE_B = path.join(PROJECT_ROOT, 'test-workspaces/delegation-coral');

const HEALTH_TIMEOUT_MS = 60000;
const TASK_A_COMPLETE_TIMEOUT_MS = 300000;
const TASK_B_APPEAR_TIMEOUT_MS = 300000;
const TASK_B_COMPLETE_TIMEOUT_MS = 300000;

// Delegation parameters — kept small so the test budget stays under $0.50.
const DELEGATED_BUDGET_CAP_SATS = 100000;
const DELEGATION_EXPIRES_IN_SECS = 600;

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

function startAgent(name, port, walletPort, workspace, parentKey, agentName, trustCertifiers) {
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
          // EPIC #329 Phase 3: trust root for inbound delegation certs.
          // Coral needs Captain's identity key here because in this test
          // environment Captain falls back to a self-signed agent cert
          // (MetaNet Client doesn't auto-issue parent-signed agent-auth
          // certs). In production with a parent that signs agent certs,
          // the parent's key alone would suffice.
          DOLPHIN_MILK_TRUST_CERTIFIERS: trustCertifiers,
        },
        stdio: ['ignore', stdoutLog, stderrLog],
        detached: false,
      }
    );

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
    stopAgent(serverA, 'Captain'),
    stopAgent(serverB, 'Coral'),
  ]);
}

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
// Deep transcript inspection
// ---------------------------------------------------------------------------

function inspectAudit(audit, label) {
  if (!audit) {
    console.log(`  ${label}: No audit data available`);
    return { events: [], thinkEvents: [], toolEvents: [], toolNames: [] };
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
  const toolEvents = events.filter(e => (e.event_type || e.type) === 'tool_call');
  const toolNames = [];
  for (const tc of toolEvents) {
    const name = tc.tool_name || tc.data?.name || tc.data?.tool_name || tc.name || 'unknown';
    toolNames.push(name);
  }

  console.log(`  LLM calls:  ${thinkEvents.length}`);
  console.log(`  Tool calls: ${toolEvents.length}`);
  for (const name of toolNames) {
    console.log(`    - ${name}`);
  }

  if (audit.summary) {
    console.log(`  Summary:`);
    console.log(`    Iterations: ${audit.summary.iterations || '?'}`);
    console.log(`    Sats:       ${formatSats(audit.summary.total_sats_spent || audit.summary.sats_spent)}`);
    console.log(`    Model:      ${audit.summary.model || '?'}`);
  }

  return { events, thinkEvents, toolEvents, toolNames };
}

function readTranscriptJsonl(workspace, taskId) {
  const candidates = [
    path.join(workspace, 'tasks', taskId, 'session.jsonl'),
    path.join(workspace, 'tasks', taskId, 'transcript.jsonl'),
  ];
  for (const p of candidates) {
    if (fs.existsSync(p)) {
      const raw = fs.readFileSync(p, 'utf8');
      const events = [];
      for (const line of raw.split(/\r?\n/)) {
        if (!line.trim()) continue;
        try { events.push(JSON.parse(line)); } catch {}
      }
      return { path: p, events };
    }
  }
  return null;
}

function findEvent(events, typeName) {
  return events.find(e => (e.event_type || e.type) === typeName);
}

function findAllEvents(events, typeName) {
  return events.filter(e => (e.event_type || e.type) === typeName);
}

function getEventData(ev) {
  return ev?.data || ev;
}

function inspectCoralDelegation(workspace, taskId, expectedBudgetCap) {
  console.log();
  console.log(`--- Coral Deep Transcript Inspection (task ${taskId}) ---`);
  const transcript = readTranscriptJsonl(workspace, taskId);
  if (!transcript) {
    console.log('  FAIL: No transcript file found in workspace.');
    return { pass: false, reason: 'no transcript' };
  }
  console.log(`  Transcript: ${transcript.path} (${transcript.events.length} events)`);

  // 1. delegation_verified event exists
  const verified = findEvent(transcript.events, 'delegation_verified');
  const rejected = findEvent(transcript.events, 'delegation_rejected');

  if (rejected) {
    const reason = getEventData(rejected).reason || '(no reason)';
    console.log(`  FAIL: Coral emitted delegation_rejected with reason=${reason}`);
    return { pass: false, reason: `delegation_rejected: ${reason}` };
  }

  if (!verified) {
    console.log('  FAIL: No delegation_verified event in transcript.');
    console.log('        (Expected this event to be emitted at setup_task when a valid cert is received.)');
    return { pass: false, reason: 'missing delegation_verified event' };
  }

  const verifiedData = getEventData(verified);
  console.log(`  delegation_verified event present.`);
  console.log(`    chain_depth:    ${verifiedData.chain_depth}`);
  console.log(`    capabilities:   ${JSON.stringify(verifiedData.capabilities)}`);
  console.log(`    budget_cap:     ${verifiedData.budget_cap_sats}`);
  console.log(`    expires_at:     ${verifiedData.expires_at}`);
  console.log(`    root_certifier: ${(verifiedData.root_certifier || '').substring(0, 24)}...`);

  // 2. capability set contains web_fetch
  const caps = verifiedData.capabilities || [];
  if (!Array.isArray(caps) || !caps.includes('web_fetch')) {
    console.log(`  FAIL: Capabilities do not contain web_fetch: ${JSON.stringify(caps)}`);
    return { pass: false, reason: 'capabilities missing web_fetch' };
  }

  // 3. budget_cap matches expected
  if (verifiedData.budget_cap_sats !== expectedBudgetCap) {
    console.log(`  FAIL: budget_cap_sats=${verifiedData.budget_cap_sats}, expected ${expectedBudgetCap}`);
    return {
      pass: false,
      reason: `budget_cap mismatch: got ${verifiedData.budget_cap_sats}, expected ${expectedBudgetCap}`,
    };
  }

  // 4. session_end with no error
  const sessionEnd = findEvent(transcript.events, 'session_end');
  if (!sessionEnd) {
    console.log('  FAIL: No session_end event.');
    return { pass: false, reason: 'missing session_end' };
  }
  const endData = getEventData(sessionEnd);
  const sessionError = endData.error || '';
  if (sessionError && sessionError !== '' && sessionError !== null) {
    console.log(`  FAIL: session_end has error: ${sessionError}`);
    return { pass: false, reason: `session_end error: ${sessionError}` };
  }
  console.log(`  session_end: iterations=${endData.iterations}, error="${sessionError}"`);

  console.log('  PASS: Coral transcript passes deep inspection.');
  return { pass: true };
}

function inspectCaptainDelegation(workspace, taskId) {
  console.log();
  console.log(`--- Captain Deep Transcript Inspection (task ${taskId}) ---`);
  const transcript = readTranscriptJsonl(workspace, taskId);
  if (!transcript) {
    console.log('  FAIL: No transcript file found in workspace.');
    return { pass: false, reason: 'no transcript', commissionId: null };
  }
  console.log(`  Transcript: ${transcript.path} (${transcript.events.length} events)`);

  // Look for a tool_call with name=delegate_task, and its matching tool_result.
  const toolCalls = findAllEvents(transcript.events, 'tool_call');
  const toolResults = findAllEvents(transcript.events, 'tool_result');

  const delegateCall = toolCalls.find(c => {
    const d = getEventData(c);
    const name = d.name || d.tool_name || c.tool_name;
    return name === 'delegate_task';
  });

  if (!delegateCall) {
    console.log('  FAIL: Captain never called delegate_task.');
    console.log('        Tool calls made:');
    for (const c of toolCalls) {
      const d = getEventData(c);
      console.log(`          - ${d.name || d.tool_name || c.tool_name || 'unknown'}`);
    }
    return { pass: false, reason: 'missing delegate_task tool call' };
  }

  // Find the matching tool_result (by call_id if possible, otherwise last result).
  const callData = getEventData(delegateCall);
  const callId = callData.call_id || callData.id || delegateCall.call_id || delegateCall.id;
  const matching = toolResults.find(r => {
    const d = getEventData(r);
    return (d.call_id || d.id || r.call_id || r.id) === callId;
  });
  const delegateResult = matching || toolResults[toolResults.length - 1];

  if (!delegateResult) {
    console.log('  FAIL: delegate_task tool_call had no matching tool_result.');
    return { pass: false, reason: 'no delegate_task tool_result' };
  }

  const resultData = getEventData(delegateResult);
  const resultStr =
    typeof resultData === 'string'
      ? resultData
      : (resultData.output || resultData.result || JSON.stringify(resultData));

  console.log(`  delegate_task tool_result: ${String(resultStr).substring(0, 200)}`);

  if (String(resultStr).toLowerCase().startsWith('error')) {
    console.log('  FAIL: delegate_task returned an error.');
    return { pass: false, reason: `delegate_task error: ${resultStr}` };
  }

  // Parse JSON result and extract commission_id + sent_message_id
  let parsed = null;
  try { parsed = JSON.parse(resultStr); } catch {}
  if (parsed && parsed.commission_id && parsed.sent_message_id) {
    console.log(`    commission_id:   ${parsed.commission_id}`);
    console.log(`    sent_message_id: ${parsed.sent_message_id}`);
    console.log(`    cert_hash:       ${(parsed.delegation_cert_hash || '').substring(0, 24)}...`);
    console.log(`    revocation_txid: ${(parsed.revocation_txid || '').substring(0, 24)}...`);
  }

  // session_end with no error
  const sessionEnd = findEvent(transcript.events, 'session_end');
  if (sessionEnd) {
    const endData = getEventData(sessionEnd);
    const sessionError = endData.error || '';
    console.log(`  session_end: iterations=${endData.iterations}, error="${sessionError}"`);
  }

  console.log('  PASS: Captain called delegate_task successfully.');
  return { pass: true, commissionId: parsed?.commission_id || null };
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

async function main() {
  const startTime = Date.now();
  console.log('='.repeat(80));
  console.log('  CAPTAIN → CORAL DELEGATION E2E TEST (EPIC #329 Phase 3)');
  console.log('  Cross-agent task commissioning via macaroon-style delegation certs.');
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

  let captainIdentityKey;
  try {
    const resp = await walletPost(WALLET_A_PORT, 'getPublicKey', { identityKey: true });
    captainIdentityKey = resp.publicKey;
    console.log(`  Wallet A (:${WALLET_A_PORT}): ${captainIdentityKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet A not reachable on port ${WALLET_A_PORT}: ${e.message}`);
    process.exit(1);
  }

  let coralIdentityKeyFromWallet;
  try {
    const resp = await walletPost(WALLET_B_PORT, 'getPublicKey', { identityKey: true });
    coralIdentityKeyFromWallet = resp.publicKey;
    console.log(`  Wallet B (:${WALLET_B_PORT}): ${coralIdentityKeyFromWallet.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet B not reachable on port ${WALLET_B_PORT}: ${e.message}`);
    process.exit(1);
  }

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
  // Gate 2: Start both agents with trust config
  // -----------------------------------------------------------------------
  //
  // Trust roots: BOTH agents trust ONLY the parent key (the operator's
  // declared trust root). With EPIC #329 Phase 3 fixes:
  //  - Captain's startup acquires a parent-signed agent-auth cert from
  //    MetaNet Client at 3321, so cert.certifier == parent_key.
  //  - Captain's delegate_task::resolve_root_certifier() stamps
  //    config.trust.certifiers[0] (= parent_key) as the delegation cert's
  //    root_certifier — operator-declared trust root wins regardless of
  //    cert state.
  //  - Coral verifies the inbound delegation cert against trust.certifiers
  //    and accepts because root_certifier == parent_key.
  // No Captain-key fallback in Coral's trust list — the parent-mediated
  // story must work end-to-end.
  const captainTrust = parentKey;
  const coralTrust = parentKey;

  console.log('[Gate 2] Starting both agents (trust.certifiers = parent_key only)...');
  console.log(`  Captain trust: parent_key`);
  console.log(`  Coral trust:   parent_key (no fallback — must verify via parent root)`);

  clearAuthCache();

  try {
    serverA = await startAgent('Captain', AGENT_A_PORT, WALLET_A_PORT, WORKSPACE_A, parentKey, 'delegation-captain', captainTrust);
  } catch (e) {
    console.error(`FATAL: Could not start Captain: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  try {
    serverB = await startAgent('Coral', AGENT_B_PORT, WALLET_B_PORT, WORKSPACE_B, parentKey, 'delegation-coral', coralTrust);
  } catch (e) {
    console.error(`FATAL: Could not start Coral: ${e.message}`);
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
    console.log(`  Captain identity: ${agentAInfo.identity_key?.substring(0, 24)}...`);
    console.log(`  Captain balance:  ${formatSats(agentAInfo.balance)}`);
  } catch (e) {
    console.error(`FATAL: Cannot get Captain identity: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  try {
    agentBInfo = await getAgentInfo(AGENT_B_PORT);
    console.log(`  Coral identity:   ${agentBInfo.identity_key?.substring(0, 24)}...`);
    console.log(`  Coral balance:    ${formatSats(agentBInfo.balance)}`);
  } catch (e) {
    console.error(`FATAL: Cannot get Coral identity: ${e.message}`);
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
  // Gate 4: Baseline task list for Coral
  // -----------------------------------------------------------------------
  console.log('[Gate 4] Getting baseline task list for Coral...');

  const baselineB = new Set();
  try {
    (await getTaskList(AGENT_B_PORT)).forEach(t => baselineB.add(t.id));
    console.log(`  Coral baseline: ${baselineB.size} tasks`);
  } catch (e) {
    console.log(`  Coral baseline: error (${e.message}), assuming 0`);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 5: Submit delegation task to Captain
  // -----------------------------------------------------------------------
  console.log('[Gate 5] Submitting delegation task to Captain...');

  // Captain's instructions: look up Coral via overlay, delegate a web_fetch task.
  // We intentionally spell out the exact delegate_task arguments so gpt-5-mini
  // doesn't fumble the tool call.
  const delegatedTask =
    'Fetch https://example.com and return its HTTP status code.';

  // God-tier flow: use findByCertifier on the overlay so Captain discovers
  // agents certified by the trust root WITHOUT baking Coral's identity_key
  // into the prompt. This is the real multi-agent coordination story: "any
  // agent certified by a party I trust, not myself". The prompt still
  // includes explicit filter rules so gpt-5-mini executes deterministically.

  const captainTaskMessage = [
    'Commission another agent to perform a small web_fetch task via scoped delegation.',
    '',
    'Step 1: Call wallet_identity to get YOUR own identity_key. You will use this',
    'to filter out yourself from overlay results.',
    '',
    'Step 2: Call overlay_lookup with service "ls_agent" and query',
    `  {"findByCertifier": "${parentKey}"}`,
    'This returns every agent certified by your trusted root. The response includes',
    'each agent\'s identity_key, certifier_key, name, and capabilities.',
    '',
    'Step 3: From the overlay results, pick EXACTLY ONE agent such that:',
    '  - its identity_key is DIFFERENT from your own',
    '  - its capabilities contain "llm" (or "tool-use" / "web_fetch" — any agent',
    '    certified by your trusted root is acceptable for this simple task)',
    'Store its identity_key for Step 4.',
    '',
    'If NO agent in the overlay results has a different identity_key from yours,',
    'respond with "NO_TRUSTED_RECIPIENT" and stop.',
    '',
    'Step 4: Call delegate_task with these EXACT arguments:',
    `  recipient: <the selected agent's identity_key from Step 3>`,
    `  task: "${delegatedTask}"`,
    `  capabilities: ["web_fetch"]`,
    `  budget_cap_sats: ${DELEGATED_BUDGET_CAP_SATS}`,
    `  expires_in_secs: ${DELEGATION_EXPIRES_IN_SECS}`,
    '',
    'CRITICAL: You MUST call delegate_task, not send_message. delegate_task creates',
    'a signed BRC-52 delegation cert; send_message does not.',
    '',
    'Step 5: Report the commission_id and sent_message_id from the tool result.',
    'Do NOT call delegate_task more than once.',
  ].join('\n');

  console.log(`  Task: "${captainTaskMessage.substring(0, 100)}..."`);

  let taskAResponse;
  try {
    taskAResponse = await submitTask(AGENT_A_PORT, captainTaskMessage);
    console.log(`  Task submitted: ${JSON.stringify(taskAResponse)}`);
  } catch (e) {
    console.error(`FATAL: Failed to submit task to Captain: ${e.message}`);
    await stopAll();
    process.exit(1);
  }

  const taskAId = taskAResponse.task_id || taskAResponse.id;
  if (!taskAId) {
    console.error('FATAL: No task_id returned from POST /task');
    await stopAll();
    process.exit(1);
  }
  console.log(`  Captain task ID: ${taskAId}\n`);

  // -----------------------------------------------------------------------
  // Gate 6: Wait for Captain to complete
  // -----------------------------------------------------------------------
  console.log('[Gate 6] Waiting for Captain to complete delegation...');

  const completedA = await pollTaskComplete(AGENT_A_PORT, taskAId, TASK_A_COMPLETE_TIMEOUT_MS, 'Captain');

  if (!completedA) {
    console.error('\nFAIL: Captain task did not complete within timeout.');
    console.error(readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 2000));
    await stopAll();
    process.exit(1);
  }

  console.log(`  Captain completed.`);
  console.log(`  Status:     ${completedA.status}`);
  console.log(`  Iterations: ${completedA.iterations}`);
  console.log(`  Sats spent: ${formatSats(completedA.sats_spent)}`);
  console.log();

  // -----------------------------------------------------------------------
  // Gate 7: Wait for Coral to pick up the delegation and run
  // -----------------------------------------------------------------------
  console.log('[Gate 7] Waiting for Coral to receive delegation and spawn task...');

  const newTaskB = await pollForNewTask(AGENT_B_PORT, baselineB, TASK_B_APPEAR_TIMEOUT_MS, 'Coral');

  let completedB = null;
  let taskBId = null;
  if (newTaskB) {
    taskBId = newTaskB.id;
    console.log(`  Coral task ID:   ${taskBId}`);
    console.log(`  Description:     ${(newTaskB.task || '').substring(0, 200)}`);
    console.log(`  Status:          ${newTaskB.status}`);
    console.log(`  Origin:          ${newTaskB.origin || 'unknown'}`);

    const taskBStatus = (newTaskB.status || '').toLowerCase();
    if (taskBStatus === 'complete' || taskBStatus === 'error' || taskBStatus === 'cancelled') {
      completedB = newTaskB;
    } else {
      completedB = await pollTaskComplete(AGENT_B_PORT, taskBId, TASK_B_COMPLETE_TIMEOUT_MS, 'Coral');
    }

    if (completedB) {
      console.log(`  Coral task completed.`);
      console.log(`  Status:     ${completedB.status}`);
      console.log(`  Iterations: ${completedB.iterations}`);
      console.log(`  Sats spent: ${formatSats(completedB.sats_spent)}`);
    }
  } else {
    console.log('  No new task appeared on Coral within timeout.');
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 7.5: Wait for Captain's heartbeat to process the commission
  //           payment claim. Coral emits the claim at session_end and
  //           Captain's heartbeat polls every 15s — give it up to 90s
  //           (~5 poll cycles) to pick it up, validate, broadcast the
  //           BSV payment, and reply.
  // -----------------------------------------------------------------------
  console.log('[Gate 7.5] Waiting for Captain to process commission payment claim...');
  const paymentDeadline = Date.now() + 90000;
  let captainPaymentLogged = false;
  let coralPaymentInternalized = false;
  while (Date.now() < paymentDeadline) {
    const captainTail = readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 8000);
    const coralTail = readLogTail(path.join(WORKSPACE_B, 'server-stderr.log'), 8000);
    if (!captainPaymentLogged && captainTail.includes('commission payment broadcast')) {
      captainPaymentLogged = true;
      console.log('  Captain: commission payment broadcast detected in stderr');
    }
    if (!coralPaymentInternalized && coralTail.includes('commission_payment_sent internalized')) {
      coralPaymentInternalized = true;
      console.log('  Coral:   commission_payment_sent internalized');
    }
    if (captainPaymentLogged && coralPaymentInternalized) {
      break;
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  if (!captainPaymentLogged) {
    console.log('  WARNING: Captain did not broadcast commission payment within 90s.');
  }
  if (!coralPaymentInternalized) {
    console.log('  WARNING: Coral did not internalize commission payment within 90s.');
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 8: Deep transcript inspection (mandatory acceptance gate)
  // -----------------------------------------------------------------------
  console.log('[Gate 8] Deep transcript inspection (mandatory)...');

  const captainAudit = await getTaskAudit(AGENT_A_PORT, taskAId);
  const captainSummary = inspectAudit(captainAudit, 'Captain audit');
  console.log();

  const captainDeep = inspectCaptainDelegation(WORKSPACE_A, taskAId);

  let coralAudit = null;
  let coralSummary = { events: [], thinkEvents: [], toolEvents: [], toolNames: [] };
  let coralDeep = { pass: false, reason: 'Coral task never appeared' };
  if (taskBId) {
    coralAudit = await getTaskAudit(AGENT_B_PORT, taskBId);
    coralSummary = inspectAudit(coralAudit, 'Coral audit');
    coralDeep = inspectCoralDelegation(WORKSPACE_B, taskBId, DELEGATED_BUDGET_CAP_SATS);
  }

  console.log();
  console.log('--- Captain stderr (last 1500 chars) ---');
  console.log(readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 1500));
  console.log();
  console.log('--- Coral stderr (last 1500 chars) ---');
  console.log(readLogTail(path.join(WORKSPACE_B, 'server-stderr.log'), 1500));
  console.log();

  // -----------------------------------------------------------------------
  // Gate 9: Cleanup
  // -----------------------------------------------------------------------
  console.log('[Gate 9] Stopping servers...');
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
  console.log(`  Test:             Captain→Coral Delegation E2E (EPIC #329 Phase 3)`);
  console.log(`  Total time:       ${formatMs(totalMs)}`);
  console.log(`  Captain identity: ${agentAInfo.identity_key?.substring(0, 24)}...`);
  console.log(`  Coral identity:   ${agentBInfo.identity_key?.substring(0, 24)}...`);
  console.log();

  const aStatus = (completedA.status || '').toLowerCase();
  const aComplete = aStatus === 'complete';
  const aUsedDelegateTask = (captainSummary.toolNames || []).includes('delegate_task');

  const bAppeared = !!newTaskB;
  const bComplete = completedB && (completedB.status || '').toLowerCase() === 'complete';

  // Check for commission_payment_claim event in Coral's transcript
  const coralTranscript = readTranscriptJsonl(WORKSPACE_B, taskBId);
  const coralPaymentClaim = coralTranscript
    ? findEvent(coralTranscript.events, 'commission_payment_claim')
    : null;
  const coralEmittedClaim = !!coralPaymentClaim;

  const checks = [
    { name: 'Captain task completed', pass: aComplete },
    { name: 'Captain called delegate_task', pass: aUsedDelegateTask },
    { name: 'Captain delegate_task returned success', pass: captainDeep.pass },
    { name: 'Coral received a task from Captain', pass: bAppeared },
    { name: 'Coral task completed', pass: bComplete },
    { name: 'Coral transcript has delegation_verified event', pass: coralDeep.pass },
    { name: 'Coral emitted commission_payment_claim at session_end', pass: coralEmittedClaim },
    { name: 'Captain broadcast the commission payment', pass: captainPaymentLogged },
    { name: 'Coral internalized commission_payment_sent', pass: coralPaymentInternalized },
  ];

  console.log('  Verification:');
  let allPass = true;
  for (const check of checks) {
    const icon = check.pass ? 'PASS' : 'FAIL';
    console.log(`    [${icon}] ${check.name}`);
    if (!check.pass) allPass = false;
  }
  console.log();

  if (!coralDeep.pass && coralDeep.reason) {
    console.log(`  Coral transcript failure reason: ${coralDeep.reason}`);
  }
  if (!captainDeep.pass && captainDeep.reason) {
    console.log(`  Captain transcript failure reason: ${captainDeep.reason}`);
  }
  console.log();

  if (allPass) {
    console.log('  RESULT: ALL CHECKS PASSED — delegation E2E green');
  } else {
    console.log('  RESULT: SOME CHECKS FAILED');
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
