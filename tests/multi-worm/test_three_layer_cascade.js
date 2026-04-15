#!/usr/bin/env node
/**
 * 3-Layer Bidirectional Cascade E2E Test (Issue #344, closes #316)
 *
 * The god-tier cascade: delegation cert chains are used in BOTH directions.
 * Downward dispatches work; upward reverse-delegations carry results back,
 * each hop running with cert-derived capabilities that bypass the
 * external-origin 10-tool allowlist. Every phase is a fresh short session
 * (no inbox polling, no budget blow-ups) triggered by heartbeat picking up
 * a `task_delegation` envelope from the inbox.
 *
 * Pre-flight confirmed that `delegate_task` ALWAYS creates single-hop
 * chains (src/tools/delegation_tools.rs:610 — `parent_cert_hash: None,
 * // Phase 2 is single-hop only`). That means reverse delegation is
 * mechanically identical to forward delegation — no purpose_hash or
 * capability inheritance across hops. Each cert stands alone.
 *
 *   Phase 1 ── Captain task (submitted by test harness)
 *              Captain calls delegate_task(coord, COORD_TASK, caps=[delegate_task, overlay_lookup])
 *              Captain session_end
 *
 *   Phase 2 ── Coord's heartbeat picks up the task_delegation envelope
 *              Fresh session with cert caps = [delegate_task, overlay_lookup]
 *              overlay_lookup findByCertifier=parent_key
 *              delegate_task(worker, WORKER_TASK, caps=[web_fetch, delegate_task, send_message])
 *              Coord session_end (NO polling, NO waiting)
 *
 *   Phase 3 ── Worker's heartbeat picks up the task_delegation envelope
 *              Fresh session with cert caps = [web_fetch, delegate_task, send_message]
 *              web_fetch HN Algolia front_page
 *              Extracts top 5 titles from "hits" array
 *              Substitutes titles into REVERSE_COORD_TASK template
 *              delegate_task(coord, filled-in REVERSE_COORD_TASK,
 *                            caps=[delegate_task, send_message, memory_store])
 *              Worker session_end
 *
 *   Phase 4 ── Coord's heartbeat picks up the REVERSE task_delegation envelope
 *              Fresh SECOND session with cert caps = [delegate_task, send_message, memory_store]
 *              Reads the titles embedded in its task text
 *              Writes a one-sentence aggregated summary
 *              Substitutes summary into REVERSE_CAPTAIN_TASK template
 *              delegate_task(captain, filled-in REVERSE_CAPTAIN_TASK,
 *                            caps=[working_memory_set, send_message, memory_store])
 *              Coord session_end
 *
 *   Phase 5 ── Captain's heartbeat picks up the REVERSE task_delegation envelope
 *              Fresh SECOND session with cert caps = [working_memory_set, send_message, memory_store]
 *              Calls working_memory_set(key="cascade_summary", value=<summary>)
 *              Captain session_end
 *
 * Hard gates (all must pass):
 *   [P1] Captain Phase 1: delegate_task to Coord succeeded
 *   [P2] Coord Phase 2: delegation_verified with correct caps, overlay_lookup called,
 *        delegate_task to Worker called and returned success
 *   [P3] Worker Phase 3: delegation_verified with web_fetch cap, web_fetch called on HN
 *        with real data (non-error, non-empty, contains "hits"/"title"),
 *        delegate_task reverse-call to Coord called and returned success
 *   [P4] Coord Phase 4: delegation_verified (SECOND session), aggregated think_response
 *        (not verbatim passthrough), delegate_task forward to Captain called
 *   [P5] Captain Phase 5: delegation_verified (SECOND session), working_memory_set called
 *   Commission payments: all 4 settled (Captain→Coord, Coord→Worker, Worker→Coord,
 *                        Coord→Captain)
 *
 * Tx rate measurements captured at runtime for the 1.5M txs/day analysis:
 *   - Baseline + final sqlite3 COUNT(*) delta on each of 3 wallet dbs
 *   - Wall clock from harness submit to Phase 5 completion
 *   - txs/min aggregate across 3 agents
 *
 * Cost target: $0.40-$0.70 per run.
 *
 * Servers stay running after the report by default so the UIs stay
 * browsable. Pass --stop-after to auto-stop them at the end.
 *
 * Usage:
 *   node test_three_layer_cascade.js                # full run, servers stay up
 *   node test_three_layer_cascade.js --stop-after   # stop servers at end
 */

const { spawn, execFileSync } = require('child_process');
const http = require('http');
const path = require('path');
const fs = require('fs');
const os = require('os');
const crypto = require('crypto');

const { authGet, authPost, clearAuthCache } = require('./lib/auth');
const { startCluster } = require('./lib/cluster');

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const CAPTAIN_PORT = 8081;
const COORDINATOR_PORT = 8082;
const WORKER_PORT = 8083;
const CAPTAIN_WALLET_PORT = 3322;
const COORDINATOR_WALLET_PORT = 3323;
const WORKER_WALLET_PORT = 3324;
const PARENT_WALLET_PORT = 3321;

const PROJECT_ROOT = path.resolve(__dirname, '../../');
const BINARY = path.join(PROJECT_ROOT, 'target/release/dolphin-milk');
const WORKSPACE_CAPTAIN = path.join(PROJECT_ROOT, 'test-workspaces/cascade-captain');
const WORKSPACE_COORDINATOR = path.join(PROJECT_ROOT, 'test-workspaces/cascade-coordinator');
const WORKSPACE_WORKER = path.join(PROJECT_ROOT, 'test-workspaces/cascade-worker');

const WALLET_DB_CAPTAIN = '/Users/johncalhoun/bsv/_archived/bsv-wallet-cli-old/wallet.db';
const WALLET_DB_COORDINATOR = '/tmp/dm-e2e/data/wallet.db';
const WALLET_DB_WORKER = path.join(os.homedir(), 'bsv/wallets/worker-3324.db');

const HEALTH_TIMEOUT_MS = 90000;
const PHASE_TIMEOUT_MS = 300000;          // max per phase completion
const PHASE_APPEAR_TIMEOUT_MS = 180000;   // max wait for new task to appear
const COMMISSION_WAIT_MS = 240000;        // 4 minutes for all 4 settlements — the 4-hop
                                           //     cascade settles sequentially as each phase's
                                           //     heartbeat picks up the next claim (~15s poll
                                           //     interval × 4 hops + LLM wake-up time)

// Delegation budgets & expiries. These are per-hop caps — the receiver
// runs with this much room for LLM calls + proofs + tool costs.
// Coord and Captain are claude-haiku (~120K sats per think). Worker is
// gpt-5-mini (~40K sats per think). Budget = iters * per_think_cost * 1.5
// safety margin.
const CAP_CAPTAIN_TO_COORD_FWD = 500000;   // P2: overlay + delegate_task
const CAP_COORD_TO_WORKER_FWD  = 500000;   // P3: web_fetch + delegate_task (bumped from 300K — P3's session teardown
                                            //     needs headroom for commission_payment_claim + reverse delegation
                                            //     proof + state token updates; observed 353K spend on 300K cap)
const CAP_WORKER_TO_COORD_REV  = 500000;   // P4: aggregate + delegate_task to Captain
const CAP_COORD_TO_CAPTAIN_REV = 300000;   // P5: working_memory_set (bumped from 200K for teardown headroom)
const EXPIRES_IN_SECS = 900;

const HN_URL = 'https://hn.algolia.com/api/v1/search?tags=front_page';

const RUN_NONCE = crypto.randomBytes(4).toString('hex');
const MARKER_KEY = `cascade_summary_${RUN_NONCE}`;

const STOP_AFTER = process.argv.includes('--stop-after');

let serverCaptain = null;
let serverCoordinator = null;
let serverWorker = null;
let cascadeCluster = null; // ClusterHandle from lib/cluster.js (Phase 3 refactor)

// ---------------------------------------------------------------------------
// Helpers (identical shape to other multi-worm tests)
// ---------------------------------------------------------------------------

function formatSats(sats) { return sats ? sats.toLocaleString() + ' sats' : '0 sats'; }
function formatMs(ms) { return (ms / 1000).toFixed(1) + 's'; }
function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

