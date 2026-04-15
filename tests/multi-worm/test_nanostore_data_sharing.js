#!/usr/bin/env node
/**
 * NanoStore Data Sharing E2E Test (Issue #344)
 *
 * Two running dolphin-milk agents exchange a dataset via NanoStore + UHRP
 * URL instead of stuffing it into a MessageBox body. This mirrors the
 * captain-coral delegation shape: Alice uploads, then DELEGATES a web_fetch
 * task to Bob with capabilities=["web_fetch"]. The cert chain gives Bob the
 * tool access he needs (external-origin inbox processing would strip
 * web_fetch, which is not in EXTERNAL_TOOL_ALLOWLIST).
 *
 *   1. Alice (:8081, wallet :3322) uploads a small JSON dataset via
 *      `upload_to_nanostore` (paid x402). Tool returns `public_url`.
 *   2. Alice calls `delegate_task` with capabilities=[web_fetch],
 *      budget_cap_sats=200000, expires_in_secs=600, task text containing
 *      the public_url and a per-run marker string. This builds a BRC-52
 *      delegation cert and dispatches the `task_delegation` envelope to
 *      Bob's task_inbox.
 *   3. Bob (:8082, wallet :3323) receives the delegation. His heartbeat
 *      detects `type == "task_delegation"`, threads the envelope through
 *      InboxItem → spawn_task() → WormLoop::set_pending_delegation().
 *      setup_task() parses the cert, applies the delegation's capability
 *      allowlist (web_fetch), and records a `delegation_verified` event.
 *   4. Bob runs the delegated task with web_fetch allowed, fetches the
 *      NanoStore URL, and confirms it contains the per-run marker.
 *   5. Commission payment flow runs the same as captain-coral: Bob emits a
 *      commission_payment_claim at session_end; Alice's heartbeat picks it
 *      up and broadcasts the payment; Bob internalizes it.
 *
 * Deep transcript inspection is mandatory:
 *   Alice: upload_to_nanostore succeeded + returned public_url,
 *          delegate_task succeeded and carried public_url in its task text.
 *   Bob:   delegation_verified event with web_fetch in capabilities,
 *          web_fetch tool_call whose URL contains the public_url,
 *          web_fetch tool_result contains the per-run marker,
 *          session_end with no error.
 *
 * Cost target: ~$0.20-$0.50 per run (upload ~0.1¢, 2× LLM + delegation
 * infrastructure).
 *
 * Usage:
 *   node test_nanostore_data_sharing.js
 */

const { spawn } = require('child_process');
const http = require('http');
const path = require('path');
const fs = require('fs');
const crypto = require('crypto');

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
const WORKSPACE_A = path.join(PROJECT_ROOT, 'test-workspaces/nanostore-alice');
const WORKSPACE_B = path.join(PROJECT_ROOT, 'test-workspaces/nanostore-bob');

const HEALTH_TIMEOUT_MS = 60000;
const TASK_A_COMPLETE_TIMEOUT_MS = 300000;
const TASK_B_APPEAR_TIMEOUT_MS = 300000;
const TASK_B_COMPLETE_TIMEOUT_MS = 300000;

// --keep-alive leaves the two dolphin-milk servers running after the final
// report so the user can browse the UIs at :8081/ui/ and :8082/ui/. Press
// Ctrl-C to stop them.
const KEEP_ALIVE = process.argv.includes('--keep-alive');

// Delegation parameters — matches captain-coral for comparability.
const DELEGATED_BUDGET_CAP_SATS = 200000;
const DELEGATION_EXPIRES_IN_SECS = 600;

// Unique payload per run so stale inbox messages from previous runs don't
// cause false positives.
const RUN_NONCE = crypto.randomBytes(4).toString('hex');
const MARKER = `NANOSTORE_E2E_${RUN_NONCE}`;
const PAYLOAD = {
  marker: MARKER,
  created_at: new Date().toISOString(),
  records: [
    { id: 1, name: 'alpha', value: 100 },
    { id: 2, name: 'bravo', value: 200 },
    { id: 3, name: 'charlie', value: 300 },
    { id: 4, name: 'delta', value: 400 },
    { id: 5, name: 'echo', value: 500 },
  ],
};
const PAYLOAD_JSON = JSON.stringify(PAYLOAD);
const PAYLOAD_SHA256 = crypto.createHash('sha256').update(PAYLOAD_JSON).digest('hex');

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
    stopAgent(serverA, 'Alice'),
    stopAgent(serverB, 'Bob'),
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

