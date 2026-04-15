// tests/multi-worm/lib/inspector.js
//
// Layered verdict inspector for the DolphinMilkShake #23 quality gate.
//
// Layer 1 (hard outcome checks) and Layer 2 (structural invariants) are
// implemented here. Layer 3 (rich expected-chain DSL) is explicitly
// deferred — it returns { status: 'DEFERRED' } so the verdict object
// still contains the field for downstream consumers.
//
// See tests/multi-worm/lib/CONTRACTS.md (section "lib/inspector.js") for
// the interface contract this file implements.
//
// Calibration note: every Layer 2 invariant below has been verified
// against the real green-path cascade transcripts at
//   test-workspaces/cascade-captain/tasks/d2880303-.../session.jsonl
//   test-workspaces/cascade-worker/tasks/06198a62-.../session.jsonl
//   test-workspaces/cascade-captain/tasks/f55ae493-.../session.jsonl
// Each predicate includes the real event JSON (truncated) as a comment
// above the implementation, so future maintainers can see what data
// the predicate expects.

'use strict';

const fs = require('fs');
const path = require('path');

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Parse a session.jsonl file. Each line is a JSON event. Skips blank lines
 * and records parse errors instead of throwing.
 *
 * Real transcript lines look like:
 *   {"ts":1776095501.703867,"type":"think_response",...,"sats_effective":116924,...}
 *   {"ts":1776095501.704013,"type":"tool_call","id":"27a5bcde","arguments":{...},"name":"web_fetch","call_id":"toolu_..."}
 *
 * @param {string} filePath absolute path to session.jsonl
 * @returns {Promise<{events: object[], parseErrors: Array<{line:number,error:string}>, path: string}>}
 */
async function readSessionJsonl(filePath) {
  if (!fs.existsSync(filePath)) {
    throw new Error(`[inspector] session.jsonl not found: ${filePath}`);
  }
  const raw = fs.readFileSync(filePath, 'utf8');
  const lines = raw.split('\n');
  const events = [];
  const parseErrors = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line || line.trim() === '') continue;
    try {
      events.push(JSON.parse(line));
    } catch (err) {
      parseErrors.push({ line: i + 1, error: err.message });
    }
  }
  return { events, parseErrors, path: filePath };
}

/**
 * Filter events by a predicate. Pure function.
 */
function findEvents(events, predicate) {
  if (!Array.isArray(events)) return [];
  return events.filter((e) => {
    try {
      return !!predicate(e);
    } catch (_err) {
      return false;
    }
  });
}

/**
 * Load every session.jsonl under `${workspaceRoot}/tasks/*`.
 * Returns one { taskId, path, events, parseErrors } per transcript.
 */
async function loadWorkspaceTranscripts(workspaceRoot) {
  const tasksDir = path.join(workspaceRoot, 'tasks');
  if (!fs.existsSync(tasksDir)) {
    return [];
  }
  const taskIds = fs
    .readdirSync(tasksDir, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name);
  const results = [];
  for (const taskId of taskIds) {
    const p = path.join(tasksDir, taskId, 'session.jsonl');
    if (!fs.existsSync(p)) continue;
    const parsed = await readSessionJsonl(p);
    results.push({ taskId, ...parsed });
  }
  return results;
}

/**
 * Pick a single "primary" transcript per agent. Strategy:
 *   1. If input.taskRecords[agentName].task_id is present, match by that.
 *   2. Else, pick the transcript with the largest max(ts) (most recent).
 */
function pickPrimaryTranscript(transcripts, preferredTaskId) {
  if (!transcripts || transcripts.length === 0) return null;
  if (preferredTaskId) {
    const hit = transcripts.find((t) => t.taskId === preferredTaskId);
    if (hit) return hit;
  }
  let best = null;
  let bestTs = -Infinity;
  for (const t of transcripts) {
    const maxTs = t.events.reduce(
      (acc, e) => (typeof e.ts === 'number' && e.ts > acc ? e.ts : acc),
      -Infinity,
    );
    if (maxTs > bestTs) {
      bestTs = maxTs;
      best = t;
    }
  }
  return best || transcripts[0];
}

// ---------------------------------------------------------------------------
// Layer 1 — hard outcome checks
// ---------------------------------------------------------------------------