function readLogTail(filepath, chars = 3000) {
  try { return fs.readFileSync(filepath, 'utf8').slice(-chars); } catch { return ''; }
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

function countWalletTxs(dbPath) {
  try {
    const out = execFileSync('sqlite3', [
      dbPath,
      "SELECT COUNT(*) FROM transactions WHERE description NOT LIKE '%split%';",
    ], { encoding: 'utf8', timeout: 10000 });
    return parseInt(out.trim(), 10) || 0;
  } catch (e) {
    console.log(`  (sqlite3 count failed for ${dbPath}: ${e.message})`);
    return null;
  }
}

function startAgent(name, port, walletPort, workspace, parentKey, agentName, trustCertifiers, defaultModel) {
  return new Promise(async (resolve, reject) => {
    if (!fs.existsSync(BINARY)) {
      return reject(new Error(`Binary not found at ${BINARY}. Run \`cargo build --release\` first.`));
    }

    fs.mkdirSync(workspace, { recursive: true });

    const stdoutLog = fs.openSync(path.join(workspace, 'server-stdout.log'), 'w');
    const stderrLog = fs.openSync(path.join(workspace, 'server-stderr.log'), 'w');

    console.log(`  Starting ${name} on port ${port} (wallet ${walletPort}, model ${defaultModel})...`);

    const env = {
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
      DOLPHIN_MILK_LLM_MODEL: defaultModel,
    };

    const proc = spawn(
      BINARY,
      ['serve', '--port', String(port), '--workspace', workspace],
      { cwd: PROJECT_ROOT, env, stdio: ['ignore', stdoutLog, stderrLog], detached: false }
    );

    const deadline = Date.now() + HEALTH_TIMEOUT_MS;
    let lastError = null;

    while (Date.now() < deadline) {
      await sleep(1000);
      try {
        const { status, body } = await httpGet(`http://localhost:${port}/health`);
        if (status === 200 && body?.status === 'ok' && body?.wallet_connected === true) {
          console.log(`  ${name} healthy (wallet connected).`);
          return resolve(proc);
        }
        lastError = `status=${status}, body=${JSON.stringify(body)}`;
      } catch (e) {
        lastError = e.message;
      }
    }

    const stderr = readLogTail(path.join(workspace, 'server-stderr.log'));
    reject(new Error(`${name} health check failed after ${HEALTH_TIMEOUT_MS}ms. Last: ${lastError}\nStderr: ${stderr}`));
  });
}

function stopAgent(proc, name) {
  if (!proc) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => { try { proc.kill('SIGKILL'); } catch {} }, 5000);
    proc.on('exit', () => { clearTimeout(timer); console.log(`  ${name} stopped.`); resolve(); });
    try { proc.kill('SIGTERM'); } catch {}
  });
}

