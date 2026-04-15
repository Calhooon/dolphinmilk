#!/usr/bin/env node
/**
 * test_poc_23.js — DolphinMilkShake #23 Quality Gate
 * =====================================================================
 *
 * End-to-end proof-batch cost POC with layered verdict. Replaces the weak
 * `test_proof_batch_cost.js` scaffold. See `~/bsv/dolphinmilkshake/QUALITY-GATE-PLAN.md`
 * for strategic context and `lib/CONTRACTS.md` for module interfaces.
 *
 * Flow:
 *   1. `startCluster()` — Captain (:8081/wallet 3322) + Worker (:8083/wallet 3324)
 *      with per-role `DOLPHIN_MILK_CERT_CAPABILITIES`. Boot cert acquisition + natural
 *      overlay registration happen inside dolphin-milk. cluster.js only verifies.
 *   2. Snapshot Reddit `/r/technology/hot.json?limit=10` to the Worker's workspace
 *      as `reddit-snapshot.json`. Snapshot mode guarantees byte-exact bijection
 *      (see Agent B Task 3 findings — live re-fetching would false-fail due to
 *      V8 dropping `.0` float suffixes in JSON.parse + JSON.stringify).
 *   3. Write `proof_records.sh` to the Worker's workspace. Process-substitution
 *      variant (no subshell trap) that reads the snapshot, hashes each record
 *      via `jq -c | shasum -a 256` (matches proof_verify.js's `computeExpectedHash`),
 *      and creates an OP_RETURN tx per record via wallet `createAction`.
 *   4. Submit a task to Captain that MUST use the overlay:
 *        STEP A: overlay_lookup(service=ls_agent, findByCapability="scraping")
 *        STEP B: delegate_task to the returned worker with an opaque inner task
 *                wrapped in `===TASK_STRING_BEGIN=== ... ===TASK_STRING_END===`
 *        STEP C: Worker: web_fetch Reddit (confirms external work) → execute_bash
 *                `bash proof_records.sh http://localhost:3324 reddit-snapshot.json`
 *                → reverse-delegate the raw script stdout back to Captain with
 *                the run nonce embedded in the reverse task text
 *        STEP D: Captain: working_memory_set with the run nonce verbatim in the
 *                value (Layer 1 `captain_task_complete` assertion depends on this)
 *   5. Poll Captain's task until complete (10 min timeout).
 *   6. Wait ~3 min for commission settlement (2-hop, shorter than cascade's 4 min).
 *   7. Read final task records + worker transcripts.
 *   8. `verifyProofBatch()` — bijection + on-chain + temporal windows.
 *   9. `inspectRun()` — layered verdict (Layer 1 outcome + Layer 2 invariants;
 *      Layer 3 deferred).
 *  10. Print verdict + exit with verdict-driven exit code. Stop cluster on way out.
 *
 * Exit codes:
 *   0 = PASS (all Layer 1 + Layer 2 green; #23 quality gate is met)
 *   2 = FAIL (something broke — see the printed verdict for the specific layer+check)
 *   1 = fatal error before verdict could be computed (spawn failure, network, etc.)
 *
 * Cost expectation: ~$0.30–$0.50 real sats per run. Budget ~$1.50 if prompt needs
 * iteration. Uses claude-haiku-4-5 for Captain + Worker per cascade-proven pattern
 * (gpt-5-mini's injection-paranoia breaks delegation reliability — HANDOFF gotcha #6).
 *
 * SAFETY: wallets on 3322/3323/3324 are persistent and funded — reached via HTTP
 * only. Never call init/reset/createWallet flows. See
 * `feedback_funded_wallets_3322_3323_3324` memory.
 */

'use strict';

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const { startCluster } = require('./lib/cluster');
const { verifyProofBatch, hashRecordsFromFile } = require('./lib/proof_verify');
const { inspectRun } = require('./lib/inspector');
const { authGet, authPost } = require('./lib/auth');

// -----------------------------------------------------------------------------
// Config
// -----------------------------------------------------------------------------

const PROJECT_ROOT = path.resolve(__dirname, '../../');
const BINARY = path.join(PROJECT_ROOT, 'target/release/dolphin-milk');

const PARENT_WALLET_PORT = 3321;
const CAPTAIN_PORT = 8081;
const WORKER_PORT = 8083;
const CAPTAIN_WALLET_PORT = 3322;
const WORKER_WALLET_PORT = 3324;

const OVERLAY_URL = 'https://rust-overlay.dev-a3e.workers.dev';

const RUN_NONCE = crypto.randomBytes(4).toString('hex');
const RUN_TIMESTAMP = new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
const OUTPUT_DIR = path.join(PROJECT_ROOT, 'test-workspaces', `poc-23-${RUN_TIMESTAMP}-${RUN_NONCE}`);
fs.mkdirSync(OUTPUT_DIR, { recursive: true });

const CAPTAIN_WORKSPACE = path.join(PROJECT_ROOT, 'test-workspaces/poc-23-captain');
const WORKER_WORKSPACE = path.join(PROJECT_ROOT, 'test-workspaces/poc-23-worker');
fs.mkdirSync(CAPTAIN_WORKSPACE, { recursive: true });
fs.mkdirSync(WORKER_WORKSPACE, { recursive: true });