function checkProofBijection(input) {
  const pv = input.proofVerdict || {};
  if (pv.verdict === 'PASS') {
    return {
      name: 'proof_bijection',
      status: 'PASS',
      evidence: { bijection: pv.bijection, onChain: pv.onChain },
      reasoning: 'proofVerdict.verdict === PASS',
    };
  }
  return {
    name: 'proof_bijection',
    status: 'FAIL',
    evidence: { bijection: pv.bijection, onChain: pv.onChain, verdict: pv.verdict },
    reasoning: `proofVerdict.verdict === ${pv.verdict || '<missing>'}`,
  };
}

function checkWalletTxDelta(input) {
  const pv = input.proofVerdict || {};
  const { expectedCount, actualCount, verifiedCount } = pv;
  const ok =
    typeof expectedCount === 'number' &&
    expectedCount === actualCount &&
    expectedCount === verifiedCount;
  return {
    name: 'wallet_tx_delta',
    status: ok ? 'PASS' : 'FAIL',
    evidence: { expectedCount, actualCount, verifiedCount },
    reasoning: ok
      ? 'expected === actual === verified (no off-by-one)'
      : `expected=${expectedCount} actual=${actualCount} verified=${verifiedCount}`,
  };
}

function checkCaptainTaskComplete(input) {
  const rec = (input.taskRecords && input.taskRecords.captain) || {};
  const nonce = input.runNonce || '';
  const status = rec.status;
  const err = rec.error;
  const resultStr = String(rec.result || '');
  const statusOk = status === 'complete';
  const errOk = err === null || err === undefined || err === '';
  const nonceOk = nonce.length > 0 && resultStr.includes(nonce);
  const ok = statusOk && errOk && nonceOk;
  return {
    name: 'captain_task_complete',
    status: ok ? 'PASS' : 'FAIL',
    evidence: {
      task_status: status,
      error: err,
      result_len: resultStr.length,
      nonce: nonce,
      nonce_found: nonceOk,
    },
    reasoning: ok
      ? "captain.status=complete, no error, runNonce present in result"
      : `statusOk=${statusOk} errOk=${errOk} nonceOk=${nonceOk}`,
  };
}

// Real log strings (from cascade-captain/server-stderr.log):
//   "message":"commission payment broadcast"            (sent side)
//   "message":"commission_payment_sent internalized"    (sender internalizing own tx)
//   "message":"commission payment internalized"         (receive side, claim path)
//   "message":"commission_payment_claim handled and paid" (issuer pays claim)
// Regexes are intentionally broad — "payment broadcast" proves the
// sending half, "internalized" / "handled and paid" proves the receiving half.
const SENT_REGEX = /"message":"commission[ _]payment[ _]broadcast"|"message":"commission_payment_sent internalized"/;
const RECV_REGEX = /"message":"commission[ _]payment[ _]internalized"|"message":"commission_payment_claim handled and paid"/;

function countCommissionMessages(stderrPath) {
  if (!stderrPath || !fs.existsSync(stderrPath)) {
    return { sent: 0, received: 0, readable: false };
  }
  const raw = fs.readFileSync(stderrPath, 'utf8');
  const lines = raw.split('\n');
  let sent = 0;
  let received = 0;
  for (const line of lines) {
    if (SENT_REGEX.test(line)) sent++;
    if (RECV_REGEX.test(line)) received++;
  }
  return { sent, received, readable: true };
}

function checkCommissionSettled(input) {
  const expected = input.expectedCommissions || { sent: 1, received: 1 };
  const logs = input.stderrLogs || {};
  const perAgent = {};
  let totalSent = 0;
  let totalReceived = 0;
  for (const [name, p] of Object.entries(logs)) {
    const counts = countCommissionMessages(p);
    perAgent[name] = counts;
    totalSent += counts.sent;
    totalReceived += counts.received;
  }
  const ok = totalSent >= expected.sent && totalReceived >= expected.received;
  return {
    name: 'commission_settled',
    status: ok ? 'PASS' : 'FAIL',
    evidence: { perAgent, totalSent, totalReceived, expected },
    reasoning: ok
      ? `commissions settled: sent=${totalSent} (need ${expected.sent}), received=${totalReceived} (need ${expected.received})`
      : `commissions under threshold: sent=${totalSent}/${expected.sent}, received=${totalReceived}/${expected.received}`,
  };
}

/**
 * Sum sats_effective across a map of { agent: [transcript, ...] }.
 * Pass a list per agent (one or more transcripts); the function walks
 * events of type === 'think_response' and totals `sats_effective`.
 *
 * Real think_response event:
 *   {"type":"think_response","sats_effective":116924,"duration_ms":10771,
 *    "sats_refunded":643901,"content":"...","model":"claude-haiku-4-5-...",
 *    "prompt_tokens":...,"completion_tokens":...,"tool_calls":[...]}
 */