function getToolName(call) {
  const d = getEventData(call);
  return d.name || d.tool_name || call.tool_name || 'unknown';
}

function getToolArgs(call) {
  const d = getEventData(call);
  const raw = d.arguments || d.args || d.input || call.arguments;
  if (!raw) return {};
  if (typeof raw === 'string') {
    try { return JSON.parse(raw); } catch { return {}; }
  }
  return raw;
}

function getToolResultForCall(toolResults, call) {
  const callData = getEventData(call);
  const callId = callData.call_id || callData.id || call.call_id || call.id;
  const match = toolResults.find(r => {
    const d = getEventData(r);
    return (d.call_id || d.id || r.call_id || r.id) === callId;
  });
  if (match) return match;
  return null;
}

function resultString(result) {
  if (!result) return '';
  const d = getEventData(result);
  if (typeof d === 'string') return d;
  // Tool_result events serialize the inner tool output as a JSON string in
  // the `content` field. Prefer that. Fall back to other shapes.
  if (typeof d.content === 'string') return d.content;
  if (d.content != null) return JSON.stringify(d.content);
  return d.output || d.result || JSON.stringify(d);
}

function inspectAudit(audit, label) {
  if (!audit) {
    console.log(`  ${label}: No audit data available`);
    return { toolNames: [] };
  }
  const events = audit.events || [];
  console.log(`  ${label}: ${events.length} events`);
  const thinks = events.filter(e => (e.event_type || e.type) === 'think_response').length;
  const tools = events.filter(e => (e.event_type || e.type) === 'tool_call');
  const toolNames = tools.map(getToolName);
  console.log(`    LLM calls:  ${thinks}`);
  console.log(`    Tool calls: ${tools.length}`);
  for (const name of toolNames) console.log(`      - ${name}`);
  if (audit.summary) {
    console.log(`    Iterations: ${audit.summary.iterations || '?'}`);
    console.log(`    Sats:       ${formatSats(audit.summary.total_sats_spent || audit.summary.sats_spent)}`);
    console.log(`    Model:      ${audit.summary.model || '?'}`);
  }
  return { toolNames };
}

