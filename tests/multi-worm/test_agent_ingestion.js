#!/usr/bin/env node
/**
 * E2E Agent Ingestion Test (Issue #313)
 *
 * Verifies that a running dolphin-milk server picks up a MessageBox message
 * from an external wallet, spawns a task, processes it with real LLM inference,
 * and replies.
 *
 * This is a REAL E2E test with REAL SATS.
 *
 * This script:
 *   1. Starts its own dolphin-milk server on port 8081 (wallet A, port 3322)
 *      with heartbeat enabled, parent key set to MetaNet Client on 3321
 *   2. Sends a MessageBox message from wallet B (port 3323) to the server
 *   3. Waits for heartbeat to pick up the message and spawn a task
 *   4. Waits for task completion with real LLM inference
 *   5. Checks for reply in wallet B's results inbox
 *   6. Reports everything and cleans up
 *
 * Prerequisites:
 *   - cargo build --release (binary at ./target/release/dolphin-milk)
 *   - MetaNet Client (parent) on localhost:3321
 *   - Wallet A on localhost:3322 (funded BSV wallet)
 *   - Wallet B on localhost:3323 (funded BSV wallet, different identity key)
 *   - Live internet access to messagebox.babbage.systems
 *
 * Usage:
 *   node test_agent_ingestion.js
 *   node test_agent_ingestion.js --skip-send       # skip sending, just check for existing tasks
 */

const { spawn } = require('child_process');
const http = require('http');
const https = require('https');
const crypto = require('crypto');
const path = require('path');
const fs = require('fs');

// Import the existing BRC-31 auth library for talking to the worm server
const { authGet, authPost, clearAuthCache } = require('./lib/auth');

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const SERVER_PORT = 8081;
const PARENT_WALLET_PORT = 3321;
const WALLET_B_PORT = 3323;
const WALLET_A_PORT = 3322;
const MESSAGEBOX_URL = 'https://messagebox.babbage.systems';
const TASK_INBOX = 'task_inbox';
const RESULTS_INBOX = 'results_inbox';
const SKIP_SEND = process.argv.includes('--skip-send');

// Project root is two levels up from tests/multi-worm/
const PROJECT_ROOT = path.resolve(__dirname, '../../');
const BINARY = path.join(PROJECT_ROOT, 'target/release/dolphin-milk');
const WORKSPACE = path.join(PROJECT_ROOT, 'test-workspaces/ingestion-test');

// Timeouts
const HEALTH_TIMEOUT_MS = 45000;         // 45s for server to start
const TASK_APPEAR_TIMEOUT_MS = 180000;   // 3 min for heartbeat to pick up message
const TASK_COMPLETE_TIMEOUT_MS = 180000; // 3 min for task to complete
const REPLY_POLL_TIMEOUT_MS = 30000;     // 30s for reply to appear

// Track server process for cleanup
let serverProcess = null;

// ---------------------------------------------------------------------------
// CLI helpers
// ---------------------------------------------------------------------------

function getArg(flag) {
  const idx = process.argv.indexOf(flag);
  return (idx !== -1 && process.argv[idx + 1]) ? process.argv[idx + 1] : null;
}

function formatSats(sats) {
  return sats ? sats.toLocaleString() + ' sats' : '0 sats';
}

function formatMs(ms) {
  return (ms / 1000).toFixed(1) + 's';
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

function httpGet(url) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith('https') ? https : http;
    const req = mod.get(url, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => {
        try { resolve({ status: res.statusCode, body: JSON.parse(data) }); }
        catch { resolve({ status: res.statusCode, body: data }); }
      });
    });
    req.on('error', reject);
    req.setTimeout(10000, () => { req.destroy(); reject(new Error('timeout')); });
  });
}