function sumThinkResponseSats(transcriptsByAgent) {
  let total = 0;
  const perAgent = {};
  for (const [agent, list] of Object.entries(transcriptsByAgent)) {
    const arr = Array.isArray(list) ? list : list ? [list] : [];
    let agentSum = 0;
    for (const t of arr) {
      if (!t || !Array.isArray(t.events)) continue;
      for (const e of t.events) {
        if (e && e.type === 'think_response' && typeof e.sats_effective === 'number') {
          agentSum += e.sats_effective;
        }
      }
    }
    perAgent[agent] = agentSum;
    total += agentSum;
  }
  return { total, perAgent };
}

function checkBudgetRespected(input, transcriptsByAgent) {
  const limit = Math.floor((input.expectedBudgetSats || 0) * 1.3);
  const { total, perAgent } = sumThinkResponseSats(transcriptsByAgent);
  const ok = limit === 0 ? true : total <= limit;
  return {
    name: 'budget_respected',
    status: ok ? 'PASS' : 'FAIL',
    evidence: { total_sats_effective: total, perAgent, expected_budget: input.expectedBudgetSats, upper_limit: limit },
    reasoning: ok
      ? `sum(sats_effective)=${total} <= ${limit} (1.3 * ${input.expectedBudgetSats})`
      : `sum(sats_effective)=${total} exceeds upper limit ${limit}`,
  };
}

// ---------------------------------------------------------------------------
// Layer 2 — structural invariants (each wrapped in try/catch)
// ---------------------------------------------------------------------------

function safeInv(name, fn) {
  try {
    return fn();
  } catch (err) {
    return {
      name,
      status: 'FAIL',
      evidence: { error: err && err.message },
      reasoning: `predicate threw: ${err && err.message}`,
      _predicate_threw: true,
    };
  }
}

function isSkipped(name, skipList) {
  return Array.isArray(skipList) && skipList.includes(name);
}

function skipInv(name, reason) {
  return {
    name,
    status: 'SKIP',
    evidence: {},
    reasoning: reason || 'skipped via input.skipInvariants',
  };
}

// Invariant 1 — overlay_was_used
//
// Expected shape (POC #23):
//   {"type":"tool_call","name":"overlay_lookup","arguments":{"query":"scraping", ...}, ...}
// The cascade test does NOT use overlay_lookup, so calibration passes
// this invariant in `skipInvariants`. The POC test will NOT skip.
function invOverlayWasUsed(captainEvents) {
  const hits = findEvents(
    captainEvents,
    (e) => e && e.type === 'tool_call' && e.name === 'overlay_lookup',
  );
  if (hits.length === 0) {
    return {
      name: 'overlay_was_used',
      status: 'FAIL',
      evidence: { tool_call_overlay_lookup_count: 0 },
      reasoning: 'Captain transcript has no tool_call with name=overlay_lookup',
    };
  }
  const matching = hits.filter((e) => {
    const asStr = JSON.stringify(e.arguments || {}).toLowerCase();
    return asStr.includes('scraping');
  });
  if (matching.length === 0) {
    return {
      name: 'overlay_was_used',
      status: 'FAIL',
      evidence: { overlay_lookup_count: hits.length, with_scraping: 0 },
      reasoning: 'overlay_lookup called but none with "scraping" in args',
    };
  }
  return {
    name: 'overlay_was_used',
    status: 'PASS',
    evidence: {
      overlay_lookup_count: hits.length,
      with_scraping: matching.length,
      first_match: { ts: matching[0].ts, arguments: matching[0].arguments },
    },
    reasoning: `overlay_lookup called ${hits.length}x, ${matching.length} with "scraping"`,
  };
}