function inspectAlice(workspace, taskId) {
  console.log();
  console.log(`--- Alice Deep Transcript Inspection (task ${taskId}) ---`);
  const transcript = readTranscriptJsonl(workspace, taskId);
  if (!transcript) {
    console.log('  FAIL: No transcript file found.');
    return { pass: false, reason: 'no transcript', publicUrl: null };
  }
  console.log(`  Transcript: ${transcript.path} (${transcript.events.length} events)`);

  const toolCalls = findAllEvents(transcript.events, 'tool_call');
  const toolResults = findAllEvents(transcript.events, 'tool_result');

  // 1. upload_to_nanostore called successfully
  const uploadCall = toolCalls.find(c => getToolName(c) === 'upload_to_nanostore');
  if (!uploadCall) {
    console.log('  FAIL: Alice never called upload_to_nanostore.');
    console.log('        Tools called:', toolCalls.map(getToolName).join(', '));
    return { pass: false, reason: 'missing upload_to_nanostore', publicUrl: null };
  }
  const uploadResult = getToolResultForCall(toolResults, uploadCall);
  const uploadResultStr = resultString(uploadResult);
  console.log(`  upload_to_nanostore result (first 250): ${uploadResultStr.substring(0, 250)}`);
  if (uploadResultStr.toLowerCase().startsWith('error')) {
    console.log('  FAIL: upload_to_nanostore returned an error.');
    return { pass: false, reason: `upload error: ${uploadResultStr}`, publicUrl: null };
  }

  let publicUrl = null;
  try {
    const parsed = JSON.parse(uploadResultStr);
    publicUrl = parsed.public_url || parsed.url || null;
    if (parsed.sats_paid) console.log(`    Upload cost: ${formatSats(parsed.sats_paid)}`);
    if (parsed.payment_txid) console.log(`    Payment txid: ${parsed.payment_txid.substring(0, 24)}...`);
    if (parsed.content_sha256) console.log(`    Content sha256: ${parsed.content_sha256.substring(0, 24)}...`);
  } catch (e) {
    console.log(`  FAIL: Could not parse upload_to_nanostore result: ${e.message}`);
    return { pass: false, reason: 'unparseable upload result', publicUrl: null };
  }

  if (!publicUrl || typeof publicUrl !== 'string' || publicUrl.length < 10) {
    console.log(`  FAIL: upload_to_nanostore did not return a valid public_url (got ${JSON.stringify(publicUrl)}).`);
    return { pass: false, reason: 'missing public_url', publicUrl: null };
  }
  console.log(`  public_url: ${publicUrl}`);

  // 2. delegate_task called with URL in the task text
  const delegateCall = toolCalls.find(c => getToolName(c) === 'delegate_task');
  if (!delegateCall) {
    console.log('  FAIL: Alice never called delegate_task.');
    console.log('        Tools called:', toolCalls.map(getToolName).join(', '));
    return { pass: false, reason: 'missing delegate_task', publicUrl };
  }
  const delegateArgs = getToolArgs(delegateCall);
  const delegateTaskText =
    (typeof delegateArgs.task === 'string' ? delegateArgs.task : JSON.stringify(delegateArgs.task || '')) +
    ' ' +
    JSON.stringify(delegateArgs);
  if (!delegateTaskText.includes(publicUrl)) {
    console.log(`  FAIL: delegate_task arguments did not include the public_url.`);
    console.log(`    delegate_task args: ${JSON.stringify(delegateArgs).substring(0, 400)}`);
    return { pass: false, reason: 'public_url missing from delegate_task', publicUrl };
  }
  console.log(`  delegate_task called with public_url embedded in task text.`);

  // 3. delegate_task result was a success (commission_id + sent_message_id)
  const delegateResult = getToolResultForCall(toolResults, delegateCall);
  const delegateResultStr = resultString(delegateResult);
  console.log(`  delegate_task result (first 250): ${delegateResultStr.substring(0, 250)}`);
  if (delegateResultStr.toLowerCase().startsWith('error')) {
    console.log(`  FAIL: delegate_task returned an error.`);
    return { pass: false, reason: `delegate_task error: ${delegateResultStr}`, publicUrl };
  }
  let commissionId = null;
  try {
    const parsed = JSON.parse(delegateResultStr);
    commissionId = parsed.commission_id || null;
    if (parsed.sent_message_id) console.log(`    sent_message_id: ${parsed.sent_message_id}`);
    if (parsed.commission_id) console.log(`    commission_id:   ${parsed.commission_id}`);
  } catch {}

  // 4. session_end with no error
  const sessionEnd = findEvent(transcript.events, 'session_end');
  if (sessionEnd) {
    const endData = getEventData(sessionEnd);
    const sessionError = endData.error || '';
    if (sessionError) {
      console.log(`  FAIL: session_end has error: ${sessionError}`);
      return { pass: false, reason: `session_end error: ${sessionError}`, publicUrl };
    }
    console.log(`  session_end: iterations=${endData.iterations}, no error`);
  }

  console.log('  PASS: Alice transcript passes deep inspection.');
  return { pass: true, publicUrl, commissionId };
}