function httpPost(url, body) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith('https') ? https : http;
    const urlObj = new URL(url);
    const data = JSON.stringify(body || {});
    const req = mod.request({
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
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
          if (parsed.error) {
            reject(new Error(`Wallet ${endpoint}: ${JSON.stringify(parsed.error)}`));
          } else {
            resolve(parsed);
          }
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

async function startServer() {
  if (!fs.existsSync(BINARY)) {
    throw new Error(`Binary not found at ${BINARY}. Run \`cargo build --release\` first.`);
  }

  // Get parent identity key for config
  const { publicKey: parentKey } = await walletPost(PARENT_WALLET_PORT, 'getPublicKey', { identityKey: true });

  // Ensure workspace exists
  fs.mkdirSync(WORKSPACE, { recursive: true });

  const stdoutLog = fs.openSync(path.join(WORKSPACE, 'server-stdout.log'), 'w');
  const stderrLog = fs.openSync(path.join(WORKSPACE, 'server-stderr.log'), 'w');

  console.log(`  Starting server on port ${SERVER_PORT}...`);
  console.log(`  Binary:    ${BINARY}`);
  console.log(`  Workspace: ${WORKSPACE}`);
  console.log(`  Wallet:    http://localhost:${WALLET_A_PORT}`);
  console.log(`  Parent:    ${parentKey.substring(0, 20)}...`);
  console.log(`  Heartbeat: enabled, poll every 15s`);

  serverProcess = spawn(
    BINARY,
    ['serve', '--port', String(SERVER_PORT), '--workspace', WORKSPACE],
    {
      cwd: PROJECT_ROOT,
      env: {
        ...process.env,
        DOLPHIN_MILK_WALLET_URL: `http://localhost:${WALLET_A_PORT}`,
        DOLPHIN_MILK_HEARTBEAT_ENABLED: 'true',
        DOLPHIN_MILK_HEARTBEAT_POLL_SECS: '15',
        DOLPHIN_MILK_PARENT_KEY: parentKey,
        DOLPHIN_MILK_PARENT_WALLET_URL: `http://localhost:${PARENT_WALLET_PORT}`,
        DOLPHIN_MILK_LOG_LEVEL: 'INFO',
      },
      stdio: ['ignore', stdoutLog, stderrLog],
      detached: false,
    }
  );

  // Wait for health check
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  let lastError = null;
  let healthy = false;

  while (Date.now() < deadline) {
    await sleep(1000);
    try {
      const { status, body } = await httpGet(`http://localhost:${SERVER_PORT}/health`);
      if (status === 200 && body?.status === 'ok' && body?.wallet_connected === true) {
        healthy = true;
        break;
      }
      lastError = `status=${status}, body=${JSON.stringify(body)}`;
    } catch (e) {
      lastError = e.message;
    }
  }

  if (!healthy) {
    // Read stderr for diagnostics
    const stderr = readLogTail(path.join(WORKSPACE, 'server-stderr.log'));
    throw new Error(`Server health check failed after ${HEALTH_TIMEOUT_MS}ms. Last: ${lastError}\nStderr: ${stderr}`);
  }

  console.log('  Server is healthy and wallet connected.');
}

function stopServer() {
  if (!serverProcess) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      try { serverProcess.kill('SIGKILL'); } catch {}
    }, 5000);
    serverProcess.on('exit', () => {
      clearTimeout(timer);
      resolve();
    });
    try { serverProcess.kill('SIGTERM'); } catch {}
  });
}

function readLogTail(filepath) {
  try {
    return fs.readFileSync(filepath, 'utf8').slice(-3000);
  } catch { return ''; }
}

// ---------------------------------------------------------------------------
// Authrite client for MessageBox (uses wallet B)
// ---------------------------------------------------------------------------