// Invariant 2 — delegation_happened
//
// Captain tool_call (real event from cascade-captain forward task):
//   {"type":"tool_call","name":"delegate_task","arguments":{"recipient":"0350cf02ff54...","capabilities":[...],"budget_cap_sats":500000,"expires_in_secs":900,"task":"..."},"call_id":"toolu_..."}
// Worker inbound event (from worker transcript):
//   {"type":"delegation_verified","root_certifier":"03ef3231...","chain_depth":1,"capabilities":["delegate_task","send_message","web_fetch"],"budget_cap_sats":500000}
//   followed by {"type":"session_start","task":"=== WORKER ROLE ..."}
// We treat the worker side as "has a delegation_verified event OR the
// initial user event starting with the delegated-task banner".
function invDelegationHappened(captainEvents, workerEvents) {
  const captainCalls = findEvents(
    captainEvents,
    (e) => e && e.type === 'tool_call' && e.name === 'delegate_task',
  );
  if (captainCalls.length === 0) {
    return {
      name: 'delegation_happened',
      status: 'FAIL',
      evidence: { captain_delegate_task_count: 0 },
      reasoning: 'Captain transcript has no tool_call with name=delegate_task',
    };
  }
  const workerInbound = findEvents(
    workerEvents,
    (e) =>
      e &&
      (e.type === 'delegation_verified' ||
        (e.type === 'session_start' && typeof e.task === 'string') ||
        (e.type === 'user' &&
          typeof e.content === 'string' &&
          /delegated task|WORKER ROLE|authorized delegated/i.test(e.content))),
  );
  if (workerInbound.length === 0) {
    return {
      name: 'delegation_happened',
      status: 'FAIL',
      evidence: {
        captain_delegate_task_count: captainCalls.length,
        worker_inbound_count: 0,
      },
      reasoning: 'Worker has no delegation_verified / session_start / delegated user event',
    };
  }
  // Timestamp sanity: earliest captain call ≤ latest worker inbound + 60s.
  const captainTs = Math.min(
    ...captainCalls.map((e) => (typeof e.ts === 'number' ? e.ts : Infinity)),
  );
  const workerTs = Math.max(
    ...workerInbound.map((e) => (typeof e.ts === 'number' ? e.ts : -Infinity)),
  );
  const tsOk = captainTs <= workerTs + 60;
  return {
    name: 'delegation_happened',
    status: tsOk ? 'PASS' : 'FAIL',
    evidence: {
      captain_delegate_task_count: captainCalls.length,
      worker_inbound_count: workerInbound.length,
      captain_earliest_ts: captainTs,
      worker_latest_inbound_ts: workerTs,
    },
    reasoning: tsOk
      ? `captain delegated ${captainCalls.length}x, worker saw ${workerInbound.length} inbound events`
      : `timestamp ordering broken: captain=${captainTs} > worker=${workerTs} + 60s`,
  };
}

// Invariant 3 — worker_did_external_work
//
// Worker tool_call real event:
//   {"type":"tool_call","name":"web_fetch","arguments":{"url":"https://hn.algolia.com/..."},"call_id":"toolu_01QVJBAwrjURumjVXWjaE66P"}
// Worker tool_result real event:
//   {"type":"tool_result","name":"web_fetch","call_id":"toolu_01QVJBAwrjURumjVXWjaE66P","success":true,"content":"{\"hits\":[..."}
// Pairing is by call_id.
//
// IMPORTANT: the cascade test uses web_fetch BUT NOT execute_bash. POC #23
// uses both. We treat execute_bash as optional-if-present so calibration
// against cascade still passes. If you want a strict "both tools used"
// variant, add 'execute_bash' to input.requireWorkerTools.
function invWorkerDidExternalWork(workerEvents, requiredTools) {
  const tools = Array.isArray(requiredTools) && requiredTools.length > 0
    ? requiredTools
    : ['web_fetch'];
  const toolsByCallId = {};
  for (const e of workerEvents) {
    if (!e) continue;
    if (e.type === 'tool_result' && e.call_id) {
      toolsByCallId[e.call_id] = e;
    }
  }
  const evidence = {};
  let allOk = true;
  for (const toolName of tools) {
    const calls = findEvents(
      workerEvents,
      (e) => e && e.type === 'tool_call' && e.name === toolName,
    );
    let okForTool = false;
    for (const call of calls) {
      const res = toolsByCallId[call.call_id];
      if (!res) continue;
      const success = res.success === true;
      const contentStr = String(
        res.content !== undefined ? res.content : res.result !== undefined ? res.result : '',
      );
      const notErrorPrefix = !contentStr.trim().toLowerCase().startsWith('error');
      if (success && notErrorPrefix) {
        okForTool = true;
        break;
      }
    }
    evidence[toolName] = { call_count: calls.length, had_success_pair: okForTool };
    if (!okForTool) allOk = false;
  }
  return {
    name: 'worker_did_external_work',
    status: allOk ? 'PASS' : 'FAIL',
    evidence: { required: tools, perTool: evidence },
    reasoning: allOk
      ? `all required worker tools produced a successful result: ${tools.join(', ')}`
      : `missing success pair for one or more required tools: ${tools.join(', ')}`,
  };
}