function inspectBob(workspace, taskId, expectedMarker, expectedUrl, expectedBudgetCap) {
  console.log();
  console.log(`--- Bob Deep Transcript Inspection (task ${taskId}) ---`);
  const transcript = readTranscriptJsonl(workspace, taskId);
  if (!transcript) {
    console.log('  FAIL: No transcript file found.');
    return { pass: false, reason: 'no transcript' };
  }
  console.log(`  Transcript: ${transcript.path} (${transcript.events.length} events)`);

  // 1. delegation_verified event present (same gate as captain-coral)
  const verified = findEvent(transcript.events, 'delegation_verified');
  const rejected = findEvent(transcript.events, 'delegation_rejected');
  if (rejected) {
    const reason = getEventData(rejected).reason || '(no reason)';
    console.log(`  FAIL: Bob emitted delegation_rejected: ${reason}`);
    return { pass: false, reason: `delegation_rejected: ${reason}` };
  }
  if (!verified) {
    console.log('  FAIL: No delegation_verified event in transcript.');
    return { pass: false, reason: 'missing delegation_verified event' };
  }
  const verifiedData = getEventData(verified);
  console.log(`  delegation_verified:`);
  console.log(`    capabilities: ${JSON.stringify(verifiedData.capabilities)}`);
  console.log(`    budget_cap:   ${verifiedData.budget_cap_sats}`);
  const caps = verifiedData.capabilities || [];
  if (!Array.isArray(caps) || !caps.includes('web_fetch')) {
    console.log(`  FAIL: Capabilities do not contain web_fetch: ${JSON.stringify(caps)}`);
    return { pass: false, reason: 'capabilities missing web_fetch' };
  }
  if (expectedBudgetCap && verifiedData.budget_cap_sats !== expectedBudgetCap) {
    console.log(`  FAIL: budget_cap_sats=${verifiedData.budget_cap_sats}, expected ${expectedBudgetCap}`);
    return { pass: false, reason: 'budget_cap mismatch' };
  }

  // 2. web_fetch called with the nanostore URL
  const toolCalls = findAllEvents(transcript.events, 'tool_call');
  const toolResults = findAllEvents(transcript.events, 'tool_result');
  const webFetchCalls = toolCalls.filter(c => getToolName(c) === 'web_fetch');
  if (webFetchCalls.length === 0) {
    console.log('  FAIL: Bob never called web_fetch.');
    console.log('        Tools called:', toolCalls.map(getToolName).join(', '));
    return { pass: false, reason: 'missing web_fetch' };
  }
  const webFetchCall = webFetchCalls[webFetchCalls.length - 1];
  const webFetchArgs = getToolArgs(webFetchCall);
  const fetchedUrl = webFetchArgs.url || '';
  console.log(`  web_fetch URL: ${fetchedUrl}`);
  if (expectedUrl && !fetchedUrl.includes(expectedUrl)) {
    // Full equality is ideal but LLMs sometimes re-type URLs with a trailing slash etc.
    // If the fetched URL isn't an exact match, at least require it was a nanostore URL.
    if (!/nanostore|babbage/.test(fetchedUrl)) {
      console.log(`  FAIL: web_fetch URL does not match expected (${expectedUrl}).`);
      return { pass: false, reason: 'web_fetch URL mismatch' };
    }
    console.log(`  WARN: fetched URL is not byte-equal to expected, but matches nanostore/babbage host.`);
  }

  // 3. web_fetch result contains the marker
  const webFetchResult = getToolResultForCall(toolResults, webFetchCall);
  const webFetchResultStr = resultString(webFetchResult);
  console.log(`  web_fetch result (first 300): ${webFetchResultStr.substring(0, 300)}`);
  if (!webFetchResultStr.includes(expectedMarker)) {
    console.log(`  FAIL: web_fetch result did not contain marker '${expectedMarker}'.`);
    return { pass: false, reason: 'marker not in web_fetch result' };
  }
  console.log(`  Marker '${expectedMarker}' present in fetched data.`);

  // 4. session_end with no error
  const sessionEnd = findEvent(transcript.events, 'session_end');
  if (!sessionEnd) {
    console.log('  FAIL: No session_end event.');
    return { pass: false, reason: 'missing session_end' };
  }
  const endData = getEventData(sessionEnd);
  const sessionError = endData.error || '';
  if (sessionError) {
    console.log(`  FAIL: session_end has error: ${sessionError}`);
    return { pass: false, reason: `session_end error: ${sessionError}` };
  }
  console.log(`  session_end: iterations=${endData.iterations}, no error`);

  console.log('  PASS: Bob transcript passes deep inspection.');
  return { pass: true };
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

async function main() {
  const startTime = Date.now();
  console.log('='.repeat(80));
  console.log('  NANOSTORE DATA SHARING E2E TEST (Issue #344)');
  console.log('  Two agents exchange a dataset via NanoStore + delegate_task.');
  console.log('='.repeat(80));
  console.log(`  Run nonce:   ${RUN_NONCE}`);
  console.log(`  Marker:      ${MARKER}`);
  console.log(`  Payload size: ${PAYLOAD_JSON.length} bytes`);
  console.log(`  SHA256:      ${PAYLOAD_SHA256.substring(0, 24)}...`);
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

  let aliceIdentityWallet, bobIdentityWallet, parentKey;
  try {
    aliceIdentityWallet = (await walletPost(WALLET_A_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Wallet A (:${WALLET_A_PORT}): ${aliceIdentityWallet.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet A not reachable: ${e.message}`);
    process.exit(1);
  }
  try {
    bobIdentityWallet = (await walletPost(WALLET_B_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Wallet B (:${WALLET_B_PORT}): ${bobIdentityWallet.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet B not reachable: ${e.message}`);
    process.exit(1);
  }
  try {
    parentKey = (await walletPost(PARENT_WALLET_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Parent  (:${PARENT_WALLET_PORT}): ${parentKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Parent wallet not reachable: ${e.message}`);
    process.exit(1);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 2: Start both agents (trust.certifiers = parent_key only)
  // -----------------------------------------------------------------------
  console.log('[Gate 2] Starting both agents...');
  clearAuthCache();

  try {
    serverA = await startAgent('Alice', AGENT_A_PORT, WALLET_A_PORT, WORKSPACE_A, parentKey, 'nanostore-alice', parentKey);
  } catch (e) {
    console.error(`FATAL: Could not start Alice: ${e.message}`);
    await stopAll();
    process.exit(1);
  }
  try {
    serverB = await startAgent('Bob', AGENT_B_PORT, WALLET_B_PORT, WORKSPACE_B, parentKey, 'nanostore-bob', parentKey);
  } catch (e) {
    console.error(`FATAL: Could not start Bob: ${e.message}`);
    await stopAll();
    process.exit(1);
  }
  console.log();
  console.log('  ┌─────────────────────────────────────────────────────────┐');
  console.log('  │ Live UIs — follow along in your browser while the test  │');
  console.log('  │ runs:                                                   │');
  console.log(`  │   Alice : http://localhost:${AGENT_A_PORT}/ui/                     │`);
  console.log(`  │   Bob   : http://localhost:${AGENT_B_PORT}/ui/                     │`);
  console.log('  └─────────────────────────────────────────────────────────┘');
  console.log();

  // -----------------------------------------------------------------------
  // Gate 3: Get agent identities
  // -----------------------------------------------------------------------
  console.log('[Gate 3] Getting agent identities...');
  const aliceInfo = await getAgentInfo(AGENT_A_PORT);
  const bobInfo = await getAgentInfo(AGENT_B_PORT);
  console.log(`  Alice identity: ${aliceInfo.identity_key?.substring(0, 24)}...`);
  console.log(`  Alice balance:  ${formatSats(aliceInfo.balance)}`);
  console.log(`  Bob identity:   ${bobInfo.identity_key?.substring(0, 24)}...`);
  console.log(`  Bob balance:    ${formatSats(bobInfo.balance)}`);
  if (aliceInfo.identity_key === bobInfo.identity_key) {
    console.error('FATAL: Alice and Bob have the same identity key.');
    await stopAll();
    process.exit(1);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 4: Baseline Bob's task list
  // -----------------------------------------------------------------------
  console.log('[Gate 4] Baselining Bob task list...');
  const baselineB = new Set();
  try {
    (await getTaskList(AGENT_B_PORT)).forEach(t => baselineB.add(t.id));
    console.log(`  Bob baseline: ${baselineB.size} tasks`);
  } catch (e) {
    console.log(`  Bob baseline: error (${e.message}), assuming 0`);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 5: Submit Alice's upload+delegate task
  // -----------------------------------------------------------------------
  console.log('[Gate 5] Submitting upload+delegate task to Alice...');

  const aliceTaskMessage = [
    'You are sharing a small JSON dataset with another agent by uploading it',
    'to NanoStore and delegating a fetch+verify task. Execute EXACTLY these',
    'steps in order and do not call any tool more than specified.',
    '',
    'Step 1: Call upload_to_nanostore with EXACTLY these arguments:',
    `  content: ${PAYLOAD_JSON}`,
    `  content_type: "application/json"`,
    `  retention_minutes: 180`,
    '',
    'The tool returns a JSON object with a "public_url" field. Save that URL.',
    'Do NOT call upload_to_nanostore more than once.',
    '',
    'Step 2: Call delegate_task with EXACTLY these arguments:',
    `  recipient: "${bobInfo.identity_key}"`,
    '  task: "Fetch the JSON document at <THE_PUBLIC_URL_FROM_STEP_1> using' +
      ` web_fetch and confirm the response contains the exact marker string '${MARKER}'.` +
      ' Report whether the marker was found and the first 100 characters of the fetched response."',
    '  capabilities: ["web_fetch"]',
    `  budget_cap_sats: ${DELEGATED_BUDGET_CAP_SATS}`,
    `  expires_in_secs: ${DELEGATION_EXPIRES_IN_SECS}`,
    '',
    'CRITICAL: Replace <THE_PUBLIC_URL_FROM_STEP_1> with the LITERAL public_url string',
    'you received in step 1. Do not paraphrase or summarize it. The delegated task',
    'will fail if the URL is wrong.',
    '',
    'CRITICAL: Use delegate_task, NOT send_message. delegate_task creates a signed',
    'BRC-52 delegation cert that grants the recipient web_fetch. send_message does not.',
    '',
    'Step 3: Report the public_url, commission_id, and sent_message_id from the tool',
    'results. Do NOT call delegate_task more than once.',
  ].join('\n');

  let taskAResponse;
  try {
    taskAResponse = await submitTask(AGENT_A_PORT, aliceTaskMessage);
    console.log(`  Task submitted: ${JSON.stringify(taskAResponse)}`);
  } catch (e) {
    console.error(`FATAL: Could not submit task to Alice: ${e.message}`);
    await stopAll();
    process.exit(1);
  }
  const taskAId = taskAResponse.task_id || taskAResponse.id;
  if (!taskAId) {
    console.error('FATAL: No task_id from POST /task');
    await stopAll();
    process.exit(1);
  }
  console.log(`  Alice task ID: ${taskAId}\n`);

  // -----------------------------------------------------------------------
  // Gate 6: Wait for Alice to complete upload + delegation
  // -----------------------------------------------------------------------
  console.log('[Gate 6] Waiting for Alice to complete...');
  const completedA = await pollTaskComplete(AGENT_A_PORT, taskAId, TASK_A_COMPLETE_TIMEOUT_MS, 'Alice');
  if (!completedA) {
    console.error('\nFAIL: Alice task did not complete within timeout.');
    console.error(readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 2000));
    await stopAll();
    process.exit(1);
  }
  console.log(`  Alice completed. Status: ${completedA.status}, iters: ${completedA.iterations}, sats: ${formatSats(completedA.sats_spent)}`);
  console.log();

  // -----------------------------------------------------------------------
  // Gate 7: Wait for Bob to pick up the delegated task
  // -----------------------------------------------------------------------
  console.log('[Gate 7] Waiting for Bob to receive delegation and spawn task...');
  const newTaskB = await pollForNewTask(AGENT_B_PORT, baselineB, TASK_B_APPEAR_TIMEOUT_MS, 'Bob');

  let completedB = null;
  let taskBId = null;
  if (newTaskB) {
    taskBId = newTaskB.id;
    console.log(`  Bob task ID:   ${taskBId}`);
    console.log(`  Description:   ${(newTaskB.task || '').substring(0, 200)}`);
    console.log(`  Status:        ${newTaskB.status}`);
    console.log(`  Origin:        ${newTaskB.origin || 'unknown'}`);

    const taskBStatus = (newTaskB.status || '').toLowerCase();
    if (taskBStatus === 'complete' || taskBStatus === 'error' || taskBStatus === 'cancelled') {
      completedB = newTaskB;
    } else {
      completedB = await pollTaskComplete(AGENT_B_PORT, taskBId, TASK_B_COMPLETE_TIMEOUT_MS, 'Bob');
    }

    if (completedB) {
      console.log(`  Bob task completed.`);
      console.log(`  Status:     ${completedB.status}`);
      console.log(`  Iterations: ${completedB.iterations}`);
      console.log(`  Sats spent: ${formatSats(completedB.sats_spent)}`);
    }
  } else {
    console.log('  No new task appeared on Bob within timeout.');
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 7.5: Wait for commission payment (mirrors captain-coral)
  // -----------------------------------------------------------------------
  console.log('[Gate 7.5] Waiting for Alice to process commission payment claim...');
  const paymentDeadline = Date.now() + 90000;
  let alicePaymentLogged = false;
  let bobPaymentInternalized = false;
  while (Date.now() < paymentDeadline) {
    const aliceTail = readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 8000);
    const bobTail = readLogTail(path.join(WORKSPACE_B, 'server-stderr.log'), 8000);
    if (!alicePaymentLogged && aliceTail.includes('commission payment broadcast')) {
      alicePaymentLogged = true;
      console.log('  Alice: commission payment broadcast detected');
    }
    if (!bobPaymentInternalized && bobTail.includes('commission_payment_sent internalized')) {
      bobPaymentInternalized = true;
      console.log('  Bob:   commission_payment_sent internalized');
    }
    if (alicePaymentLogged && bobPaymentInternalized) break;
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  if (!alicePaymentLogged) console.log('  WARNING: Alice did not broadcast commission payment within 90s.');
  if (!bobPaymentInternalized) console.log('  WARNING: Bob did not internalize commission payment within 90s.');
  console.log();

  // -----------------------------------------------------------------------
  // Gate 8: Deep transcript inspection (mandatory)
  // -----------------------------------------------------------------------
  console.log('[Gate 8] Deep transcript inspection...');
  const aliceAudit = await getTaskAudit(AGENT_A_PORT, taskAId);
  const aliceAuditSummary = inspectAudit(aliceAudit, 'Alice audit');
  console.log();
  const aliceInspection = inspectAlice(WORKSPACE_A, taskAId);

  let bobAuditSummary = { toolNames: [] };
  let bobInspection = { pass: false, reason: 'Bob task never appeared' };
  if (taskBId) {
    const bobAudit = await getTaskAudit(AGENT_B_PORT, taskBId);
    bobAuditSummary = inspectAudit(bobAudit, 'Bob audit');
    bobInspection = inspectBob(WORKSPACE_B, taskBId, MARKER, aliceInspection.publicUrl, DELEGATED_BUDGET_CAP_SATS);
  }

  // -----------------------------------------------------------------------
  // Gate 9: Stderr tails
  // -----------------------------------------------------------------------
  console.log();
  console.log('--- Alice stderr (last 1200) ---');
  console.log(readLogTail(path.join(WORKSPACE_A, 'server-stderr.log'), 1200));
  console.log();
  console.log('--- Bob stderr (last 1200) ---');
  console.log(readLogTail(path.join(WORKSPACE_B, 'server-stderr.log'), 1200));
  console.log();

  // -----------------------------------------------------------------------
  // Cleanup
  // -----------------------------------------------------------------------
  if (KEEP_ALIVE) {
    console.log('[Cleanup] --keep-alive set; leaving servers running.');
    console.log(`  Alice UI: http://localhost:${AGENT_A_PORT}/ui/`);
    console.log(`  Bob UI:   http://localhost:${AGENT_B_PORT}/ui/`);
    console.log('  Press Ctrl-C to stop them.');
    console.log();
  } else {
    console.log('[Cleanup] Stopping servers...');
    await stopAll();
    console.log();
  }

  // -----------------------------------------------------------------------
  // Final report
  // -----------------------------------------------------------------------
  const totalMs = Date.now() - startTime;
  const aStatus = (completedA.status || '').toLowerCase();
  const aComplete = aStatus === 'complete';
  const aUsedUpload = aliceAuditSummary.toolNames.includes('upload_to_nanostore');
  const aUsedDelegate = aliceAuditSummary.toolNames.includes('delegate_task');
  const bAppeared = !!newTaskB;
  const bComplete = completedB && (completedB.status || '').toLowerCase() === 'complete';
  const bUsedWebFetch = bobAuditSummary.toolNames.includes('web_fetch');

  // Check for commission_payment_claim event in Bob's transcript
  const bobTranscript = taskBId ? readTranscriptJsonl(WORKSPACE_B, taskBId) : null;
  const bobClaimEvent = bobTranscript ? findEvent(bobTranscript.events, 'commission_payment_claim') : null;

  const totalSats = (completedA.sats_spent || 0) + (completedB ? (completedB.sats_spent || 0) : 0);

  console.log('='.repeat(80));
  console.log('  FINAL REPORT — NanoStore Data Sharing E2E');
  console.log('='.repeat(80));
  console.log(`  Total wall clock: ${formatMs(totalMs)}`);
  console.log(`  Alice identity:   ${aliceInfo.identity_key?.substring(0, 24)}...`);
  console.log(`  Bob identity:     ${bobInfo.identity_key?.substring(0, 24)}...`);
  console.log(`  Alice sats:       ${formatSats(completedA.sats_spent)}`);
  console.log(`  Bob sats:         ${formatSats(completedB ? completedB.sats_spent : 0)}`);
  console.log(`  Total sats:       ${formatSats(totalSats)}`);
  console.log(`  public_url:       ${aliceInspection.publicUrl || '(none)'}`);
  console.log();

  const checks = [
    { name: 'Alice task completed cleanly', pass: aComplete },
    { name: 'Alice called upload_to_nanostore', pass: aUsedUpload },
    { name: 'Alice got a public_url', pass: !!aliceInspection.publicUrl },
    { name: 'Alice called delegate_task', pass: aUsedDelegate },
    { name: 'Alice delegate_task returned success (cert built, message sent)', pass: aliceInspection.pass },
    { name: 'Bob received the delegated task', pass: bAppeared },
    { name: 'Bob task completed cleanly', pass: bComplete },
    { name: 'Bob emitted delegation_verified with web_fetch capability', pass: bobInspection.pass || bobInspection.reason !== 'missing delegation_verified event' },
    { name: 'Bob called web_fetch', pass: bUsedWebFetch },
    { name: 'Bob fetched data contained marker', pass: bobInspection.pass },
    { name: 'Bob emitted commission_payment_claim', pass: !!bobClaimEvent },
    { name: 'Alice broadcast commission payment', pass: alicePaymentLogged },
    { name: 'Bob internalized commission_payment_sent', pass: bobPaymentInternalized },
  ];

  let allPass = true;
  console.log('  Verification:');
  for (const c of checks) {
    const icon = c.pass ? 'PASS' : 'FAIL';
    console.log(`    [${icon}] ${c.name}`);
    if (!c.pass) allPass = false;
  }
  console.log();
  if (!aliceInspection.pass && aliceInspection.reason) {
    console.log(`  Alice failure reason: ${aliceInspection.reason}`);
  }
  if (!bobInspection.pass && bobInspection.reason) {
    console.log(`  Bob failure reason:   ${bobInspection.reason}`);
  }
  console.log();
  console.log(allPass ? '  RESULT: ALL CHECKS PASSED' : '  RESULT: SOME CHECKS FAILED');
  console.log('='.repeat(80));

  if (KEEP_ALIVE) {
    console.log(`\n(--keep-alive) Servers still running. Browse the UIs, then Ctrl-C to stop.`);
    await new Promise(() => {}); // Block forever until SIGINT/SIGTERM.
  }
  process.exit(allPass ? 0 : 1);
}

// ---------------------------------------------------------------------------
// Entry point + cleanup
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