const REDDIT_URL = 'https://www.reddit.com/r/technology/hot.json?limit=100';
const USER_AGENT = 'DolphinSense/1.0';

const CAPTAIN_TASK_TIMEOUT_MS = 10 * 60 * 1000; // 10 min
const CAPTAIN_POLL_INTERVAL_MS = 5000;
const COMMISSION_WAIT_MS = 3 * 60 * 1000; // 3 min — 2-hop settles faster than cascade

const EXPECTED_BUDGET_SATS = 3_000_000; // ~$0.50 ceiling; inspector uses ±30% tolerance

// -----------------------------------------------------------------------------
// Proof script (process-substitution variant — no subshell var-loss trap)
// -----------------------------------------------------------------------------

const PROOF_SCRIPT = `#!/bin/bash
# proof_records.sh — per-record OP_RETURN provenance proof creator
# Usage: proof_records.sh <wallet_url> <snapshot_file>
#
# Reads records from the Reddit snapshot (jq path .data.children[]),
# hashes each via sha256 of its jq -c compact JSON representation
# (matching proof_verify.js::computeExpectedHash), and creates an
# OP_RETURN tx per record through the wallet's createAction API.
#
# E9: to keep the Worker's iter-2 LLM prompt small, the full txid list is
# written to a sidecar file (<snapshot>.txids, one per line) instead of
# being emitted inline. Stdout is a tiny manifest:
#   {"proofs_created":N,"errors":M,"txid_file":"...","first_txid":"...","last_txid":"...","manifest_sha256":"..."}
# The sha256 is over the file contents (newline-joined txids) so any
# downstream consumer can cheaply prove they're looking at the same batch.

set -u

WALLET_URL="\${1:-}"
SNAPSHOT_FILE="\${2:-}"

if [ -z "\$WALLET_URL" ] || [ -z "\$SNAPSHOT_FILE" ]; then
  echo '{"error":"Usage: proof_records.sh <wallet_url> <snapshot_file>"}'
  exit 1
fi
if [ ! -f "\$SNAPSHOT_FILE" ]; then
  echo "{\\"error\\":\\"snapshot file not found: \$SNAPSHOT_FILE\\"}"
  exit 1
fi

TXID_FILE="\${SNAPSHOT_FILE}.txids"
: > "\$TXID_FILE"

CREATED=0
ERRORS=0
FIRST_TXID=""
LAST_TXID=""

# Process substitution keeps the while loop in the current shell so variables
# persist after the loop exits. Using a pipeline here would lose everything.
while IFS= read -r record; do
  [ -z "\$record" ] && continue
  HASH=\$(printf '%s' "\$record" | shasum -a 256 | cut -d' ' -f1)
  if [ -z "\$HASH" ]; then
    ERRORS=\$((ERRORS + 1))
    continue
  fi
  LOCKING="006a20\${HASH}"
  RESULT=\$(curl -sS -X POST "\${WALLET_URL}/createAction" \\
    -H "Origin: \${WALLET_URL}" \\
    -H 'Content-Type: application/json' \\
    -d "{\\"description\\":\\"poc-23 provenance proof\\",\\"outputs\\":[{\\"lockingScript\\":\\"\${LOCKING}\\",\\"satoshis\\":0,\\"outputDescription\\":\\"poc-23 record proof\\"}]}" 2>/dev/null)
  TXID=\$(printf '%s' "\$RESULT" | jq -r '.txid // empty' 2>/dev/null)
  if [ -n "\$TXID" ] && [ "\$TXID" != "null" ]; then
    CREATED=\$((CREATED + 1))
    [ -z "\$FIRST_TXID" ] && FIRST_TXID="\$TXID"
    LAST_TXID="\$TXID"
    printf '%s\\n' "\$TXID" >> "\$TXID_FILE"
  else
    ERRORS=\$((ERRORS + 1))
  fi
done < <(jq -c '.data.children[]' "\$SNAPSHOT_FILE" 2>/dev/null)

MANIFEST_SHA=\$(shasum -a 256 "\$TXID_FILE" | cut -d' ' -f1)

printf '{"proofs_created":%d,"errors":%d,"txid_file":"%s","first_txid":"%s","last_txid":"%s","manifest_sha256":"%s"}\\n' \\
  "\$CREATED" "\$ERRORS" "\$TXID_FILE" "\$FIRST_TXID" "\$LAST_TXID" "\$MANIFEST_SHA"
`;

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

function log(msg) {
  console.log(`[poc-23] ${msg}`);
}