// Invariant 4 — reverse_path_existed
//
// In the cascade, Worker reverse-delegates with delegate_task (real event):
//   {"type":"tool_call","name":"delegate_task","arguments":{"recipient":"034aa44668fbc73c...","capabilities":["working_memory_set","send_message","memory_store"],...}}
// In POC #23 the reverse hop could be delegate_task OR send_message.
// Recipient check: if input.captainIdentityKey is provided, we assert
// the args.recipient matches. Otherwise we accept any reverse call.
function invReversePathExisted(workerEvents, captainIdentityKey) {
  const calls = findEvents(
    workerEvents,
    (e) =>
      e &&
      e.type === 'tool_call' &&
      (e.name === 'delegate_task' || e.name === 'send_message'),
  );
  if (calls.length === 0) {
    return {
      name: 'reverse_path_existed',
      status: 'FAIL',
      evidence: { reverse_call_count: 0 },
      reasoning: 'Worker has no delegate_task or send_message tool_call',
    };
  }
  if (!captainIdentityKey) {
    return {
      name: 'reverse_path_existed',
      status: 'PASS',
      evidence: {
        reverse_call_count: calls.length,
        first: { ts: calls[0].ts, name: calls[0].name },
        captain_key_not_provided: true,
      },
      reasoning: `worker made ${calls.length} reverse-path call(s); captain key not provided so recipient not checked`,
    };
  }
  const matched = calls.filter((e) => {
    const args = e.arguments || {};
    const r = args.recipient || args.recipient_key || args.to;
    return typeof r === 'string' && r === captainIdentityKey;
  });
  const ok = matched.length > 0;
  return {
    name: 'reverse_path_existed',
    status: ok ? 'PASS' : 'FAIL',
    evidence: {
      reverse_call_count: calls.length,
      recipient_matched_count: matched.length,
      captain_key: captainIdentityKey,
    },
    reasoning: ok
      ? `worker made ${calls.length} reverse calls, ${matched.length} targeting captain identity`
      : `worker made ${calls.length} reverse calls but none matched captain identity`,
  };
}

function invCommissionMessagesFired(input) {
  // Softer threshold than Layer 1: "did commission fire at all".
  const logs = input.stderrLogs || {};
  let anySent = false;
  let anyReceived = false;
  const perAgent = {};
  for (const [name, p] of Object.entries(logs)) {
    const counts = countCommissionMessages(p);
    perAgent[name] = counts;
    if (counts.sent > 0) anySent = true;
    if (counts.received > 0) anyReceived = true;
  }
  const ok = anySent || anyReceived;
  return {
    name: 'commission_messages_fired',
    status: ok ? 'PASS' : 'FAIL',
    evidence: { perAgent, anySent, anyReceived },
    reasoning: ok
      ? 'at least one commission-related stderr entry observed'
      : 'no commission-related stderr entries in any agent log',
  };
}

// ---------------------------------------------------------------------------
// inspectRun — main entry point
// ---------------------------------------------------------------------------