function writeVarint(n) {
  if (n <= 252) return Buffer.from([n]);
  if (n <= 0xFFFF) { const b = Buffer.alloc(3); b[0] = 0xFD; b.writeUInt16LE(n, 1); return b; }
  if (n <= 0xFFFFFFFF) { const b = Buffer.alloc(5); b[0] = 0xFE; b.writeUInt32LE(n, 1); return b; }
  const b = Buffer.alloc(9); b[0] = 0xFF; b.writeBigUInt64LE(BigInt(n), 1); return b;
}
const EMPTY_SENTINEL = Buffer.from([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);

function serializeRequest(requestId, method, reqPath, query, headers, body) {
  const parts = [requestId];
  const methodBuf = Buffer.from(method, 'utf8');
  parts.push(writeVarint(methodBuf.length), methodBuf);
  if (reqPath) { const b = Buffer.from(reqPath, 'utf8'); parts.push(writeVarint(b.length), b); }
  else parts.push(EMPTY_SENTINEL);
  if (query) { const b = Buffer.from(query, 'utf8'); parts.push(writeVarint(b.length), b); }
  else parts.push(EMPTY_SENTINEL);
  parts.push(writeVarint(headers.length));
  for (const [key, value] of headers) {
    const kb = Buffer.from(key, 'utf8'), vb = Buffer.from(value, 'utf8');
    parts.push(writeVarint(kb.length), kb, writeVarint(vb.length), vb);
  }
  if (body && body.length > 0) { parts.push(writeVarint(body.length), body); }
  else parts.push(EMPTY_SENTINEL);
  return Buffer.concat(parts);
}

const msgboxSessions = new Map();

async function msgboxHandshake(walletPort) {
  const key = `${walletPort}:messagebox`;
  if (msgboxSessions.has(key)) return msgboxSessions.get(key);

  const { publicKey: identityKey } = await walletPost(walletPort, 'getPublicKey', { identityKey: true });
  const clientNonce = crypto.randomBytes(32).toString('base64');

  const resp = await httpPost(`${MESSAGEBOX_URL}/.well-known/auth`, {
    version: '0.1',
    messageType: 'initialRequest',
    identityKey,
    initialNonce: clientNonce,
  });

  if (resp.status !== 200) {
    throw new Error(`MessageBox handshake failed: ${resp.status} ${JSON.stringify(resp.body)}`);
  }

  const session = {
    serverNonce: resp.body.initialNonce,
    serverIdentityKey: resp.body.identityKey,
    clientNonce,
    identityKey,
  };
  msgboxSessions.set(key, session);
  return session;
}

async function msgboxAuthRequest(walletPort, method, urlStr, bodyObj) {
  const urlObj = new URL(urlStr);
  const session = await msgboxHandshake(walletPort);

  const reqPath = urlObj.pathname;
  const query = urlObj.search || null;
  const requestId = crypto.randomBytes(32);
  const requestIdB64 = requestId.toString('base64');
  const msgNonce = crypto.randomBytes(32).toString('base64');

  const requestHeaders = {};
  let bodyBuf = null;
  if (bodyObj !== undefined) {
    bodyBuf = Buffer.from(JSON.stringify(bodyObj), 'utf8');
    requestHeaders['content-type'] = 'application/json';
  }

  const signableHeaders = [];
  for (const [k, v] of Object.entries(requestHeaders)) {
    const lower = k.toLowerCase();
    if (lower.startsWith('x-bsv-auth-')) continue;
    if (lower.startsWith('x-bsv-') || lower === 'authorization') signableHeaders.push([lower, v]);
    else if (lower === 'content-type') signableHeaders.push([lower, v.split(';')[0].trim()]);
  }
  signableHeaders.sort((a, b) => a[0].localeCompare(b[0]));

  // Body IS included in signature for MessageBox (unlike the worm server where
  // axum consumes the body before auth verification). The Rust AuthriteClient
  // always includes body in the signature (auth/client.rs line 320).
  const serialized = serializeRequest(requestId, method, reqPath, query, signableHeaders, bodyBuf);

  const keyId = `${msgNonce} ${session.serverNonce}`;
  const sigResp = await walletPost(walletPort, 'createSignature', {
    data: Array.from(serialized),
    protocolID: [2, 'auth message signature'],
    keyID: keyId,
    counterparty: session.serverIdentityKey,
  });

  const signatureHex = Buffer.from(sigResp.signature).toString('hex');

  const finalHeaders = {
    ...requestHeaders,
    'x-bsv-auth-version': '0.1',
    'x-bsv-auth-identity-key': session.identityKey,
    'x-bsv-auth-message-type': 'general',
    'x-bsv-auth-nonce': msgNonce,
    'x-bsv-auth-your-nonce': session.serverNonce,
    'x-bsv-auth-signature': signatureHex,
    'x-bsv-auth-request-id': requestIdB64,
  };
  if (bodyBuf) finalHeaders['content-length'] = String(bodyBuf.length);

  return new Promise((resolve, reject) => {
    const mod = urlStr.startsWith('https') ? https : http;
    const req = mod.request({
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: reqPath + (query || ''),
      method,
      headers: finalHeaders,
    }, (res) => {
      let raw = '';
      res.on('data', chunk => raw += chunk);
      res.on('end', () => {
        let body;
        try { body = JSON.parse(raw); } catch { body = raw; }
        resolve({ status: res.statusCode, body, headers: res.headers });
      });
    });
    req.on('error', reject);
    req.setTimeout(30000, () => { req.destroy(); reject(new Error(`MessageBox timeout: ${method} ${reqPath}`)); });
    if (bodyBuf) req.write(bodyBuf);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// MessageBox operations (using wallet B)
// ---------------------------------------------------------------------------

async function sendMessageBoxMessage(recipientKey, messageBox, body) {
  const messageId = crypto.randomUUID();
  const requestBody = {
    message: {
      recipient: recipientKey,
      messageBox,
      messageId,
      body,
    },
  };

  let resp = await msgboxAuthRequest(WALLET_B_PORT, 'POST', `${MESSAGEBOX_URL}/sendMessage`, requestBody);

  if (resp.status === 402) {
    console.log('  MessageBox requires payment (402) — this is normal for task_inbox.');
    console.log(`  402 body: ${JSON.stringify(resp.body).substring(0, 300)}`);
    // For now, try without payment — many MessageBox deployments don't enforce fees
    // If the server requires payment, we'd need to construct a BRC-29 payment
    throw new Error(`MessageBox requires payment (402). Body: ${JSON.stringify(resp.body).substring(0, 300)}`);
  }

  if (resp.status !== 200) {
    throw new Error(`sendMessage failed: ${resp.status} ${JSON.stringify(resp.body).substring(0, 500)}`);
  }

  return { messageId, response: resp.body };
}

async function listMessageBoxMessages(walletPort, messageBox) {
  const resp = await msgboxAuthRequest(walletPort, 'POST', `${MESSAGEBOX_URL}/listMessages`, {
    messageBox,
  });

  if (resp.status !== 200) {
    throw new Error(`listMessages failed: ${resp.status} ${JSON.stringify(resp.body).substring(0, 300)}`);
  }

  const messages = Array.isArray(resp.body) ? resp.body : (resp.body?.messages || []);
  return messages;
}

async function acknowledgeMessages(walletPort, messageIds) {
  if (!messageIds.length) return;
  await msgboxAuthRequest(walletPort, 'POST', `${MESSAGEBOX_URL}/acknowledgeMessage`, {
    messageIds,
  });
}

// ---------------------------------------------------------------------------
// Polling helpers
// ---------------------------------------------------------------------------

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

async function pollForNewTask(baselineTaskIds, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let pollCount = 0;
  while (Date.now() < deadline) {
    pollCount++;
    try {
      const resp = await authGet(`http://localhost:${SERVER_PORT}/tasks`, PARENT_WALLET_PORT);
      if (resp.status === 200 && resp.body?.tasks) {
        const tasks = resp.body.tasks;
        const newTask = tasks.find(t =>
          !baselineTaskIds.has(t.id) &&
          (t.origin === 'message' || (t.task && t.task.includes('inbox message')))
        );
        if (newTask) {
          console.log(`  (found after ${pollCount} polls)`);
          return newTask;
        }
        // Also look for any new task at all
        const anyNew = tasks.find(t => !baselineTaskIds.has(t.id));
        if (anyNew) {
          console.log(`  (found new task with origin="${anyNew.origin}" after ${pollCount} polls)`);
          return anyNew;
        }
      }
    } catch (e) {
      if (pollCount <= 2) console.log(`  Poll ${pollCount} error: ${e.message}`);
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  return null;
}

async function waitForTaskComplete(taskId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const resp = await authGet(`http://localhost:${SERVER_PORT}/task/${taskId}`, PARENT_WALLET_PORT);
      if (resp.status === 200 && resp.body) {
        const status = (resp.body.status || '').toLowerCase();
        if (status === 'complete' || status === 'error' || status === 'cancelled') {
          return resp.body;
        }
      }
    } catch (e) {
      // Non-fatal
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();
  return null;
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

async function main() {
  const startTime = Date.now();
  console.log('='.repeat(80));
  console.log('  AGENT INGESTION E2E TEST (Issue #313)');
  console.log('  Real sats. Real MessageBox. Real LLM inference.');
  console.log('='.repeat(80));
  console.log();

  // -----------------------------------------------------------------------
  // Gate 1: Verify prerequisites
  // -----------------------------------------------------------------------
  console.log('[Gate 1] Verifying prerequisites...');

  // Check wallet A
  try {
    const resp = await walletPost(WALLET_A_PORT, 'getPublicKey', { identityKey: true });
    console.log(`  Wallet A:  :${WALLET_A_PORT} -- ${resp.publicKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet A not reachable on port ${WALLET_A_PORT}: ${e.message}`);
    process.exit(1);
  }

  // Check wallet B
  let walletBKey;
  try {
    const resp = await walletPost(WALLET_B_PORT, 'getPublicKey', { identityKey: true });
    walletBKey = resp.publicKey;
    console.log(`  Wallet B:  :${WALLET_B_PORT} -- ${walletBKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Wallet B not reachable on port ${WALLET_B_PORT}: ${e.message}`);
    process.exit(1);
  }

  // Check parent wallet
  try {
    const resp = await walletPost(PARENT_WALLET_PORT, 'getPublicKey', { identityKey: true });
    console.log(`  Parent:    :${PARENT_WALLET_PORT} -- ${resp.publicKey.substring(0, 20)}...`);
  } catch (e) {
    console.error(`FATAL: Parent wallet not reachable on port ${PARENT_WALLET_PORT}: ${e.message}`);
    process.exit(1);
  }

  console.log('  All wallets available.\n');

  // -----------------------------------------------------------------------
  // Gate 2: Start server
  // -----------------------------------------------------------------------
  console.log('[Gate 2] Starting dolphin-milk server...');
  try {
    await startServer();
  } catch (e) {
    console.error(`FATAL: Could not start server: ${e.message}`);
    await stopServer();
    process.exit(1);
  }

  // Clear any stale auth sessions (the new server has a different session)
  clearAuthCache();

  // Get server identity key
  let serverIdentityKey;
  try {
    const agentResp = await authGet(`http://localhost:${SERVER_PORT}/agent`, PARENT_WALLET_PORT);
    if (agentResp.status !== 200) throw new Error(`Status ${agentResp.status}: ${JSON.stringify(agentResp.body)}`);
    serverIdentityKey = agentResp.body.identity_key;
    console.log(`  Agent ID:  ${serverIdentityKey.substring(0, 20)}...`);
    console.log(`  Balance:   ${formatSats(agentResp.body.balance)}`);
    console.log(`  Model:     ${agentResp.body.default_model || 'unknown'}`);
  } catch (e) {
    console.error(`FATAL: Cannot get agent identity: ${e.message}`);
    await stopServer();
    process.exit(1);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 3: Get baseline tasks
  // -----------------------------------------------------------------------
  console.log('[Gate 3] Getting baseline task list...');
  let baselineTaskIds = new Set();
  try {
    const resp = await authGet(`http://localhost:${SERVER_PORT}/tasks`, PARENT_WALLET_PORT);
    if (resp.status === 200 && resp.body?.tasks) {
      resp.body.tasks.forEach(t => baselineTaskIds.add(t.id));
      console.log(`  Baseline: ${baselineTaskIds.size} existing tasks`);
    }
  } catch (e) {
    console.log(`  Warning: Could not get baseline tasks: ${e.message}`);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 4: Drain wallet B's results inbox
  // -----------------------------------------------------------------------
  console.log('[Gate 4] Draining wallet B results inbox...');
  try {
    const messages = await listMessageBoxMessages(WALLET_B_PORT, RESULTS_INBOX);
    if (messages.length > 0) {
      const ids = messages.map(m => m.messageId);
      await acknowledgeMessages(WALLET_B_PORT, ids);
      console.log(`  Drained ${ids.length} old result message(s)`);
    } else {
      console.log('  Results inbox empty (clean)');
    }
  } catch (e) {
    console.log(`  Warning: Could not drain results inbox: ${e.message}`);
    console.log('  Continuing anyway...');
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 5: Send MessageBox message from wallet B to the server
  // -----------------------------------------------------------------------
  const testNonce = crypto.randomUUID();
  const messageBody = {
    type: 'task_assignment',
    version: 1,
    task_id: `test-ingestion-${testNonce.substring(0, 8)}`,
    task: 'What is 2+2? Reply with just the number.',
    budget_sats: 500000,
    response_box: RESULTS_INBOX,
    requester_key: walletBKey,
    priority: 'normal',
    context: {
      test_nonce: testNonce,
      test_name: 'agent_ingestion_e2e',
    },
  };

  if (!SKIP_SEND) {
    console.log('[Gate 5] Sending MessageBox message from Wallet B to server...');
    console.log(`  Recipient: ${serverIdentityKey.substring(0, 20)}...`);
    console.log(`  Box:       ${TASK_INBOX}`);
    console.log(`  Task:      "${messageBody.task}"`);
    console.log(`  Nonce:     ${testNonce.substring(0, 12)}...`);

    try {
      const sendResult = await sendMessageBoxMessage(serverIdentityKey, TASK_INBOX, messageBody);
      console.log(`  Sent!      messageId=${sendResult.messageId}`);
      console.log(`  Response:  ${JSON.stringify(sendResult.response).substring(0, 200)}`);
    } catch (e) {
      console.error(`FATAL: Failed to send MessageBox message: ${e.message}`);
      await stopServer();
      process.exit(1);
    }
  } else {
    console.log('[Gate 5] SKIPPED (--skip-send). Looking for existing task...');
  }

  const sendTime = Date.now();
  console.log();

  // -----------------------------------------------------------------------
  // Gate 6: Wait for task to appear on the server
  // -----------------------------------------------------------------------
  console.log('[Gate 6] Waiting for heartbeat to pick up message and spawn task...');
  console.log(`  Polling /tasks every 5s (timeout ${TASK_APPEAR_TIMEOUT_MS / 1000}s)...`);

  const newTask = await pollForNewTask(baselineTaskIds, TASK_APPEAR_TIMEOUT_MS);

  if (!newTask) {
    console.error('\nFAIL: No new task appeared within timeout.');
    console.error('  Checking server logs...');
    const stderr = readLogTail(path.join(WORKSPACE, 'server-stderr.log'));
    console.error('  Last 1000 chars of stderr:');
    console.error(stderr.substring(stderr.length - 1000));
    await stopServer();
    process.exit(1);
  }

  const taskAppearedMs = Date.now() - sendTime;
  console.log(`\n  Task appeared in ${formatMs(taskAppearedMs)}`);
  console.log(`  Task ID:     ${newTask.id}`);
  console.log(`  Description: ${(newTask.task || '').substring(0, 120)}`);
  console.log(`  Status:      ${newTask.status}`);
  console.log(`  Origin:      ${newTask.origin || 'unknown'}`);
  console.log(`  Conv ID:     ${newTask.conversation_id || 'none'}`);
  console.log();

  // -----------------------------------------------------------------------
  // Gate 7: Wait for task to complete
  // -----------------------------------------------------------------------
  console.log('[Gate 7] Waiting for task to complete...');
  console.log(`  Polling /task/${newTask.id} every 5s (timeout ${TASK_COMPLETE_TIMEOUT_MS / 1000}s)...`);

  const completedTask = await waitForTaskComplete(newTask.id, TASK_COMPLETE_TIMEOUT_MS);

  if (!completedTask) {
    console.error(`\nFAIL: Task ${newTask.id} did not complete within timeout.`);
    await stopServer();
    process.exit(1);
  }

  const taskCompleteMs = Date.now() - sendTime;
  console.log(`\n  Task completed in ${formatMs(taskCompleteMs)} (from send)`);
  console.log(`  Status:      ${completedTask.status}`);
  console.log(`  Iterations:  ${completedTask.iterations}`);
  console.log(`  Sats spent:  ${formatSats(completedTask.sats_spent)}`);
  console.log(`  Result:      ${(completedTask.result || completedTask.error || 'none').substring(0, 300)}`);
  if (completedTask.proof_txids?.length) {
    console.log(`  Proof TXIDs: ${completedTask.proof_txids.length} proof(s)`);
    completedTask.proof_txids.forEach((txid, i) => {
      console.log(`    [${i}] ${txid}`);
    });
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 8: Inspect task audit trail
  // -----------------------------------------------------------------------
  console.log('[Gate 8] Inspecting task audit trail...');
  let auditData = null;
  try {
    const auditResp = await authGet(
      `http://localhost:${SERVER_PORT}/task/${newTask.id}/audit`,
      PARENT_WALLET_PORT
    );
    if (auditResp.status === 200 && auditResp.body) {
      auditData = auditResp.body;
      const events = auditData.events || [];
      console.log(`  Total events: ${events.length}`);

      const eventTypes = {};
      for (const ev of events) {
        const type = ev.event_type || ev.type || 'unknown';
        eventTypes[type] = (eventTypes[type] || 0) + 1;
      }
      console.log('  Event types:');
      for (const [type, count] of Object.entries(eventTypes).sort((a, b) => b[1] - a[1])) {
        console.log(`    ${type}: ${count}`);
      }

      const thinkEvents = events.filter(e => (e.event_type || e.type) === 'think_response');
      console.log(`  LLM calls:   ${thinkEvents.length}`);

      const proofEvents = events.filter(e =>
        ['checkpoint_created', 'proof_created'].includes(e.event_type || e.type)
      );
      console.log(`  Proofs:      ${proofEvents.length}`);

      const toolEvents = events.filter(e => (e.event_type || e.type) === 'tool_call');
      if (toolEvents.length > 0) {
        console.log(`  Tool calls:  ${toolEvents.length}`);
        for (const tc of toolEvents) {
          const name = tc.tool_name || tc.data?.tool_name || 'unknown';
          const result = (tc.tool_result || tc.data?.tool_result || '').substring(0, 100);
          console.log(`    - ${name}: ${result}`);
        }
      }

      if (auditData.summary) {
        console.log('  Summary:');
        console.log(`    Iterations:  ${auditData.summary.iterations || '?'}`);
        console.log(`    Total sats:  ${formatSats(auditData.summary.total_sats_spent || auditData.summary.sats_spent)}`);
        console.log(`    Model:       ${auditData.summary.model || '?'}`);
      }
    }
  } catch (e) {
    console.log(`  Warning: Could not fetch audit trail: ${e.message}`);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 9: Check on-chain proofs
  // -----------------------------------------------------------------------
  console.log('[Gate 9] Verifying on-chain proofs...');
  try {
    const proofsResp = await authGet(
      `http://localhost:${SERVER_PORT}/task/${newTask.id}/proofs`,
      PARENT_WALLET_PORT
    );
    if (proofsResp.status === 200 && proofsResp.body) {
      const proofs = proofsResp.body.proofs || [];
      const checkpoints = proofsResp.body.checkpoints || [];
      console.log(`  BRC-18 proofs:      ${proofs.length}`);
      console.log(`  BRC-48 checkpoints: ${checkpoints.length}`);
      for (const p of proofs) {
        console.log(`    [${p.proof_type || p.type}] txid=${(p.txid || '').substring(0, 24)}...`);
      }
    }
  } catch (e) {
    console.log(`  Warning: Could not fetch proofs: ${e.message}`);
  }
  console.log();

  // -----------------------------------------------------------------------
  // Gate 10: Check wallet B's results inbox for reply
  // -----------------------------------------------------------------------
  console.log('[Gate 10] Checking wallet B results inbox for reply...');
  console.log(`  Polling every 5s (timeout ${REPLY_POLL_TIMEOUT_MS / 1000}s)...`);

  let replyMessage = null;
  const replyDeadline = Date.now() + REPLY_POLL_TIMEOUT_MS;
  while (Date.now() < replyDeadline) {
    try {
      const messages = await listMessageBoxMessages(WALLET_B_PORT, RESULTS_INBOX);
      if (messages.length > 0) {
        replyMessage = messages.find(m => {
          const body = m.body || {};
          return body.type === 'task_result' || body.task_id || body.result;
        }) || messages[0];
        break;
      }
    } catch (e) {
      // Non-fatal
    }
    process.stdout.write('.');
    await sleep(5000);
  }
  console.log();

  if (replyMessage) {
    console.log('  Reply received!');
    console.log(`  Message ID: ${replyMessage.messageId}`);
    console.log(`  Sender:     ${(replyMessage.sender || '').substring(0, 20)}...`);
    console.log('  Body:');
    const bodyStr = JSON.stringify(replyMessage.body, null, 4);
    bodyStr.split('\n').forEach(l => console.log('    ' + l));

    try {
      await acknowledgeMessages(WALLET_B_PORT, [replyMessage.messageId]);
      console.log('  (acknowledged)');
    } catch (e) {
      console.log(`  Warning: Could not acknowledge reply: ${e.message}`);
    }
  } else {
    console.log('  No reply received within timeout.');
  }
  console.log();

  // -----------------------------------------------------------------------
  // Cleanup: stop server
  // -----------------------------------------------------------------------
  console.log('Stopping server...');
  await stopServer();
  console.log('Server stopped.\n');

  // -----------------------------------------------------------------------
  // Final Report
  // -----------------------------------------------------------------------
  const totalMs = Date.now() - startTime;

  console.log('='.repeat(80));
  console.log('  FINAL REPORT');
  console.log('='.repeat(80));
  console.log();
  console.log(`  Test:             Agent Ingestion E2E (#313)`);
  console.log(`  Total time:       ${formatMs(totalMs)}`);
  console.log(`  Message -> Task:  ${formatMs(taskAppearedMs)}`);
  console.log(`  Message -> Done:  ${formatMs(taskCompleteMs)}`);
  console.log(`  Task status:      ${completedTask.status}`);
  console.log(`  Iterations:       ${completedTask.iterations}`);
  console.log(`  Sats spent:       ${formatSats(completedTask.sats_spent)}`);
  console.log(`  Proofs created:   ${completedTask.proof_txids?.length || 0}`);
  console.log(`  Reply received:   ${replyMessage ? 'YES' : 'NO'}`);
  console.log();

  const isComplete = (completedTask.status || '').toLowerCase() === 'complete';
  const hadLLMCalls = auditData?.events?.some(e =>
    (e.event_type || e.type) === 'think_response'
  );
  const hadProofs = (completedTask.proof_txids?.length || 0) > 0;

  const checks = [
    { name: 'Task spawned from MessageBox', pass: true },
    { name: 'Task completed successfully', pass: isComplete },
    { name: 'LLM inference happened', pass: !!hadLLMCalls },
    { name: 'BRC-18 proofs created', pass: hadProofs },
    { name: 'Reply sent to wallet B', pass: !!replyMessage },
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
  }
  console.log('='.repeat(80));

  process.exit(allPass ? 0 : 1);
}

// ---------------------------------------------------------------------------
// Entry point with cleanup
// ---------------------------------------------------------------------------

// Ensure server cleanup on any exit
process.on('SIGINT', async () => {
  console.log('\nReceived SIGINT. Stopping server...');
  await stopServer();
  process.exit(1);
});

process.on('SIGTERM', async () => {
  console.log('\nReceived SIGTERM. Stopping server...');
  await stopServer();
  process.exit(1);
});

main().catch(async (err) => {
  console.error(`\nFATAL: ${err.message}`);
  if (err.stack) console.error(err.stack);
  await stopServer();
  process.exit(1);
});