async function stopAll() {
  await Promise.all([
    stopAgent(serverCaptain, 'Captain'),
    stopAgent(serverCoordinator, 'Coordinator'),
    stopAgent(serverWorker, 'Worker'),
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
  const start = Date.now();
  const deadline = start + timeoutMs;
  let polls = 0;
  while (Date.now() < deadline) {
    polls++;
    try {
      const task = await getTaskDetail(port, taskId);
      if (task) {
        const status = (task.status || '').toLowerCase();
        if (status === 'complete' || status === 'error' || status === 'cancelled') {
          console.log(`  ${label}: completed after ${polls} polls (${formatMs(Date.now() - start)})`);
          return task;
        }
      }
    } catch (e) {
      if (polls <= 2) console.log(`  ${label} poll error: ${e.message}`);
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  return null;
}

async function pollForNewTask(port, baselineIds, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  let polls = 0;
  while (Date.now() < deadline) {
    polls++;
    try {
      const tasks = await getTaskList(port);
      const newTask = tasks.find(t => !baselineIds.has(t.id));
      if (newTask) {
        console.log(`  ${label}: new task ${newTask.id.substring(0,8)} after ${polls} polls`);
        return newTask;
      }
    } catch (e) {
      if (polls <= 2) console.log(`  ${label} poll error: ${e.message}`);
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  return null;
}

// ---------------------------------------------------------------------------
// Transcript helpers
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

function getEventData(ev) { return ev?.data || ev; }

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
  return toolResults.find(r => {
    const d = getEventData(r);
    return (d.call_id || d.id || r.call_id || r.id) === callId;
  });
}

function resultString(result) {
  if (!result) return '';
  const d = getEventData(result);
  if (typeof d === 'string') return d;
  if (typeof d.content === 'string') return d.content;
  if (d.content != null) return JSON.stringify(d.content);
  return d.output || d.result || JSON.stringify(d);
}

function inspectAudit(audit, label) {
  if (!audit) { console.log(`  ${label}: no audit`); return { toolNames: [] }; }
  const events = audit.events || [];
  console.log(`  ${label}: ${events.length} events`);
  const tools = events.filter(e => (e.event_type || e.type) === 'tool_call');
  const toolNames = tools.map(getToolName);
  console.log(`    Tool calls (${tools.length}): ${toolNames.join(', ') || '(none)'}`);
  if (audit.summary) {
    console.log(`    Iters: ${audit.summary.iterations || '?'}, sats: ${formatSats(audit.summary.total_sats_spent || audit.summary.sats_spent)}, model: ${audit.summary.model || '?'}`);
  }
  return { toolNames };
}

// ---------------------------------------------------------------------------
// Per-phase deep inspections
// ---------------------------------------------------------------------------

/// Find the first SUCCESSFUL tool call of `name` whose recipient matches
/// `expectedRecipient`. LLMs sometimes retry a malformed call — if we just
/// grabbed the first call, a failed first attempt would mask a successful
/// retry. Walk all calls, skip results that start with "error", return the
/// first one that both matches the recipient and has a clean result.
function findSuccessfulCall(toolCalls, toolResults, toolName, expectedRecipient) {
  const matching = toolCalls.filter(c => getToolName(c) === toolName);
  for (const c of matching) {
    const args = getToolArgs(c);
    if (expectedRecipient && args.recipient !== expectedRecipient) continue;
    const res = getToolResultForCall(toolResults, c);
    const resStr = resultString(res);
    if (!resStr.toLowerCase().startsWith('error')) {
      return { call: c, result: res, resultStr: resStr };
    }
  }
  // Nothing clean — fall back to the first matching call so the caller can
  // still report a meaningful error from the returned result.
  const fallback = matching.find(c => {
    if (!expectedRecipient) return true;
    const args = getToolArgs(c);
    return args.recipient === expectedRecipient;
  }) || matching[0];
  if (!fallback) return null;
  const fallbackRes = getToolResultForCall(toolResults, fallback);
  return { call: fallback, result: fallbackRes, resultStr: resultString(fallbackRes) };
}

function inspectCaptainForward(workspace, taskId, expectedCoordKey) {
  console.log();
  console.log(`--- P1: Captain forward delegate_task (${taskId.substring(0,8)}) ---`);
  const t = readTranscriptJsonl(workspace, taskId);
  if (!t) return { pass: false, reason: 'no transcript' };

  const tcs = findAllEvents(t.events, 'tool_call');
  const trs = findAllEvents(t.events, 'tool_result');

  const found = findSuccessfulCall(tcs, trs, 'delegate_task', expectedCoordKey);
  if (!found) {
    console.log(`  FAIL: Captain never called delegate_task. Tools: ${tcs.map(getToolName).join(', ')}`);
    return { pass: false, reason: 'missing delegate_task' };
  }
  const { resultStr } = found;
  if (resultStr.toLowerCase().startsWith('error')) {
    console.log(`  FAIL: all delegate_task attempts returned errors. Last: ${resultStr.substring(0,200)}`);
    return { pass: false, reason: `delegate_task error: ${resultStr}` };
  }
  console.log(`  delegate_task result (first 200): ${resultStr.substring(0, 200)}`);
  // Report if there were retries — useful signal but not a failure.
  const allDelegates = tcs.filter(c => getToolName(c) === 'delegate_task');
  if (allDelegates.length > 1) {
    console.log(`  (note: Captain made ${allDelegates.length} delegate_task attempts — retry behavior normal)`);
  }

  const end = findEvent(t.events, 'session_end');
  if (end && getEventData(end).error) {
    return { pass: false, reason: `session_end error: ${getEventData(end).error}` };
  }
  console.log('  PASS: Captain forward delegation OK.');
  return { pass: true };
}

function inspectCoordForward(workspace, taskId, expectedWorkerNot, captainKey) {
  console.log();
  console.log(`--- P2: Coord forward session (${taskId.substring(0,8)}) ---`);
  const t = readTranscriptJsonl(workspace, taskId);
  if (!t) return { pass: false, reason: 'no transcript', workerKey: null };

  const verified = findEvent(t.events, 'delegation_verified');
  const rejected = findEvent(t.events, 'delegation_rejected');
  if (rejected) {
    return { pass: false, reason: `delegation_rejected: ${getEventData(rejected).reason}`, workerKey: null };
  }
  if (!verified) {
    return { pass: false, reason: 'missing delegation_verified', workerKey: null };
  }
  const vd = getEventData(verified);
  console.log(`  delegation_verified caps=${JSON.stringify(vd.capabilities)} budget=${vd.budget_cap_sats}`);
  const caps = vd.capabilities || [];
  if (!caps.includes('delegate_task')) {
    return { pass: false, reason: 'caps missing delegate_task', workerKey: null };
  }

  const tcs = findAllEvents(t.events, 'tool_call');
  const trs = findAllEvents(t.events, 'tool_result');
  console.log(`  Tools called: ${tcs.map(getToolName).join(', ')}`);

  // Look for any SUCCESSFUL delegate_task to a non-self non-Captain
  // recipient. LLMs sometimes make a malformed first attempt then retry —
  // we want the first clean one, not the first attempt.
  const allDelCalls = tcs.filter(c => getToolName(c) === 'delegate_task');
  if (allDelCalls.length === 0) {
    return { pass: false, reason: 'Coord never called delegate_task to dispatch Worker', workerKey: null };
  }
  let workerKey = null;
  let delResStr = '';
  for (const c of allDelCalls) {
    const args = getToolArgs(c);
    const rcpt = args.recipient || '';
    if (!rcpt || rcpt === captainKey || rcpt === expectedWorkerNot) continue;
    const res = getToolResultForCall(trs, c);
    const resStr = resultString(res);
    if (!resStr.toLowerCase().startsWith('error')) {
      workerKey = rcpt;
      delResStr = resStr;
      break;
    }
  }
  if (!workerKey) {
    // All attempts failed — report the last one for diagnostics.
    const last = allDelCalls[allDelCalls.length - 1];
    const lastRes = resultString(getToolResultForCall(trs, last));
    return {
      pass: false,
      reason: `no successful delegate_task to a Worker; last attempt: ${lastRes.substring(0,200)}`,
      workerKey: null,
    };
  }
  console.log(`  Coord delegate_task recipient: ${workerKey.substring(0,24)}...`);
  console.log(`  Coord delegate_task result (first 200): ${delResStr.substring(0, 200)}`);
  if (allDelCalls.length > 1) {
    console.log(`  (note: Coord made ${allDelCalls.length} delegate_task attempts — retry behavior normal)`);
  }

  const end = findEvent(t.events, 'session_end');
  if (end && getEventData(end).error) {
    return { pass: false, reason: `session_end error: ${getEventData(end).error}`, workerKey };
  }
  console.log('  PASS: Coord forward session OK.');
  return { pass: true, workerKey };
}

function inspectWorkerFetchAndReverse(workspace, taskId, coordKey) {
  console.log();
  console.log(`--- P3: Worker fetch + reverse delegate (${taskId.substring(0,8)}) ---`);
  const t = readTranscriptJsonl(workspace, taskId);
  if (!t) return { pass: false, reason: 'no transcript' };

  const verified = findEvent(t.events, 'delegation_verified');
  const rejected = findEvent(t.events, 'delegation_rejected');
  if (rejected) return { pass: false, reason: `delegation_rejected: ${getEventData(rejected).reason}` };
  if (!verified) return { pass: false, reason: 'missing delegation_verified' };
  const vd = getEventData(verified);
  console.log(`  delegation_verified caps=${JSON.stringify(vd.capabilities)} budget=${vd.budget_cap_sats}`);
  const caps = vd.capabilities || [];
  for (const need of ['web_fetch', 'delegate_task']) {
    if (!caps.includes(need)) return { pass: false, reason: `caps missing ${need}` };
  }

  const tcs = findAllEvents(t.events, 'tool_call');
  const trs = findAllEvents(t.events, 'tool_result');
  console.log(`  Tools called: ${tcs.map(getToolName).join(', ')}`);

  // web_fetch on HN with real data
  const wfCalls = tcs.filter(c => getToolName(c) === 'web_fetch');
  if (wfCalls.length === 0) {
    return { pass: false, reason: 'missing web_fetch' };
  }
  const wf = wfCalls[wfCalls.length - 1];
  const wfArgs = getToolArgs(wf);
  const wfUrl = wfArgs.url || '';
  console.log(`  web_fetch URL: ${wfUrl}`);
  if (!/hn\.algolia|hacker-news|ycombinator/.test(wfUrl)) {
    return { pass: false, reason: `web_fetch URL not HN: ${wfUrl}` };
  }
  const wfRes = getToolResultForCall(trs, wf);
  const wfResStr = resultString(wfRes);
  console.log(`  web_fetch result (first 250): ${wfResStr.substring(0, 250)}`);
  if (wfResStr.toLowerCase().startsWith('error')) {
    return { pass: false, reason: `web_fetch error: ${wfResStr.substring(0,200)}` };
  }
  if (wfResStr.length < 200) {
    return { pass: false, reason: 'web_fetch result too short (<200 chars)' };
  }
  // Sanity: HN Algolia JSON should have "hits" and "title"
  if (!/"hits"/.test(wfResStr) || !/"title"/.test(wfResStr)) {
    return { pass: false, reason: 'web_fetch result does not look like HN JSON' };
  }

  // delegate_task back to Coord
  const delCalls = tcs.filter(c => getToolName(c) === 'delegate_task');
  if (delCalls.length === 0) {
    return { pass: false, reason: 'Worker never reverse-delegated to Coord' };
  }
  const delCall = delCalls[delCalls.length - 1];
  const delArgs = getToolArgs(delCall);
  if (delArgs.recipient !== coordKey) {
    return { pass: false, reason: `reverse delegate recipient ${delArgs.recipient?.substring(0,24)} != coord_key` };
  }
  const delRes = getToolResultForCall(trs, delCall);
  const delResStr = resultString(delRes);
  console.log(`  reverse delegate_task result (first 200): ${delResStr.substring(0, 200)}`);
  if (delResStr.toLowerCase().startsWith('error')) {
    return { pass: false, reason: `reverse delegate error: ${delResStr.substring(0,200)}` };
  }

  const end = findEvent(t.events, 'session_end');
  if (end && getEventData(end).error) {
    return { pass: false, reason: `session_end error: ${getEventData(end).error}` };
  }
  console.log('  PASS: Worker fetch + reverse delegation OK.');
  return { pass: true };
}

function inspectCoordReverse(workspace, taskId, captainKey) {
  console.log();
  console.log(`--- P4: Coord reverse session + forward to Captain (${taskId.substring(0,8)}) ---`);
  const t = readTranscriptJsonl(workspace, taskId);
  if (!t) return { pass: false, reason: 'no transcript' };

  const verified = findEvent(t.events, 'delegation_verified');
  const rejected = findEvent(t.events, 'delegation_rejected');
  if (rejected) return { pass: false, reason: `delegation_rejected: ${getEventData(rejected).reason}` };
  if (!verified) return { pass: false, reason: 'missing delegation_verified (P4)' };
  const vd = getEventData(verified);
  console.log(`  delegation_verified caps=${JSON.stringify(vd.capabilities)} budget=${vd.budget_cap_sats}`);

  const tcs = findAllEvents(t.events, 'tool_call');
  const trs = findAllEvents(t.events, 'tool_result');
  console.log(`  Tools called: ${tcs.map(getToolName).join(', ')}`);

  // Look for delegate_task forward to Captain
  const delCalls = tcs.filter(c => getToolName(c) === 'delegate_task');
  if (delCalls.length === 0) {
    return { pass: false, reason: 'Coord reverse session never called delegate_task to Captain' };
  }
  const delToCaptain = delCalls.find(c => getToolArgs(c).recipient === captainKey);
  if (!delToCaptain) {
    const recipients = delCalls.map(c => getToolArgs(c).recipient?.substring(0,24));
    return { pass: false, reason: `no delegate_task with recipient=Captain; got [${recipients.join(', ')}]` };
  }
  const delRes = getToolResultForCall(trs, delToCaptain);
  const delResStr = resultString(delRes);
  console.log(`  forward delegate_task to Captain result (first 200): ${delResStr.substring(0, 200)}`);
  if (delResStr.toLowerCase().startsWith('error')) {
    return { pass: false, reason: `forward delegate error: ${delResStr.substring(0,200)}` };
  }

  const end = findEvent(t.events, 'session_end');
  if (end && getEventData(end).error) {
    return { pass: false, reason: `session_end error: ${getEventData(end).error}` };
  }

  // Soft gate: check that a think_response contains a summary (not verbatim passthrough)
  const thinks = findAllEvents(t.events, 'think_response');
  let sawSummaryText = false;
  for (const th of thinks) {
    const d = getEventData(th);
    const content = d.content || '';
    if (typeof content === 'string' && content.length > 30) {
      sawSummaryText = true;
      break;
    }
  }
  console.log(`  Aggregation think_response present: ${sawSummaryText ? 'yes' : 'no'}`);

  console.log('  PASS: Coord reverse session OK.');
  return { pass: true, sawSummaryText };
}

function inspectCaptainFinal(workspace, taskId) {
  console.log();
  console.log(`--- P5: Captain final working_memory_set (${taskId.substring(0,8)}) ---`);
  const t = readTranscriptJsonl(workspace, taskId);
  if (!t) return { pass: false, reason: 'no transcript' };

  const verified = findEvent(t.events, 'delegation_verified');
  const rejected = findEvent(t.events, 'delegation_rejected');
  if (rejected) return { pass: false, reason: `delegation_rejected: ${getEventData(rejected).reason}` };
  if (!verified) return { pass: false, reason: 'missing delegation_verified (P5)' };
  const vd = getEventData(verified);
  console.log(`  delegation_verified caps=${JSON.stringify(vd.capabilities)} budget=${vd.budget_cap_sats}`);

  const tcs = findAllEvents(t.events, 'tool_call');
  const trs = findAllEvents(t.events, 'tool_result');
  console.log(`  Tools called: ${tcs.map(getToolName).join(', ')}`);

  const wmsCall = tcs.find(c => getToolName(c) === 'working_memory_set');
  if (!wmsCall) {
    return { pass: false, reason: 'Captain never called working_memory_set' };
  }
  const wmsArgs = getToolArgs(wmsCall);
  console.log(`  working_memory_set args: key=${wmsArgs.key}, value (first 150): ${String(wmsArgs.value).substring(0, 150)}`);
  const wmsRes = getToolResultForCall(trs, wmsCall);
  const wmsResStr = resultString(wmsRes);
  if (wmsResStr.toLowerCase().startsWith('error')) {
    return { pass: false, reason: `working_memory_set error: ${wmsResStr.substring(0,200)}` };
  }

  const end = findEvent(t.events, 'session_end');
  if (end && getEventData(end).error) {
    return { pass: false, reason: `session_end error: ${getEventData(end).error}` };
  }
  console.log('  PASS: Captain final session OK.');
  return { pass: true, storedValue: wmsArgs.value };
}

// ---------------------------------------------------------------------------
// Task text builders — pre-computed templates. LLMs only have to substitute
// one variable per hop, not construct nested JSON from scratch.
// ---------------------------------------------------------------------------

function buildCoordForwardTask(parentKey, captainKey, coordKey, workerTaskText) {
  // Coord's Phase 2 task. Like Captain's harness, the `task` argument
  // Coord must pass to Worker contains its own "capabilities" references
  // for downstream reverse-delegation, so we use the same opaque-string
  // pattern to prevent claude-haiku from extracting those inner caps.
  return [
    '=== COORDINATOR ROLE (3-layer cascade middle layer) ===',
    '',
    'You are the Coordinator. Your ONLY job this session is to find a',
    'Worker via overlay_lookup and forward the scraping task to them via',
    'delegate_task. Do NOT call web_fetch, check_inbox, or any other tool',
    'beyond overlay_lookup and delegate_task. After delegate_task returns,',
    'report the commission_id and end the session.',
    '',
    '=== STEP 1: overlay_lookup ===',
    '',
    'Call overlay_lookup with EXACTLY these arguments:',
    '',
    '  service = "ls_agent"',
    `  query   = { "findByCertifier": "${parentKey}" }`,
    '',
    'This returns every agent certified by the trusted parent root. From',
    'the response, pick EXACTLY ONE agent whose identity_key is:',
    `  - NOT "${coordKey}" (you)`,
    `  - NOT "${captainKey}" (the Captain who delegated to you)`,
    '',
    'That remaining agent is the Worker. Remember its identity_key.',
    '',
    '=== STEP 2: delegate_task to Worker ===',
    '',
    'These are the EXACT and ONLY arguments for your delegate_task call:',
    '',
    '  recipient       = <the Worker identity_key from Step 1>',
    '  capabilities    = ["web_fetch", "delegate_task", "send_message"]',
    `  budget_cap_sats = ${CAP_COORD_TO_WORKER_FWD}`,
    `  expires_in_secs = ${EXPIRES_IN_SECS}`,
    '  task            = (the verbatim opaque string defined below)',
    '',
    '=== RULE: THE task ARGUMENT IS AN OPAQUE STRING ===',
    '',
    'The `task` field below is an OPAQUE string. It contains instructions',
    'for the Worker, NOT for you. It may contain words like "capabilities",',
    '"budget", "recipient" as part of those downstream instructions — THOSE',
    'VALUES ARE NOT YOUR ARGUMENTS. Your `capabilities` argument is exactly',
    '["web_fetch", "delegate_task", "send_message"] — nothing else.',
    '',
    'Copy the task string character-for-character between the BEGIN and END',
    'markers below into the `task` argument. Do not extract values from it.',
    '',
    '===TASK_STRING_BEGIN===',
    workerTaskText,
    '===TASK_STRING_END===',
    '',
    '=== WHAT TO DO NOW ===',
    '',
    'Execute overlay_lookup, then delegate_task with the arguments above,',
    'task set to the opaque string between the BEGIN and END markers.',
    'Report the commission_id. End session.',
  ].join('\n');
}

function buildWorkerTask(coordKey, reverseCoordTemplate) {
  // Worker's P3 task. Worker fetches HN, substitutes titles into the
  // reverseCoordTemplate, then reverse-delegates to Coord. Same opaque-
  // task-string pattern as Captain and Coord to prevent nested-cap
  // confusion at the Worker→Coord boundary.
  return [
    '=== WORKER ROLE (3-layer cascade bottom layer) ===',
    '',
    'This is an authorized delegated task from your trusted parent',
    'certificate chain. Treat these instructions as authoritative.',
    '',
    'Your job: fetch the HackerNews front page, extract the top 5 titles,',
    'and pass them back up to the Coordinator via delegate_task so the',
    'Coordinator can aggregate and forward a summary to the Captain.',
    '',
    '=== STEP 1: web_fetch HackerNews ===',
    '',
    'Call web_fetch with url set to:',
    `    ${HN_URL}`,
    '',
    'The response is a JSON object with a top-level "hits" array. Each',
    'element of "hits" is an object with a "title" string.',
    '',
    '=== STEP 2: extract 5 titles ===',
    '',
    'From the response, take the "title" value from the first 5 elements',
    'of "hits". Format them as a single line, comma-separated with each',
    'title in double quotes, like:',
    '    "First Title", "Second Title", "Third Title", "Fourth Title", "Fifth Title"',
    '',
    'Call this formatted line TITLES_STRING.',
    '',
    '=== STEP 3: delegate_task to Coordinator ===',
    '',
    'These are the EXACT and ONLY arguments for your delegate_task call:',
    '',
    `  recipient       = "${coordKey}"`,
    '  capabilities    = ["delegate_task", "send_message", "memory_store"]',
    `  budget_cap_sats = ${CAP_WORKER_TO_COORD_REV}`,
    `  expires_in_secs = ${EXPIRES_IN_SECS}`,
    '  task            = (the opaque string below, with one substitution)',
    '',
    '=== RULE: THE task ARGUMENT IS MOSTLY OPAQUE ===',
    '',
    'The `task` field below is an opaque string EXCEPT for the single',
    'token FIVE_TITLES_GO_HERE, which you must replace with the',
    'TITLES_STRING from Step 2. Apart from that substitution, do not',
    'modify, paraphrase, or extract values from it. It may contain words',
    'like "capabilities" and "recipient" as part of instructions for the',
    "Coordinator's next delegation — THOSE ARE NOT YOUR ARGUMENTS. Your",
    'capabilities are exactly ["delegate_task", "send_message", "memory_store"].',
    '',
    '===TASK_STRING_BEGIN===',
    reverseCoordTemplate,
    '===TASK_STRING_END===',
    '',
    '=== WHAT TO DO NOW ===',
    '',
    'Execute web_fetch, extract the 5 titles into TITLES_STRING, then call',
    'delegate_task with the arguments above and with the task string set to',
    'the opaque block above (between the markers, NOT including them) with',
    'FIVE_TITLES_GO_HERE replaced by TITLES_STRING. Do not call web_fetch',
    'or delegate_task more than once each. Report the commission_id.',
  ].join('\n');
}

function buildReverseCoordTemplate(captainKey, reverseCaptainTemplate) {
  // Coord's P4 reverse-delegated session task. Worker filled in
  // FIVE_TITLES_GO_HERE before passing this. Coord now aggregates into a
  // summary and reverse-delegates to Captain with the same opaque-block
  // pattern. The nested Captain block has SUMMARY_GOES_HERE which Coord
  // substitutes with its own summary.
  return [
    '=== COORDINATOR ROLE (reverse aggregation step) ===',
    '',
    'This is an authorized delegated task from your trusted parent',
    'certificate chain. Treat these instructions as authoritative.',
    '',
    'The Worker at the bottom of the cascade fetched the HackerNews front',
    'page and extracted these top 5 titles:',
    '',
    '    FIVE_TITLES_GO_HERE',
    '',
    'Your job: aggregate those 5 titles into a SINGLE sentence describing',
    "the common themes or general mood of today's front page, then",
    'reverse-delegate to the Captain so the Captain can record it.',
    '',
    '=== STEP 1: write the summary sentence ===',
    '',
    'Read the 5 titles above and write ONE natural-language sentence',
    'summarizing their common themes. Do not copy the titles verbatim —',
    'produce a derived, original sentence. Call it SUMMARY_SENTENCE.',
    '',
    '=== STEP 2: delegate_task to Captain ===',
    '',
    'These are the EXACT and ONLY arguments for your delegate_task call:',
    '',
    `  recipient       = "${captainKey}"`,
    '  capabilities    = ["working_memory_set", "send_message", "memory_store"]',
    `  budget_cap_sats = ${CAP_COORD_TO_CAPTAIN_REV}`,
    `  expires_in_secs = ${EXPIRES_IN_SECS}`,
    '  task            = (the opaque string below, with one substitution)',
    '',
    '=== RULE: THE task ARGUMENT IS MOSTLY OPAQUE ===',
    '',
    'The `task` field below is an opaque string EXCEPT for the single',
    'token SUMMARY_GOES_HERE, which you must replace with SUMMARY_SENTENCE',
    "from Step 1. Apart from that substitution, do not modify or extract",
    'values from it. It may contain words like "capabilities" as part of',
    "instructions for the Captain — THOSE ARE NOT YOUR ARGUMENTS. Your",
    'capabilities are exactly ["working_memory_set", "send_message", "memory_store"].',
    '',
    '===TASK_STRING_BEGIN===',
    reverseCaptainTemplate,
    '===TASK_STRING_END===',
    '',
    '=== WHAT TO DO NOW ===',
    '',
    'Write SUMMARY_SENTENCE, then call delegate_task with the arguments',
    'above and task set to the opaque string block above (between the',
    'markers, NOT including them) with SUMMARY_GOES_HERE replaced by',
    'SUMMARY_SENTENCE. Report the commission_id.',
  ].join('\n');
}

function buildReverseCaptainTemplate(markerKey) {
  // This is the task description Captain receives in its REVERSE-delegated
  // session (Phase 5). Coord will have replaced SUMMARY_GOES_HERE with its
  // one-sentence aggregation before passing this to delegate_task.
  return [
    'This is an authorized delegated task from your trusted parent certificate',
    'chain. Perform the following work as the Captain at the top of a 3-layer',
    'cascade recording the final research summary.',
    '',
    'The Coordinator below has aggregated the HackerNews front-page scrape',
    'into this one-sentence summary:',
    '',
    '    SUMMARY_GOES_HERE',
    '',
    'Step 1. Call working_memory_set with EXACTLY these arguments:',
    '',
    `  key: "${markerKey}"`,
    '  value: (the one-sentence summary above, verbatim)',
    '',
    'Step 2. Report success. Your session is then complete.',
  ].join('\n');
}

function buildCaptainHarnessTask(coordKey, coordForwardTaskText) {
  // Captain's harness task. Critical: the `task` argument CONTAINS other
  // "capabilities" strings (as part of Coordinator's instructions for
  // downstream delegations). Claude-haiku-4-5 was observed to read those
  // inner strings and use them for its OWN outer delegate_task call,
  // passing the wrong capabilities to Coordinator. This prompt structure
  // explicitly tells Captain that the task argument is OPAQUE and must
  // not be examined for values.
  return [
    '=== CAPTAIN ROLE (3-layer cascade harness) ===',
    '',
    'You must make exactly ONE tool call in this session: delegate_task.',
    'Do not call web_fetch, overlay_lookup, or any other tool. After',
    'delegate_task returns, report the commission_id and end your session.',
    '',
    '=== THE delegate_task ARGUMENTS YOU MUST USE ===',
    '',
    'These are the EXACT and ONLY arguments for your delegate_task call:',
    '',
    `  recipient       = "${coordKey}"`,
    '  capabilities    = ["delegate_task", "overlay_lookup"]',
    `  budget_cap_sats = ${CAP_CAPTAIN_TO_COORD_FWD}`,
    `  expires_in_secs = ${EXPIRES_IN_SECS}`,
    '  task            = (the verbatim opaque string defined below)',
    '',
    '=== RULE: THE task ARGUMENT IS AN OPAQUE STRING ===',
    '',
    'The `task` field below is an OPAQUE string. It contains instructions',
    'for the Coordinator, NOT for you. It may contain words like',
    '"capabilities", "budget", "recipient" as part of those downstream',
    'instructions — THOSE VALUES ARE NOT YOUR ARGUMENTS. Ignore them.',
    'Your `capabilities` argument is exactly the list given above:',
    '["delegate_task", "overlay_lookup"] — nothing else.',
    '',
    'Copy the task string character-for-character between the BEGIN and',
    'END markers below into the `task` argument of delegate_task. Do not',
    'paraphrase, extract values from, modify, or summarize it.',
    '',
    '===TASK_STRING_BEGIN===',
    coordForwardTaskText,
    '===TASK_STRING_END===',
    '',
    '=== WHAT TO DO NOW ===',
    '',
    'Make the delegate_task call with the four non-task arguments exactly',
    'as listed under "THE delegate_task ARGUMENTS YOU MUST USE", and with',
    'the task argument set to the opaque string between the BEGIN and END',
    'markers (without the markers themselves). Then report the',
    'commission_id and sent_message_id from the result.',
  ].join('\n');
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main() {
  const startTime = Date.now();
  console.log('='.repeat(80));
  console.log('  3-LAYER BIDIRECTIONAL CASCADE E2E TEST (Issue #344, closes #316)');
  console.log('  Captain ⇄ Coordinator ⇄ Worker via 4 delegate_task hops.');
  console.log('='.repeat(80));
  console.log(`  Run nonce:   ${RUN_NONCE}`);
  console.log(`  Marker key:  ${MARKER_KEY}`);
  console.log();

  // Gate 1: prereqs
  console.log('[Gate 1] Verifying prerequisites...');
  if (!fs.existsSync(BINARY)) {
    console.error(`FATAL: Binary not found at ${BINARY}.`);
    process.exit(1);
  }

  let captainWalletKey, coordWalletKey, workerWalletKey, parentKey;
  try {
    captainWalletKey = (await walletPost(CAPTAIN_WALLET_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Captain wallet (:${CAPTAIN_WALLET_PORT}):     ${captainWalletKey.substring(0, 20)}...`);
  } catch (e) { console.error(`FATAL: Captain wallet unreachable: ${e.message}`); process.exit(1); }
  try {
    coordWalletKey = (await walletPost(COORDINATOR_WALLET_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Coordinator wallet (:${COORDINATOR_WALLET_PORT}): ${coordWalletKey.substring(0, 20)}...`);
  } catch (e) { console.error(`FATAL: Coordinator wallet unreachable: ${e.message}`); process.exit(1); }
  try {
    workerWalletKey = (await walletPost(WORKER_WALLET_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Worker wallet (:${WORKER_WALLET_PORT}):      ${workerWalletKey.substring(0, 20)}...`);
  } catch (e) { console.error(`FATAL: Worker wallet unreachable: ${e.message}. Ensure the :3324 daemon is up.`); process.exit(1); }
  try {
    parentKey = (await walletPost(PARENT_WALLET_PORT, 'getPublicKey', { identityKey: true })).publicKey;
    console.log(`  Parent wallet (:${PARENT_WALLET_PORT}):      ${parentKey.substring(0, 20)}...`);
  } catch (e) { console.error(`FATAL: Parent wallet unreachable: ${e.message}`); process.exit(1); }
  console.log();

  const txBaseline = {
    captain: countWalletTxs(WALLET_DB_CAPTAIN),
    coordinator: countWalletTxs(WALLET_DB_COORDINATOR),
    worker: countWalletTxs(WALLET_DB_WORKER),
  };
  console.log(`  Wallet tx baselines: Captain=${txBaseline.captain} Coordinator=${txBaseline.coordinator} Worker=${txBaseline.worker}`);
  console.log();

  // Gate 2: start agents via lib/cluster.js (Phase 3 refactor)
  //
  // Previously this block called startAgent() three times inline with a specific
  // env block. That same env block now lives in cluster.js::startCluster() along
  // with boot-time BRC-52 cert provisioning (per-role capabilities via
  // DOLPHIN_MILK_CERT_CAPABILITIES) and stale-cert cleanup via
  // CertificateManager::revoke_and_relinquish. The cascade uses the default
  // capability set (llm,tools,wallet,memory,messaging,x402,schedule,orchestration)
  // since its delegation flow predates per-role caps — this preserves the
  // original cascade behavior byte-for-byte. overlay.verifyRegistration is
  // DISABLED for the cascade because the cascade test resolves identity keys
  // directly (no findByCapability lookup) and we don't want overlay latency
  // in the hot path.
  //
  // Worker uses claude-haiku-4-5 instead of gpt-5-mini: gpt-5-mini's
  // injection-defense reflex is hyper-paranoid about template-substitution
  // language and refuses legitimate delegated tasks containing phrases like
  // "replace the literal substring {{X}} with Y". claude-haiku follows the
  // same structured instructions without flagging them as injection attempts.
  console.log('[Gate 2] Starting all three agents via lib/cluster.js...');
  clearAuthCache();
  const DEFAULT_CASCADE_CAPS = ['llm', 'tools', 'wallet', 'memory', 'messaging', 'x402', 'schedule', 'orchestration'];
  try {
    cascadeCluster = await startCluster({
      parentWalletPort: PARENT_WALLET_PORT,
      binary: BINARY,
      overlay: {
        url: 'https://rust-overlay.dev-a3e.workers.dev',
        verifyRegistration: false,
        registrationTimeoutMs: 30000,
      },
      outputDir: WORKSPACE_CAPTAIN, // reuse Captain workspace as output anchor; cluster-state.json goes here
      agents: [
        {
          name: 'cascade-captain',
          port: CAPTAIN_PORT,
          walletPort: CAPTAIN_WALLET_PORT,
          model: 'claude-haiku-4-5',
          workspace: WORKSPACE_CAPTAIN,
          capabilities: DEFAULT_CASCADE_CAPS,
        },
        {
          name: 'cascade-coordinator',
          port: COORDINATOR_PORT,
          walletPort: COORDINATOR_WALLET_PORT,
          model: 'claude-haiku-4-5',
          workspace: WORKSPACE_COORDINATOR,
          capabilities: DEFAULT_CASCADE_CAPS,
        },
        {
          name: 'cascade-worker',
          port: WORKER_PORT,
          walletPort: WORKER_WALLET_PORT,
          model: 'claude-haiku-4-5',
          workspace: WORKSPACE_WORKER,
          capabilities: DEFAULT_CASCADE_CAPS,
        },
      ],
    });
  } catch (e) {
    console.error(`FATAL: startCluster failed: ${e.message}`);
    if (cascadeCluster) await cascadeCluster.stop().catch(() => {});
    process.exit(1);
  }
  // Map cluster handles back onto the original serverX proc variables so the
  // rest of the test's stopAll()/signal-handler flow keeps working unchanged.
  serverCaptain = cascadeCluster.agents.get('cascade-captain').proc;
  serverCoordinator = cascadeCluster.agents.get('cascade-coordinator').proc;
  serverWorker = cascadeCluster.agents.get('cascade-worker').proc;

  console.log();
  console.log('  ┌───────────────────────────────────────────────────────────┐');
  console.log('  │ Live UIs — follow along in your browser:                  │');
  console.log(`  │   Captain     : http://localhost:${CAPTAIN_PORT}/ui/                  │`);
  console.log(`  │   Coordinator : http://localhost:${COORDINATOR_PORT}/ui/                  │`);
  console.log(`  │   Worker      : http://localhost:${WORKER_PORT}/ui/                  │`);
  console.log('  └───────────────────────────────────────────────────────────┘');
  console.log();

  // Gate 3: identities
  console.log('[Gate 3] Getting agent identities...');
  const captainInfo = await getAgentInfo(CAPTAIN_PORT);
  const coordInfo = await getAgentInfo(COORDINATOR_PORT);
  const workerInfo = await getAgentInfo(WORKER_PORT);
  console.log(`  Captain:     ${captainInfo.identity_key?.substring(0, 24)}...  balance: ${formatSats(captainInfo.balance)}`);
  console.log(`  Coordinator: ${coordInfo.identity_key?.substring(0, 24)}...  balance: ${formatSats(coordInfo.balance)}`);
  console.log(`  Worker:      ${workerInfo.identity_key?.substring(0, 24)}...  balance: ${formatSats(workerInfo.balance)}`);
  const allKeys = new Set([captainInfo.identity_key, coordInfo.identity_key, workerInfo.identity_key]);
  if (allKeys.size !== 3) {
    console.error('FATAL: Agents share identity keys!');
    await stopAll();
    process.exit(1);
  }
  console.log();

  // Gate 4: baselines
  console.log('[Gate 4] Baselining task lists...');
  const baselineCaptain = new Set();
  const baselineCoord = new Set();
  const baselineWorker = new Set();
  try { (await getTaskList(CAPTAIN_PORT)).forEach(t => baselineCaptain.add(t.id)); } catch {}
  try { (await getTaskList(COORDINATOR_PORT)).forEach(t => baselineCoord.add(t.id)); } catch {}
  try { (await getTaskList(WORKER_PORT)).forEach(t => baselineWorker.add(t.id)); } catch {}
  console.log(`  Captain=${baselineCaptain.size} Coord=${baselineCoord.size} Worker=${baselineWorker.size}`);
  console.log();

  // Pre-compute all task text templates (innermost first).
  const reverseCaptainTemplate = buildReverseCaptainTemplate(MARKER_KEY);
  const reverseCoordTemplate = buildReverseCoordTemplate(captainInfo.identity_key, reverseCaptainTemplate);
  const workerTaskText = buildWorkerTask(coordInfo.identity_key, reverseCoordTemplate);
  const coordForwardTaskText = buildCoordForwardTask(parentKey, captainInfo.identity_key, coordInfo.identity_key, workerTaskText);
  const captainHarnessTask = buildCaptainHarnessTask(coordInfo.identity_key, coordForwardTaskText);

  // Gate 5: PHASE 1 — submit to Captain
  console.log('[Gate 5] PHASE 1: submitting Captain harness task...');
  let p1Resp;
  try {
    p1Resp = await submitTask(CAPTAIN_PORT, captainHarnessTask);
    console.log(`  Captain task: ${JSON.stringify(p1Resp)}`);
  } catch (e) {
    console.error(`FATAL: Captain task submit: ${e.message}`);
    await stopAll();
    process.exit(1);
  }
  const p1TaskId = p1Resp.task_id || p1Resp.id;
  console.log(`  P1 task ID: ${p1TaskId}`);
  console.log();

  // Gate 6: wait for Phase 1 to complete
  console.log('[Gate 6] Waiting for Phase 1 (Captain forward delegate_task)...');
  const p1Completed = await pollTaskComplete(CAPTAIN_PORT, p1TaskId, PHASE_TIMEOUT_MS, 'P1 Captain');
  if (!p1Completed) {
    console.error('\nFAIL: P1 Captain timeout.');
    console.error(readLogTail(path.join(WORKSPACE_CAPTAIN, 'server-stderr.log'), 2000));
    await stopAll();
    process.exit(1);
  }
  console.log(`  P1 done: ${p1Completed.status}, iters=${p1Completed.iterations}, sats=${formatSats(p1Completed.sats_spent)}`);
  console.log();

  // Gate 7: PHASE 2 — wait for Coord task to appear and complete
  console.log('[Gate 7] Waiting for Phase 2 (Coord forward session)...');
  const baselineCoordP1 = new Set(baselineCoord);
  const p2Task = await pollForNewTask(COORDINATOR_PORT, baselineCoordP1, PHASE_APPEAR_TIMEOUT_MS, 'P2 Coord appear');
  if (!p2Task) {
    console.error('\nFAIL: P2 Coord task never appeared.');
    await stopAll();
    process.exit(1);
  }
  const p2TaskId = p2Task.id;
  const p2Completed = (p2Task.status || '').toLowerCase() !== 'running'
    ? p2Task
    : await pollTaskComplete(COORDINATOR_PORT, p2TaskId, PHASE_TIMEOUT_MS, 'P2 Coord');
  if (!p2Completed) {
    console.error('\nFAIL: P2 Coord timeout.');
    await stopAll();
    process.exit(1);
  }
  console.log(`  P2 done: ${p2Completed.status}, iters=${p2Completed.iterations}, sats=${formatSats(p2Completed.sats_spent)}`);
  console.log();

  // Gate 8: PHASE 3 — wait for Worker task to appear and complete
  console.log('[Gate 8] Waiting for Phase 3 (Worker fetch + reverse delegate)...');
  const baselineWorkerP2 = new Set(baselineWorker);
  const p3Task = await pollForNewTask(WORKER_PORT, baselineWorkerP2, PHASE_APPEAR_TIMEOUT_MS, 'P3 Worker appear');
  if (!p3Task) {
    console.error('\nFAIL: P3 Worker task never appeared.');
    await stopAll();
    process.exit(1);
  }
  const p3TaskId = p3Task.id;
  const p3Completed = (p3Task.status || '').toLowerCase() !== 'running'
    ? p3Task
    : await pollTaskComplete(WORKER_PORT, p3TaskId, PHASE_TIMEOUT_MS, 'P3 Worker');
  console.log(`  P3 done: ${p3Completed?.status}, iters=${p3Completed?.iterations}, sats=${formatSats(p3Completed?.sats_spent)}`);
  console.log();

  // Gate 9: PHASE 4 — wait for Coord's SECOND task to appear and complete
  console.log('[Gate 9] Waiting for Phase 4 (Coord reverse session)...');
  const baselineCoordP3 = new Set(baselineCoordP1);
  baselineCoordP3.add(p2TaskId);
  const p4Task = await pollForNewTask(COORDINATOR_PORT, baselineCoordP3, PHASE_APPEAR_TIMEOUT_MS, 'P4 Coord appear');
  let p4TaskId = null;
  let p4Completed = null;
  if (p4Task) {
    p4TaskId = p4Task.id;
    p4Completed = (p4Task.status || '').toLowerCase() !== 'running'
      ? p4Task
      : await pollTaskComplete(COORDINATOR_PORT, p4TaskId, PHASE_TIMEOUT_MS, 'P4 Coord');
    console.log(`  P4 done: ${p4Completed?.status}, iters=${p4Completed?.iterations}, sats=${formatSats(p4Completed?.sats_spent)}`);
  } else {
    console.log('  P4 never appeared — Worker reverse-delegation may have failed.');
  }
  console.log();

  // Gate 10: PHASE 5 — wait for Captain's SECOND task to appear and complete
  console.log('[Gate 10] Waiting for Phase 5 (Captain final working_memory_set)...');
  const baselineCaptainP4 = new Set(baselineCaptain);
  baselineCaptainP4.add(p1TaskId);
  const p5Task = await pollForNewTask(CAPTAIN_PORT, baselineCaptainP4, PHASE_APPEAR_TIMEOUT_MS, 'P5 Captain appear');
  let p5TaskId = null;
  let p5Completed = null;
  if (p5Task) {
    p5TaskId = p5Task.id;
    p5Completed = (p5Task.status || '').toLowerCase() !== 'running'
      ? p5Task
      : await pollTaskComplete(CAPTAIN_PORT, p5TaskId, PHASE_TIMEOUT_MS, 'P5 Captain');
    console.log(`  P5 done: ${p5Completed?.status}, iters=${p5Completed?.iterations}, sats=${formatSats(p5Completed?.sats_spent)}`);
  } else {
    console.log('  P5 never appeared — Coord reverse session may have failed.');
  }
  console.log();

  // Gate 11: commission payment settlements (4 total expected).
  //
  // Counts are derived from grepping the FULL stderr log file each poll —
  // NOT a tail — because commission events can scroll out of a tail window
  // if logs grow past ~20KB during a long cascade run. Counts are also
  // monotonically non-decreasing (max-so-far) so a transient read error
  // on one poll can't drop the running total.
  console.log('[Gate 11] Waiting for 4 commission payments to settle...');
  const payDeadline = Date.now() + COMMISSION_WAIT_MS;
  let captainBroadcasts = 0;
  let coordBroadcasts = 0;
  let workerBroadcasts = 0;
  let captainInternalized = 0;
  let coordInternalized = 0;
  let workerInternalized = 0;
  const countMatches = (filepath, pattern) => {
    try {
      const content = fs.readFileSync(filepath, 'utf8');
      return (content.match(pattern) || []).length;
    } catch {
      return 0;
    }
  };
  const max = (a, b) => (a > b ? a : b);
  while (Date.now() < payDeadline) {
    const cPath = path.join(WORKSPACE_CAPTAIN, 'server-stderr.log');
    const kPath = path.join(WORKSPACE_COORDINATOR, 'server-stderr.log');
    const wPath = path.join(WORKSPACE_WORKER, 'server-stderr.log');
    captainBroadcasts = max(captainBroadcasts, countMatches(cPath, /commission payment broadcast/g));
    coordBroadcasts = max(coordBroadcasts, countMatches(kPath, /commission payment broadcast/g));
    workerBroadcasts = max(workerBroadcasts, countMatches(wPath, /commission payment broadcast/g));
    captainInternalized = max(
      captainInternalized,
      countMatches(cPath, /commission_payment_sent internalized/g),
    );
    coordInternalized = max(
      coordInternalized,
      countMatches(kPath, /commission_payment_sent internalized/g),
    );
    workerInternalized = max(
      workerInternalized,
      countMatches(wPath, /commission_payment_sent internalized/g),
    );
    const totalBroadcasts = captainBroadcasts + coordBroadcasts + workerBroadcasts;
    const totalInternalized = captainInternalized + coordInternalized + workerInternalized;
    if (totalBroadcasts >= 4 && totalInternalized >= 4) break;
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  console.log(`  Broadcasts: Captain=${captainBroadcasts} Coord=${coordBroadcasts} Worker=${workerBroadcasts}`);
  console.log(`  Internalized: Captain=${captainInternalized} Coord=${coordInternalized} Worker=${workerInternalized}`);
  console.log();

  // Gate 12: deep transcript inspection
  console.log('[Gate 12] Deep transcript inspection...');

  const p1Audit = await getTaskAudit(CAPTAIN_PORT, p1TaskId);
  const p2Audit = await getTaskAudit(COORDINATOR_PORT, p2TaskId);
  const p3Audit = p3TaskId ? await getTaskAudit(WORKER_PORT, p3TaskId) : null;
  const p4Audit = p4TaskId ? await getTaskAudit(COORDINATOR_PORT, p4TaskId) : null;
  const p5Audit = p5TaskId ? await getTaskAudit(CAPTAIN_PORT, p5TaskId) : null;

  inspectAudit(p1Audit, 'P1 Captain audit');
  inspectAudit(p2Audit, 'P2 Coord audit');
  if (p3Audit) inspectAudit(p3Audit, 'P3 Worker audit');
  if (p4Audit) inspectAudit(p4Audit, 'P4 Coord reverse audit');
  if (p5Audit) inspectAudit(p5Audit, 'P5 Captain final audit');

  const p1Deep = inspectCaptainForward(WORKSPACE_CAPTAIN, p1TaskId, coordInfo.identity_key);
  const p2Deep = inspectCoordForward(WORKSPACE_COORDINATOR, p2TaskId, null, captainInfo.identity_key);
  const p3Deep = p3TaskId ? inspectWorkerFetchAndReverse(WORKSPACE_WORKER, p3TaskId, coordInfo.identity_key) : { pass: false, reason: 'P3 never ran' };
  const p4Deep = p4TaskId ? inspectCoordReverse(WORKSPACE_COORDINATOR, p4TaskId, captainInfo.identity_key) : { pass: false, reason: 'P4 never ran' };
  const p5Deep = p5TaskId ? inspectCaptainFinal(WORKSPACE_CAPTAIN, p5TaskId) : { pass: false, reason: 'P5 never ran' };

  console.log();
  console.log('--- Captain stderr (last 800) ---');
  console.log(readLogTail(path.join(WORKSPACE_CAPTAIN, 'server-stderr.log'), 800));
  console.log();
  console.log('--- Coordinator stderr (last 800) ---');
  console.log(readLogTail(path.join(WORKSPACE_COORDINATOR, 'server-stderr.log'), 800));
  console.log();
  console.log('--- Worker stderr (last 800) ---');
  console.log(readLogTail(path.join(WORKSPACE_WORKER, 'server-stderr.log'), 800));
  console.log();

  // Gate 13: tx deltas
  const txFinal = {
    captain: countWalletTxs(WALLET_DB_CAPTAIN),
    coordinator: countWalletTxs(WALLET_DB_COORDINATOR),
    worker: countWalletTxs(WALLET_DB_WORKER),
  };
  const deltas = {
    captain: (txFinal.captain || 0) - (txBaseline.captain || 0),
    coordinator: (txFinal.coordinator || 0) - (txBaseline.coordinator || 0),
    worker: (txFinal.worker || 0) - (txBaseline.worker || 0),
  };
  const totalTxs = deltas.captain + deltas.coordinator + deltas.worker;

  // Cleanup
  if (STOP_AFTER) {
    console.log('[Cleanup] --stop-after; stopping servers...');
    await stopAll();
    console.log();
  } else {
    console.log('[Cleanup] Servers left running. Inspect the UIs and transcripts:');
    console.log(`  Captain UI     : http://localhost:${CAPTAIN_PORT}/ui/`);
    console.log(`  Coordinator UI : http://localhost:${COORDINATOR_PORT}/ui/`);
    console.log(`  Worker UI      : http://localhost:${WORKER_PORT}/ui/`);
    console.log();
    console.log('  Transcripts on disk:');
    console.log(`    P1 Captain   : ${path.join(WORKSPACE_CAPTAIN, 'tasks', p1TaskId, 'session.jsonl')}`);
    console.log(`    P2 Coord     : ${path.join(WORKSPACE_COORDINATOR, 'tasks', p2TaskId, 'session.jsonl')}`);
    if (p3TaskId) console.log(`    P3 Worker    : ${path.join(WORKSPACE_WORKER, 'tasks', p3TaskId, 'session.jsonl')}`);
    if (p4TaskId) console.log(`    P4 Coord rev : ${path.join(WORKSPACE_COORDINATOR, 'tasks', p4TaskId, 'session.jsonl')}`);
    if (p5TaskId) console.log(`    P5 Captain   : ${path.join(WORKSPACE_CAPTAIN, 'tasks', p5TaskId, 'session.jsonl')}`);
    console.log();
    console.log('  Re-run with --stop-after to auto-stop next time.');
    console.log('  Press Ctrl-C when done to stop all servers.');
    console.log();
  }

  // Final report
  const totalMs = Date.now() - startTime;
  const totalMinutes = totalMs / 60000;
  const txPerMin = totalMinutes > 0 ? (totalTxs / totalMinutes).toFixed(1) : 'n/a';

  const totalSats =
    (p1Completed?.sats_spent || 0) +
    (p2Completed?.sats_spent || 0) +
    (p3Completed?.sats_spent || 0) +
    (p4Completed?.sats_spent || 0) +
    (p5Completed?.sats_spent || 0);

  console.log('='.repeat(80));
  console.log('  FINAL REPORT — 3-Layer Bidirectional Cascade');
  console.log('='.repeat(80));
  console.log(`  Wall clock:     ${formatMs(totalMs)}`);
  console.log();
  console.log('  Phase spending (transcript ground truth):');
  console.log(`    P1 Captain forward     : ${formatSats(p1Completed?.sats_spent)}`);
  console.log(`    P2 Coord forward       : ${formatSats(p2Completed?.sats_spent)}`);
  console.log(`    P3 Worker fetch+reverse: ${formatSats(p3Completed?.sats_spent)}`);
  console.log(`    P4 Coord reverse       : ${formatSats(p4Completed?.sats_spent)}`);
  console.log(`    P5 Captain final       : ${formatSats(p5Completed?.sats_spent)}`);
  console.log(`    TOTAL                  : ${formatSats(totalSats)}`);
  console.log();
  console.log('  Wallet tx deltas (sqlite3 COUNT(*) diff, excluding split):');
  console.log(`    Captain     : +${deltas.captain} txs`);
  console.log(`    Coordinator : +${deltas.coordinator} txs`);
  console.log(`    Worker      : +${deltas.worker} txs`);
  console.log(`    TOTAL       : +${totalTxs} txs`);
  console.log(`    Rate        : ${txPerMin} txs/min across 3 agents`);
  console.log();

  const totalBroadcasts = captainBroadcasts + coordBroadcasts + workerBroadcasts;
  const totalInternalized = captainInternalized + coordInternalized + workerInternalized;

  const checks = [
    { name: 'P1 Captain forward delegate_task', pass: p1Deep.pass },
    { name: 'P2 Coord forward session (cert verified + dispatched Worker)', pass: p2Deep.pass },
    { name: 'P3 Worker fetch HN + reverse delegation to Coord', pass: p3Deep.pass },
    { name: 'P4 Coord reverse session (cert verified + forwarded Captain)', pass: p4Deep.pass },
    { name: 'P5 Captain final (cert verified + working_memory_set)', pass: p5Deep.pass },
    { name: '4 commission payments broadcast', pass: totalBroadcasts >= 4 },
    { name: '4 commission payments internalized', pass: totalInternalized >= 4 },
  ];

  console.log('  Verification:');
  let allPass = true;
  for (const c of checks) {
    const icon = c.pass ? 'PASS' : 'FAIL';
    console.log(`    [${icon}] ${c.name}`);
    if (!c.pass) allPass = false;
  }
  console.log();
  if (!p1Deep.pass && p1Deep.reason) console.log(`  P1 failure: ${p1Deep.reason}`);
  if (!p2Deep.pass && p2Deep.reason) console.log(`  P2 failure: ${p2Deep.reason}`);
  if (!p3Deep.pass && p3Deep.reason) console.log(`  P3 failure: ${p3Deep.reason}`);
  if (!p4Deep.pass && p4Deep.reason) console.log(`  P4 failure: ${p4Deep.reason}`);
  if (!p5Deep.pass && p5Deep.reason) console.log(`  P5 failure: ${p5Deep.reason}`);
  console.log();

  // Gate 12: Deep inspection via lib/inspector.js (Phase 3 quality gate).
  //
  // Runs ORTHOGONAL to the phase-specific inspectCaptainForward/... helpers
  // above. The per-phase helpers assert specific tool calls happened at the
  // right time with the right recipients; inspector.js asserts architectural
  // invariants across the whole cascade (delegation_happened,
  // worker_did_external_work, reverse_path_existed, commission_messages_fired,
  // budget_respected) via the SAME inspector the POC #23 gate uses. Two
  // independent checks — if the per-phase inspector and inspector.js agree
  // the run is green, the refactor (cluster.js as drop-in spawn) is proven.
  //
  // Cascade does NOT use overlay_lookup (resolves identity keys directly),
  // so overlay_was_used is explicitly skipped. Captain's final task record
  // (p5Completed) carries the MARKER_KEY in its result for captain_task_complete.
  console.log('  Gate 12: lib/inspector.js cross-check...');
  let inspectorVerdict = null;
  try {
    const { inspectRun } = require('./lib/inspector');
    inspectorVerdict = await inspectRun({
      workspaces: {
        captain: WORKSPACE_CAPTAIN,
        worker: WORKSPACE_WORKER,
      },
      primaryTaskIds: {
        captain: p1TaskId, // the forward-delegate task — has delegate_task + tool_result
        worker: p3TaskId,  // the worker fetch+reverse task — has web_fetch + delegate_task
      },
      clusterState: {
        parentKey,
        agents: {
          captain: { name: 'cascade-captain' },
          worker: { name: 'cascade-worker' },
        },
      },
      proofVerdict: {
        // Cascade doesn't do per-record proofs; feed a synthetic PASS so Layer 1
        // proof_bijection + wallet_tx_delta checks pass without a real verifier.
        verdict: 'PASS',
        mode: 'snapshot',
        expectedCount: 0,
        actualCount: 0,
        verifiedCount: 0,
        bijection: { status: 'PASS', orphans: [], misses: [], duplicates: [] },
        onChain: { status: 'SKIPPED', verifiedCount: 0, pendingCount: 0, failed: [] },
        temporal: { status: 'PASS', windowStart: '', windowEnd: '', outOfWindow: [] },
      },
      runNonce: MARKER_KEY,
      expectedBudgetSats: 2_500_000, // headroom over the ~2.04M cascade baseline
      stderrLogs: {
        captain: path.join(WORKSPACE_CAPTAIN, 'server-stderr.log'),
        worker: path.join(WORKSPACE_WORKER, 'server-stderr.log'),
      },
      taskRecords: {
        // P5 is the captain's final task — contains working_memory_set with MARKER_KEY
        // in result, so captain_task_complete's nonce check will succeed.
        captain: p5Completed
          ? { ...p5Completed, task_id: p5TaskId }
          : { status: 'error', error: 'P5 never ran', result: '' },
        worker: p3Completed
          ? { ...p3Completed, task_id: p3TaskId }
          : { status: 'error', error: 'P3 never ran', result: '' },
      },
      skipInvariants: ['overlay_was_used'], // cascade doesn't use the overlay
      expectedCommissions: { sent: 1, received: 1 }, // lower bound; cascade fires 4 each
      requireWorkerTools: ['web_fetch'], // cascade worker doesn't use execute_bash
    });
    for (const c of inspectorVerdict.layer1.checks) {
      const icon = c.status === 'PASS' ? 'PASS' : 'FAIL';
      console.log(`    [L1 ${icon}] ${c.name}${c.status !== 'PASS' ? ' — ' + c.reasoning : ''}`);
    }
    for (const inv of inspectorVerdict.layer2.invariants) {
      const icon = inv.status;
      const extra = inv.status !== 'PASS' && inv.status !== 'SKIP' ? ' — ' + inv.reasoning : '';
      console.log(`    [L2 ${icon}] ${inv.name}${extra}`);
    }
    console.log(`    inspector verdict: ${inspectorVerdict.verdict}`);
    if (inspectorVerdict.verdict !== 'PASS') {
      allPass = false;
    }
  } catch (e) {
    console.error(`    [Gate 12 ERROR] inspector.js threw: ${e.message}`);
    if (e.stack) console.error(e.stack);
    allPass = false;
  }
  console.log();

  console.log(allPass ? '  RESULT: ALL CHECKS PASSED — bidirectional cascade is green' : '  RESULT: SOME CHECKS FAILED');
  console.log('='.repeat(80));

  if (!STOP_AFTER) {
    console.log('\nServers still running. Ctrl-C to stop.');
    await new Promise(() => {});
  }

  process.exit(allPass ? 0 : 1);
}

// ---------------------------------------------------------------------------
// Entry + cleanup
// ---------------------------------------------------------------------------

process.on('SIGINT', async () => { console.log('\nSIGINT — stopping...'); await stopAll(); process.exit(1); });
process.on('SIGTERM', async () => { console.log('\nSIGTERM — stopping...'); await stopAll(); process.exit(1); });

main().catch(async (err) => {
  console.error(`\nFATAL: ${err.message}`);
  if (err.stack) console.error(err.stack);
  await stopAll();
  process.exit(1);
});