async function inspectRun(input) {
  const startedAt = Date.now();
  const warnings = [];
  const workspaces = input.workspaces || {};
  const taskRecords = input.taskRecords || {};
  const skipInvariants = input.skipInvariants || [];

  // 1. Load transcripts for each workspace
  const transcriptsByAgent = {};
  const primaryByAgent = {};
  for (const [agent, ws] of Object.entries(workspaces)) {
    try {
      const list = await loadWorkspaceTranscripts(ws);
      transcriptsByAgent[agent] = list;
      // Precedence: explicit primaryTaskIds override > taskRecords.task_id/id.
      // primaryTaskIds is for calibration/debug where the "captain task"
      // recorded in taskRecords is a different task from the forward-delegate
      // transcript we want to inspect.
      const preferredTaskId = (input.primaryTaskIds && input.primaryTaskIds[agent]) ||
        (taskRecords[agent] && taskRecords[agent].task_id) ||
        (taskRecords[agent] && taskRecords[agent].id) ||
        null;
      primaryByAgent[agent] = pickPrimaryTranscript(list, preferredTaskId);
    } catch (err) {
      warnings.push(`[inspector] failed to load transcripts for ${agent}: ${err.message}`);
      transcriptsByAgent[agent] = [];
      primaryByAgent[agent] = null;
    }
  }

  const captainEvents = (primaryByAgent.captain && primaryByAgent.captain.events) || [];
  const workerEvents = (primaryByAgent.worker && primaryByAgent.worker.events) || [];

  // Primary-only map for scoped aggregates: budget check and totals focus
  // on the single delegated-task transcript per agent, not every task ever
  // run in the workspace.
  const primaryMap = {};
  for (const [agent, primary] of Object.entries(primaryByAgent)) {
    primaryMap[agent] = primary ? [primary] : [];
  }

  // 2. Totals (scoped to primary transcripts)
  const { total: satsSpent, perAgent: satsPerAgent } = sumThinkResponseSats(primaryMap);
  let txCount = 0;
  const perAgentIters = {};
  for (const [agent, list] of Object.entries(primaryMap)) {
    let iters = 0;
    for (const t of list) {
      for (const e of t.events) {
        if (e && e.type === 'proof_created') txCount++;
        if (e && e.type === 'session_end' && typeof e.iterations === 'number') {
          iters += e.iterations;
        }
      }
    }
    perAgentIters[agent] = iters;
  }

  // 3. Layer 1 checks
  const layer1Checks = [];
  const l1Runners = [
    () => checkProofBijection(input),
    () => checkWalletTxDelta(input),
    () => checkCaptainTaskComplete(input),
    () => checkCommissionSettled(input),
    () => checkBudgetRespected(input, primaryMap),
  ];
  for (const run of l1Runners) {
    try {
      layer1Checks.push(run());
    } catch (err) {
      const name = (run.name || 'unknown').replace(/^bound /, '');
      warnings.push(`[inspector] L1 check ${name} threw: ${err.message}`);
      layer1Checks.push({
        name,
        status: 'FAIL',
        evidence: { error: err.message },
        reasoning: `check threw: ${err.message}`,
      });
    }
  }
  const layer1Status = layer1Checks.every((c) => c.status === 'PASS') ? 'PASS' : 'FAIL';

  // 4. Layer 2 invariants
  const layer2Invariants = [];
  const l2Runners = [
    {
      name: 'overlay_was_used',
      run: () => invOverlayWasUsed(captainEvents),
    },
    {
      name: 'delegation_happened',
      run: () => invDelegationHappened(captainEvents, workerEvents),
    },
    {
      name: 'worker_did_external_work',
      run: () => invWorkerDidExternalWork(workerEvents, input.requireWorkerTools),
    },
    {
      name: 'reverse_path_existed',
      run: () => invReversePathExisted(workerEvents, input.captainIdentityKey),
    },
    {
      name: 'commission_messages_fired',
      run: () => invCommissionMessagesFired(input),
    },
  ];
  for (const { name, run } of l2Runners) {
    if (isSkipped(name, skipInvariants)) {
      layer2Invariants.push(skipInv(name, `skipped via input.skipInvariants`));
      continue;
    }
    const result = safeInv(name, run);
    if (result._predicate_threw) {
      warnings.push(`[inspector] L2 invariant ${name} threw: ${result.reasoning}`);
      delete result._predicate_threw;
    }
    layer2Invariants.push(result);
  }
  // Layer 2 pass = every non-skip invariant is PASS
  const nonSkippedL2 = layer2Invariants.filter((i) => i.status !== 'SKIP');
  const layer2Status = nonSkippedL2.length > 0 && nonSkippedL2.every((i) => i.status === 'PASS')
    ? 'PASS'
    : nonSkippedL2.length === 0
      ? 'PASS'
      : 'FAIL';

  // 5. Layer 3 — deferred
  const layer3 = {
    status: 'DEFERRED',
    reason:
      'Layer 3 (expected-chain DSL) not implemented in Phase 2 first pass. Add in follow-up after Layers 1+2 are green on real runs.',
  };

  // 6. Verdict
  const verdict = layer1Status === 'PASS' && layer2Status === 'PASS' ? 'PASS' : 'FAIL';
  const exitCode = verdict === 'PASS' ? 0 : 2;

  return {
    verdict,
    exitCode,
    runNonce: input.runNonce,
    layer1: { status: layer1Status, checks: layer1Checks },
    layer2: { status: layer2Status, invariants: layer2Invariants },
    layer3,
    totals: {
      satsSpent,
      satsPerAgent,
      txCount,
      wallClockMs: Date.now() - startedAt,
      perAgentIters,
    },
    warnings,
    generatedAt: new Date().toISOString(),
  };
}

module.exports = {
  inspectRun,
  readSessionJsonl,
  findEvents,
  // exposed for testing
  _internal: {
    loadWorkspaceTranscripts,
    pickPrimaryTranscript,
    sumThinkResponseSats,
    countCommissionMessages,
  },
};