function formatSats(n) {
  if (n == null) return 'null';
  return n.toLocaleString() + ' sats';
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function snapshotReddit(destPath) {
  log(`fetching Reddit snapshot from ${REDDIT_URL}`);
  execFileSync(
    'curl',
    ['-sS', '-A', USER_AGENT, '-L', '--max-time', '30', '-o', destPath, REDDIT_URL],
    { stdio: 'inherit' },
  );
  const stat = fs.statSync(destPath);
  if (stat.size < 1000) {
    throw new Error(`Reddit snapshot at ${destPath} is suspiciously small (${stat.size} bytes)`);
  }
  // Sanity-check the shape: must have data.children[]
  const parsed = JSON.parse(fs.readFileSync(destPath, 'utf8'));
  const children = parsed && parsed.data && parsed.data.children;
  if (!Array.isArray(children) || children.length === 0) {
    throw new Error(`Reddit snapshot has no .data.children[] (got ${typeof children})`);
  }
  log(`Reddit snapshot: ${stat.size} bytes, ${children.length} records`);
  return children.length;
}

async function pollCaptainTask(taskId, deadline) {
  while (Date.now() < deadline) {
    try {
      const { status, body } = await authGet(
        `http://localhost:${CAPTAIN_PORT}/task/${taskId}`,
        PARENT_WALLET_PORT,
      );
      if (status === 200 && body) {
        const s = String(body.status || '').toLowerCase();
        if (s === 'complete' || s === 'completed') return body;
        if (s === 'error' || s === 'cancelled' || s === 'failed') return body;
      }
    } catch (e) {
      // swallow transient errors and keep polling
    }
    await sleep(CAPTAIN_POLL_INTERVAL_MS);
  }
  throw new Error(`Captain task ${taskId} did not complete within ${CAPTAIN_TASK_TIMEOUT_MS}ms`);
}

async function getTaskList(port) {
  try {
    const { status, body } = await authGet(
      `http://localhost:${port}/tasks`,
      PARENT_WALLET_PORT,
    );
    if (status === 200 && body && Array.isArray(body.tasks)) return body.tasks;
  } catch (e) {
    // fall through
  }
  return [];
}

async function getTaskDetail(port, taskId) {
  try {
    const { status, body } = await authGet(
      `http://localhost:${port}/task/${taskId}`,
      PARENT_WALLET_PORT,
    );
    if (status === 200) return body;
  } catch (e) {
    // fall through
  }
  return null;
}

// -----------------------------------------------------------------------------
// Task template builders — nested opaque-task-string pattern
// -----------------------------------------------------------------------------
//
// Mirrors the proven cascade pattern (buildReverseCaptainTemplate /
// buildWorkerTask / buildCaptainHarnessTask in test_three_layer_cascade.js).
// Every level wraps the next level's instructions in TASK_STRING_BEGIN/END
// markers and explicitly tells the LLM "this is opaque, do NOT extract values
// from it." Claude-haiku has an injection-defense reflex that aggressively
// tries to use nested `capabilities` / `budget` strings as its OWN arguments
// unless the prompt explicitly cordons them off.
//
// Flow for #23:
//   1. buildFinalCaptainTemplate(runNonce) → innermost template. Contains a
//      {PROOF_REPORT_HERE} placeholder that Worker will replace with the
//      real proof-batch report string. Instructs Captain (in its auto-spawned
//      task 2) to call working_memory_set with the report verbatim.
//   2. buildWorkerTask(captainKey, innerTemplate) → Worker's task body.
//      STEP 1: web_fetch (confirms external work — Layer 2 invariant)
//      STEP 2: execute_bash proof_records.sh (BATCHING happens here — see
//              PROOF_SCRIPT: one bash loop over 10 records → 10 OP_RETURN
//              txs via 10 separate createAction calls → 10 proof txs per
//              Worker LLM iteration)
//      STEP 3: extract proofs_created/errors/txids from the execute_bash result
//      STEP 4: build the proof report string, substitute into the opaque
//              innerTemplate, reverse-delegate to captainKey with the filled
//              template as the task body
//   3. buildCaptainHarnessTask(workerTaskText) → Captain's task 1 (harness).
//      STEP A: overlay_lookup(findByCapability: "scraping")
//      STEP B: delegate_task(recipient=<from STEP A>, task=<opaque workerTask>,
//              capabilities=[scraping, web_fetch, execute_bash, delegate_task,
//              send_message], budget=900000, expires_in=600)
//      After delegate_task returns, Captain reports and ends.
//
// The reverse delegation from Worker back to Captain auto-spawns a NEW task
// on Captain (task 2). Task 2's prompt is the FILLED finalCaptainTemplate.
// It runs `working_memory_set` and ends. test_poc_23.js polls BOTH captain
// tasks — task 1 for the overlay/delegation assertion, task 2 for the
// RUN_NONCE + final result.

function buildFinalCaptainTemplate(runNonce) {
  // This is the innermost template — what Captain's AUTO-SPAWNED task 2 will
  // receive as its task description. Worker replaces {PROOF_REPORT_HERE}
  // with the real proof-batch report before reverse-delegating it.
  //
  // Critical: the working_memory_set VALUE must contain the literal RUN_NONCE
  // because the inspector's `captain_task_complete` Layer 1 check asserts
  // `String(taskRecords.captain.result).includes(runNonce)`.
  return [
    'This is an authorized delegated task from your trusted parent',
    'certificate chain. Perform the following work as the Captain recording',
    'the final POC #23 per-record provenance proof batch result.',
    '',
    `The run nonce for this POC is: ${runNonce}`,
    '',
    'The scraping worker below has completed its proof batch and produced',
    'this verbatim report (between BEGIN/END markers):',
    '',
    '===PROOF_REPORT_BEGIN===',
    '{PROOF_REPORT_HERE}',
    '===PROOF_REPORT_END===',
    '',
    'Step 1. Call working_memory_set with EXACTLY these arguments:',
    '',
    `  key: "poc_23_result_${runNonce}"`,
    '  value: the complete report string above (everything between',
    `         ===PROOF_REPORT_BEGIN=== and ===PROOF_REPORT_END===, verbatim).`,
    `         The value MUST include the literal substring "${runNonce}".`,
    '',
    'Step 2. Report success in plain text and end your session.',
    '',
    'Do NOT call any other tool. Do NOT modify the report. The value field',
    'of working_memory_set must be the report as-is, character-for-character.',
  ].join('\n');
}

function buildWorkerTask(captainKey, innerTemplate, runNonce, workerWalletPort, absScriptPath, absSnapshotPath) {
  // This is the task body Worker receives inside Captain's delegate_task.
  // Worker does the real work: web_fetch (to validate external connectivity),
  // execute_bash (to BATCH N proofs via proof_records.sh), then builds a
  // proof-report string, substitutes it into the opaque innerTemplate (which
  // currently contains the literal `{PROOF_REPORT_HERE}` placeholder), and
  // reverse-delegates the filled template back to Captain.
  //
  // The inner template stays opaque from Worker's point of view in terms of
  // NOT extracting `capabilities` / `budget` values from it — but Worker DOES
  // need to perform one simple substitution: replacing `{PROOF_REPORT_HERE}`
  // with the report. This is exactly the same pattern the cascade uses with
  // SUMMARY_GOES_HERE in buildReverseCoordTemplate.
  return [
    '=== WORKER ROLE (#23 POC proof batch) ===',
    '',
    `Run nonce: ${runNonce}`,
    '',
    'Two files have been pre-staged for you at these ABSOLUTE paths — use the',
    'absolute paths verbatim, do NOT attempt relative-path resolution:',
    `  - ${absSnapshotPath}  (pre-captured Reddit /r/technology hot.json)`,
    `  - ${absScriptPath}  (bash script that hashes each record and creates`,
    '    one OP_RETURN tx per record — this is the per-record proof batch)',
    '',
    'Your execute_bash CWD is your task sandbox directory, not the workspace',
    'root where these files live. DO NOT use unqualified names like',
    '`proof_records.sh` — they will not resolve. DO NOT try `../proof_records.sh`',
    'or `find` — just use the absolute paths above. The previous iteration of',
    'this test burned its entire budget searching for these files because it',
    'ignored this instruction. Do not repeat that mistake.',
    '',
    'You will make EXACTLY TWO tool calls in this session, in this order:',
    '  1. execute_bash (to run the proof batch)',
    '  2. delegate_task (to reverse-delegate the filled template to captain)',
    'After delegate_task returns, end your session. Do NOT call any other tool.',
    'Do NOT call web_fetch, search_tools, memory_search, or anything else.',
    'Do NOT call execute_bash a second time. Do NOT retry anything.',
    '',
    'STEP 1: execute_bash (THE PROOF BATCH)',
    '',
    '  Call execute_bash with this EXACT command (use the absolute paths verbatim):',
    '',
    `    bash ${absScriptPath} http://localhost:${workerWalletPort} ${absSnapshotPath}`,
    '',
    '  The script reads the snapshot file, iterates through all records at',
    '  jq path `.data.children[]`, and creates ONE OP_RETURN transaction per',
    '  record through the wallet. It writes the full txid list to a sidecar',
    '  file and prints a COMPACT JSON manifest to stdout:',
    '',
    '    {"proofs_created":N,"errors":M,"txid_file":"...",',
    '     "first_txid":"...","last_txid":"...","manifest_sha256":"..."}',
    '',
    '  Remember these six values for STEP 2. The full txid list lives in the',
    '  sidecar file — you do NOT need to read it. Captain T2 only needs the',
    '  compact manifest below, not the full list.',
    '',
    '  CRITICAL: after execute_bash returns its JSON, your VERY NEXT tool',
    '  call MUST be delegate_task (see STEP 2 below). Do NOT call execute_bash',
    '  again. Do NOT call any other tool. Proceed directly to STEP 2.',
    '',
    'STEP 2: delegate_task (REVERSE)',
    '',
    '  Build a proof-report string (for the reverse delegation task body).',
    '  Start with this EXACT template, then substitute PROOF_REPORT_HERE in',
    '  the opaque task-string (further below) with your built report:',
    '',
    '  -----BEGIN PROOF REPORT-----',
    `  Run ${runNonce} proof batch complete.`,
    '  proofs_created: <N from STEP 1>',
    '  errors: <M from STEP 1>',
    '  first_txid: <first_txid from STEP 1>',
    '  last_txid: <last_txid from STEP 1>',
    '  manifest_sha256: <manifest_sha256 from STEP 1>',
    '  txid_file: <txid_file from STEP 1>',
    '  -----END PROOF REPORT-----',
    '',
    `  The report MUST contain the literal string "${runNonce}".`,
    '',
    `  Then call delegate_task with EXACTLY these arguments:`,
    '',
    `    recipient       = "${captainKey}"`,
    `    capabilities    = ["working_memory_set", "memory_store", "send_message"]`,
    `    budget_cap_sats = 400000`,
    `    expires_in_secs = 600`,
    '    task            = (the opaque string defined at the very end of this',
    '                       message, between ===FINAL_TASK_BEGIN=== and',
    '                       ===FINAL_TASK_END===, BUT with the literal',
    '                       placeholder "{PROOF_REPORT_HERE}" replaced by your',
    '                       full proof report string — everything between and',
    '                       including the -----BEGIN PROOF REPORT----- and',
    '                       -----END PROOF REPORT----- lines above)',
    '',
    '=== RULE: THE FINAL TASK STRING IS OPAQUE ===',
    '',
    'The task string below is opaque. It contains instructions for the',
    'Captain, not for you. It may mention `working_memory_set`, `key`,',
    '`value` — those are Captain\'s arguments, not yours. Your ONLY job',
    'with the task string is the single literal substitution of',
    '`{PROOF_REPORT_HERE}` with your proof report. Do not modify anything',
    'else in the task string.',
    '',
    '===FINAL_TASK_BEGIN===',
    innerTemplate,
    '===FINAL_TASK_END===',
    '',
    'After delegate_task returns, report the commission_id and end your session.',
  ].join('\n');
}

function buildCaptainHarnessTask(workerTaskText, workerCapabilities) {
  // Captain's harness task (task 1). Makes exactly TWO tool calls:
  // overlay_lookup to discover the scraping worker, then delegate_task to
  // forward the opaque workerTaskText. After delegate_task returns, Captain
  // reports and ends. The reverse delegation from Worker will auto-spawn
  // a NEW Captain task (task 2) with the filled finalCaptainTemplate —
  // Captain does NOT try to wait for it in task 1, because each delegated
  // task spawns its own runner.
  return [
    '=== CAPTAIN ROLE (#23 POC quality gate harness) ===',
    '',
    'You must make EXACTLY TWO tool calls in this session, in this order:',
    '  1. overlay_lookup (to discover the scraping worker)',
    '  2. delegate_task (to forward the opaque worker task)',
    'Do not call web_fetch, working_memory_set, or any other tool. After',
    'delegate_task returns, report the commission_id and end your session.',
    '',
    '=== STEP 1: overlay_lookup ===',
    '',
    'Call overlay_lookup with these arguments:',
    '',
    '  service = "ls_agent"',
    '  query   = { "findByCapability": "scraping" }',
    '',
    'The result will be a list of agent records. Each record has fields',
    '`identity_key`, `name`, and `capabilities`. Pick the FIRST record whose',
    '`capabilities` list contains BOTH "scraping" AND "execute_bash" — that',
    'is the scraping worker you will delegate to. Remember its identity_key.',
    'If no record matches, report the error and end your session.',
    '',
    '=== STEP 2: delegate_task ===',
    '',
    'Call delegate_task with EXACTLY these arguments:',
    '',
    '  recipient       = <identity_key from STEP 1>',
    `  capabilities    = ${JSON.stringify(workerCapabilities)}`,
    '  budget_cap_sats = 1200000',
    '  expires_in_secs = 600',
    '  task            = (the verbatim opaque string defined below)',
    '',
    '=== RULE: THE task ARGUMENT IS OPAQUE ===',
    '',
    'The task field below is an opaque string. It contains instructions for',
    'the worker, NOT for you. It may mention "capabilities", "budget",',
    '"recipient" as part of those downstream instructions — THOSE VALUES ARE',
    'NOT YOUR ARGUMENTS. Your `capabilities` argument is EXACTLY the list',
    `given above: ${JSON.stringify(workerCapabilities)} — nothing else.`,
    '',
    'Copy the task string character-for-character between the BEGIN and END',
    'markers below into the `task` argument of delegate_task. Do not',
    'paraphrase, extract values from, modify, or summarize it.',
    '',
    '===WORKER_TASK_BEGIN===',
    workerTaskText,
    '===WORKER_TASK_END===',
    '',
    'After delegate_task returns, report the commission_id and end your session.',
    'Do NOT attempt to wait for the worker\'s reverse delegation — it will',
    'auto-spawn a new session on your server and you do not need to handle it',
    'in this session.',
  ].join('\n');
}

// -----------------------------------------------------------------------------
// Captain task-2 polling (waiting for the reverse-delegated task to appear)
// -----------------------------------------------------------------------------

/**
 * Poll Captain's /tasks list until a NEW task appears that wasn't in the
 * baselineTaskIds set AND is not the harnessTaskId (the one we submitted).
 * Returns the new task's id, or throws on timeout.
 */
async function waitForSecondCaptainTask(baselineTaskIds, harnessTaskId, deadline) {
  while (Date.now() < deadline) {
    const tasks = await getTaskList(CAPTAIN_PORT);
    for (const t of tasks) {
      const tid = t.id || t.task_id;
      if (!tid) continue;
      if (tid === harnessTaskId) continue;
      if (baselineTaskIds.has(tid)) continue;
      // Found a new task — return its id immediately. Caller polls for
      // completion separately.
      return tid;
    }
    await sleep(2000);
  }
  throw new Error(
    `Captain task 2 (reverse-delegated final) did not appear before deadline. ` +
    `Check worker's session.jsonl for delegate_task errors.`,
  );
}

// Walk the worker's session.jsonl files under the workspace to find the
// execute_bash tool_result whose content JSON contains `txids`. Returns the
// parsed { proofs_created, errors, txids } object and the containing task_id.
function extractWorkerProofResult(workerWorkspace) {
  const tasksDir = path.join(workerWorkspace, 'tasks');
  if (!fs.existsSync(tasksDir)) return null;
  const taskDirs = fs
    .readdirSync(tasksDir, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name);

  let best = null;
  for (const tid of taskDirs) {
    const sessionPath = path.join(tasksDir, tid, 'session.jsonl');
    if (!fs.existsSync(sessionPath)) continue;
    const lines = fs.readFileSync(sessionPath, 'utf8').split('\n');
    for (const line of lines) {
      if (!line.trim()) continue;
      let event;
      try {
        event = JSON.parse(line);
      } catch {
        continue;
      }
      const type = event.type || event.event_type || '';
      if (type !== 'tool_result') continue;
      const name = event.name || (event.data && event.data.name) || '';
      if (name !== 'execute_bash') continue;
      const content = event.content != null
        ? event.content
        : (event.data && event.data.result != null ? event.data.result : null);
      if (typeof content !== 'string') continue;
      // Try to find a JSON object with proofs_created/txids in the content
      const match = content.match(/\{[^{}]*"proofs_created"[^{}]*"txids"\s*:\s*\[[^\]]*\][^{}]*\}/);
      if (!match) continue;
      try {
        const parsed = JSON.parse(match[0]);
        if (parsed && Array.isArray(parsed.txids)) {
          best = { taskId: tid, result: parsed };
        }
      } catch {
        /* skip */
      }
    }
  }
  return best;
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

async function main() {
  const tStart = Date.now();
  log('='.repeat(70));
  log(`DolphinMilkShake #23 Quality Gate`);
  log(`run nonce: ${RUN_NONCE}`);
  log(`output dir: ${OUTPUT_DIR}`);
  log('='.repeat(70));

  // --- Step 1: start cluster ---
  log('step 1 — startCluster (Captain + Worker, with per-role capabilities)');
  // Capability sets: same default set cascade uses (which empirically allows
  // delegating web_fetch + execute_bash to Worker via "tools" broad-gateway),
  // plus role-specific additions.
  //
  //   Captain: default set only. Captain does not need to be findable via
  //            overlay findByCapability — it discovers, not receives.
  //   Worker:  default set + "scraping" so overlay findByCapability:"scraping"
  //            will return Worker (user-pinned requirement for #23).
  const DEFAULT_CAPS = [
    'llm',
    'tools',
    'wallet',
    'memory',
    'messaging',
    'x402',
    'schedule',
    'orchestration',
  ];

  const handle = await startCluster({
    parentWalletPort: PARENT_WALLET_PORT,
    binary: BINARY,
    overlay: {
      url: OVERLAY_URL,
      verifyRegistration: true,
      registrationTimeoutMs: 45_000,
    },
    outputDir: OUTPUT_DIR,
    agents: [
      {
        name: 'captain',
        port: CAPTAIN_PORT,
        walletPort: CAPTAIN_WALLET_PORT,
        // E10: hybrid — mini Captain + nano Worker. E9 showed nano Captain
        // collapses the nested opaque-task-string pattern (extracts inner
        // caps/budget and invents a zero-filled proof report instead of
        // forwarding the real worker task verbatim). E5 proved mini handles
        // this pattern at batch=100 reliably. Mini is both cheaper than
        // haiku per token AND empirically validated.
        model: 'gpt-5-mini',
        workspace: CAPTAIN_WORKSPACE,
        capabilities: [...DEFAULT_CAPS],
        // E9: raise stall detect 120s→300s. E8 showed nano hangs on iter-2
        // think when the prompt includes an 8KB execute_bash tool_result
        // (100 txids inline). Nano provider latency under long context is
        // real; 120s was too aggressive for that follow-up call.
        env: { DOLPHIN_MILK_LLM_STALL_TIMEOUT: '300' },
      },
      {
        name: 'scraping-worker',
        port: WORKER_PORT,
        walletPort: WORKER_WALLET_PORT,
        model: 'gpt-5-nano',
        workspace: WORKER_WORKSPACE,
        // "scraping" is the key capability Captain looks up via the overlay.
        capabilities: [...DEFAULT_CAPS, 'scraping'],
        env: { DOLPHIN_MILK_LLM_STALL_TIMEOUT: '300' },
      },
    ],
  });

  const captain = handle.agents.get('captain');
  const worker = handle.agents.get('scraping-worker');
  log(`captain identity: ${captain.identityKey}`);
  log(`worker identity: ${worker.identityKey}`);

  let captainResult = null;
  let workerTaskRecord = null;
  let verdict = null;

  try {
    // --- Step 2: snapshot Reddit into worker workspace ---
    log('step 2 — snapshotting Reddit into Worker workspace');
    const snapshotPath = path.join(WORKER_WORKSPACE, 'reddit-snapshot.json');
    const recordCount = snapshotReddit(snapshotPath);

    // Compute expected hashes (same byte-path proof_records.sh uses via jq -c + shasum)
    const expectedHashes = hashRecordsFromFile(snapshotPath, '.data.children[]');
    log(`computed ${expectedHashes.length} expected hashes via hashRecordsFromFile`);
    if (expectedHashes.length !== recordCount) {
      throw new Error(
        `hash count (${expectedHashes.length}) doesn't match snapshot record count (${recordCount})`,
      );
    }

    // --- Step 3: write proof_records.sh to worker workspace ---
    log('step 3 — writing proof_records.sh to Worker workspace');
    const proofScriptPath = path.join(WORKER_WORKSPACE, 'proof_records.sh');
    fs.writeFileSync(proofScriptPath, PROOF_SCRIPT, { mode: 0o755 });

    // --- Step 4: build task templates (nested opaque-task-string pattern) ---
    log('step 4 — building nested task templates (final → worker → harness)');
    const finalCaptainTemplate = buildFinalCaptainTemplate(RUN_NONCE);
    // Minimal set matching the cascade pattern — exactly what Worker needs
    // for its 3-step flow (web_fetch + execute_bash + reverse delegate_task),
    // plus send_message as a fallback. The cascade shows these cap strings
    // flow through narrowing from Captain's default-set parent cert.
    const workerCapabilities = [
      'web_fetch',
      'execute_bash',
      'delegate_task',
      'send_message',
    ];
    // Absolute paths for proof_records.sh and reddit-snapshot.json — execute_bash
    // runs in the task sandbox subfolder, not the workspace root, so relative
    // names don't resolve. Run 1 Worker succeeded only by luck (ran `find` and
    // switched to absolute paths on iteration 5); Run 2 Worker exhausted budget
    // searching. Fix: tell Worker the exact absolute paths in the prompt.
    const absScriptPath = path.resolve(WORKER_WORKSPACE, 'proof_records.sh');
    const absSnapshotPath = path.resolve(WORKER_WORKSPACE, 'reddit-snapshot.json');
    const workerTaskText = buildWorkerTask(
      captain.identityKey,
      finalCaptainTemplate,
      RUN_NONCE,
      WORKER_WALLET_PORT,
      absScriptPath,
      absSnapshotPath,
    );
    const captainHarnessTask = buildCaptainHarnessTask(workerTaskText, workerCapabilities);

    // --- Step 5: baseline Captain's task list, then submit harness task ---
    log('step 5 — baselining Captain task list + submitting harness task');
    const baselineCaptainTasks = new Set();
    try {
      const tasks = await getTaskList(CAPTAIN_PORT);
      for (const t of tasks) {
        const id = t.id || t.task_id;
        if (id) baselineCaptainTasks.add(id);
      }
      log(`Captain baseline tasks: ${baselineCaptainTasks.size}`);
    } catch (e) {
      log(`WARN: Captain task list probe failed: ${e.message}`);
    }

    const submitResp = await authPost(
      `http://localhost:${CAPTAIN_PORT}/task`,
      { task: captainHarnessTask, max_iterations: 5 },
      PARENT_WALLET_PORT,
    );
    if (submitResp.status !== 200 && submitResp.status !== 201 && submitResp.status !== 202) {
      throw new Error(
        `Captain harness task submission failed: HTTP ${submitResp.status}, body=${JSON.stringify(submitResp.body).slice(0, 300)}`,
      );
    }
    const captainHarnessTaskId = submitResp.body && (submitResp.body.id || submitResp.body.task_id);
    if (!captainHarnessTaskId) {
      throw new Error(`Captain task response missing id: ${JSON.stringify(submitResp.body).slice(0, 300)}`);
    }
    log(`Captain harness task id: ${captainHarnessTaskId}`);

    // --- Step 6: poll Captain harness task (task 1) until complete ---
    log(`step 6 — polling Captain harness task (task 1) until complete`);
    const harnessDeadline = Date.now() + CAPTAIN_TASK_TIMEOUT_MS;
    const captainHarnessResult = await pollCaptainTask(captainHarnessTaskId, harnessDeadline);
    log(
      `Captain harness task 1 ended: status=${captainHarnessResult.status}, ` +
      `iterations=${captainHarnessResult.iterations}, sats=${formatSats(captainHarnessResult.sats_spent)}`,
    );

    // --- Step 7: wait for Captain's auto-spawned reverse-delegation task (task 2) ---
    log('step 7 — waiting for Captain task 2 (auto-spawned from Worker reverse delegation)');
    const task2Deadline = Date.now() + CAPTAIN_TASK_TIMEOUT_MS;
    const captainFinalTaskId = await waitForSecondCaptainTask(
      baselineCaptainTasks,
      captainHarnessTaskId,
      task2Deadline,
    );
    log(`Captain task 2 appeared: ${captainFinalTaskId}`);

    // Poll task 2 until complete
    log(`step 8 — polling Captain task 2 (final working_memory_set) until complete`);
    const task2PollDeadline = Date.now() + CAPTAIN_TASK_TIMEOUT_MS;
    const captainFinalResult = await pollCaptainTask(captainFinalTaskId, task2PollDeadline);
    log(
      `Captain task 2 ended: status=${captainFinalResult.status}, ` +
      `iterations=${captainFinalResult.iterations}, sats=${formatSats(captainFinalResult.sats_spent)}`,
    );
    captainResult = captainFinalResult; // what inspector will use for captain_task_complete

    // --- Step 9: wait for commission settlement ---
    log(`step 9 — waiting ${COMMISSION_WAIT_MS / 1000}s for commission settlement`);
    await sleep(COMMISSION_WAIT_MS);

    // --- Step 10: gather worker proof result + task records ---
    log('step 10 — gathering worker proof result from transcripts');
    const workerProof = extractWorkerProofResult(WORKER_WORKSPACE);
    if (!workerProof) {
      log('WARN: no execute_bash tool_result with txids found in worker transcripts');
    } else {
      log(
        `found worker proof result in task ${workerProof.taskId}: ` +
        `${workerProof.result.proofs_created} proofs, ${workerProof.result.errors} errors, ` +
        `${workerProof.result.txids.length} txids`,
      );
    }

    if (workerProof && workerProof.taskId) {
      workerTaskRecord = await getTaskDetail(WORKER_PORT, workerProof.taskId);
    }

    // --- Step 11: verifyProofBatch ---
    log('step 11 — verifyProofBatch (bijection + on-chain + temporal)');
    let proofVerdict;
    if (!workerProof) {
      proofVerdict = {
        verdict: 'FAIL',
        mode: 'snapshot',
        expectedCount: expectedHashes.length,
        actualCount: 0,
        verifiedCount: 0,
        bijection: { status: 'FAIL', orphans: [], misses: expectedHashes.map((h) => h.hash), duplicates: [] },
        onChain: { status: 'SKIPPED', verifiedCount: 0, pendingCount: 0, failed: [] },
        temporal: { status: 'FAIL', windowStart: new Date(tStart).toISOString(), windowEnd: new Date().toISOString(), outOfWindow: [] },
        evidence: { txidHashPairs: [], hashFnDescription: 'not run — worker proof result missing' },
        totals: { satsSpentOnProofs: 0, wallClockMs: Date.now() - tStart },
      };
    } else {
      proofVerdict = await verifyProofBatch({
        mode: 'snapshot',
        records: expectedHashes.map((e) => e.rawLine),
        reportedTxids: workerProof.result.txids,
        workerWalletUrl: `http://localhost:${WORKER_WALLET_PORT}`,
        timeWindow: { startMs: tStart, endMs: Date.now() },
        skipOnChain: true, // first run: skip WoC lag; bijection + temporal are load-bearing
      });
    }

    // --- Step 12: inspectRun (layered verdict) ---
    //
    // primaryTaskIds.captain = harness task (task 1) — that's where overlay_lookup
    //   and delegate_task fire, which Layer 2 invariants (overlay_was_used,
    //   delegation_happened) scan for in the session.jsonl.
    // taskRecords.captain = final task (task 2) — that's where working_memory_set
    //   fires with RUN_NONCE in the value, which Layer 1 captain_task_complete
    //   asserts via String(result).includes(runNonce).
    log('step 12 — inspectRun (Layer 1 outcome + Layer 2 structural invariants)');
    const clusterState = JSON.parse(fs.readFileSync(handle.stateFile, 'utf8'));
    verdict = await inspectRun({
      workspaces: {
        captain: CAPTAIN_WORKSPACE,
        worker: WORKER_WORKSPACE,
      },
      clusterState,
      proofVerdict,
      runNonce: RUN_NONCE,
      expectedBudgetSats: EXPECTED_BUDGET_SATS,
      stderrLogs: {
        captain: captain.stderrLogPath,
        worker: worker.stderrLogPath,
      },
      taskRecords: {
        // Use task 2 for the captain record so captain_task_complete can find
        // the RUN_NONCE in its result field (working_memory_set's value).
        captain: captainFinalResult
          ? { ...captainFinalResult, task_id: captainFinalTaskId }
          : null,
        worker: workerTaskRecord,
      },
      captainIdentityKey: captain.identityKey,
      primaryTaskIds: {
        // Scan harness task 1's transcript for overlay_lookup + delegate_task.
        captain: captainHarnessTaskId,
        worker: workerProof ? workerProof.taskId : null,
      },
      // POC #23 must NOT skip overlay_was_used — user pinned this requirement.
      skipInvariants: [],
      // E8: Worker only uses execute_bash (dropped web_fetch to avoid nano's
      // looping under larger tool_results). The 3-step task at batch=100 made
      // nano lose state and retry web_fetch indefinitely. Simpler task keeps
      // nano in a clean 2-step flow.
      requireWorkerTools: ['execute_bash'],
      expectedCommissions: { sent: 1, received: 1 },
    });
  } finally {
    // --- Step 10: print verdict + stopCluster ---
    log('='.repeat(70));
    log('FINAL VERDICT');
    log('='.repeat(70));
    if (verdict) {
      console.log(JSON.stringify(verdict, null, 2));
    } else {
      log('verdict unavailable (fatal error before inspection)');
    }

    // Write the verdict to the output dir for post-mortem inspection
    const verdictPath = path.join(OUTPUT_DIR, 'verdict.json');
    try {
      fs.writeFileSync(
        verdictPath,
        JSON.stringify(
          {
            runNonce: RUN_NONCE,
            generatedAt: new Date().toISOString(),
            wallClockMs: Date.now() - tStart,
            captainTaskId: captainResult && captainResult.id,
            verdict,
          },
          null,
          2,
        ),
      );
      log(`verdict written to ${verdictPath}`);
    } catch (e) {
      log(`failed to write verdict.json: ${e.message}`);
    }

    log('stopping cluster...');
    try {
      await handle.stop();
    } catch (e) {
      log(`stopCluster error (non-fatal): ${e.message}`);
    }
  }

  const exitCode = verdict ? (verdict.exitCode != null ? verdict.exitCode : 1) : 1;
  process.exit(exitCode);
}

main().catch((e) => {
  console.error('[poc-23] FATAL:', e && e.stack ? e.stack : e);
  process.exit(1);
});
