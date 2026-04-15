#!/usr/bin/env node
// tests/multi-worm/test_inspector_calibration.js
//
// Fixture-based calibration test for lib/inspector.js.
//
// Loads the most recent green cascade's real transcripts (committed
// 09b7eb8 "multi-worm: proof batch cost POC (DolphinMilkShake #23)") and
// asserts that every Layer 1 + Layer 2 predicate passes on known-green
// data. This is the "does the inspector actually recognize known-green
// runs as green" gate. If this test fails, a predicate is broken and
// must be fixed before proceeding.
//
// The cascade does NOT use overlay_lookup (overlay_was_used is skipped
// via input.skipInvariants) and does NOT use execute_bash on the worker
// (worker required tools default to ['web_fetch']). The POC #23 harness
// that will call inspectRun for the real gate should NOT skip those.

'use strict';

const assert = require('assert').strict;
const path = require('path');
const { inspectRun } = require('./lib/inspector');

// Real task IDs from the committed green run 09b7eb8 / Agent B's report.
const CASCADE_CAPTAIN_WS = path.resolve(__dirname, '../../test-workspaces/cascade-captain');
const CASCADE_WORKER_WS = path.resolve(__dirname, '../../test-workspaces/cascade-worker');
const CAPTAIN_FORWARD_TASK_ID = 'd2880303-4283-45fd-ac3b-998fdf9b7a98'; // captain -> coordinator delegate_task
const WORKER_TASK_ID = '06198a62-f314-422d-9e40-1f3758e2ab5e'; // worker web_fetch + reverse delegate_task

// Captain identity key from cascade logs (address that working_memory_set was routed to).
// Matches the recipient of the worker's reverse delegate_task (to coordinator 0350cf02...),
// so for calibration we do NOT pin captainIdentityKey — we let reverse_path_existed just
// check that a reverse call happened. That matches the "or just assert the call happened"
// language in the spec.

const RUN_NONCE = 'cascade_summary_91a93b04'; // this string appears in the captain session_end.result

// Synthetic proof verdict — the cascade run predates proof_verify.js, so we
// feed a PASS verdict to exercise Layer 1 proof_bijection / wallet_tx_delta
// branches. Real inspector runs will get this from proof_verify.js.
const syntheticProofVerdict = {
  verdict: 'PASS',
  mode: 'snapshot',
  expectedCount: 0,
  actualCount: 0,
  verifiedCount: 0,
  bijection: { status: 'PASS', orphans: [], misses: [], duplicates: [] },
  onChain: { status: 'SKIPPED', verifiedCount: 0, pendingCount: 0, failed: [] },
  temporal: { status: 'PASS', windowStart: '', windowEnd: '', outOfWindow: [] },
};

// Synthetic taskRecords: the real captain transcript's session_end has
// result="**Success.** The one-sentence summary has been recorded to
// working memory under key `cascade_summary_91a93b04`. ..."
const syntheticTaskRecords = {
  captain: {
    task_id: 'f55ae493-35ee-49eb-ab07-037b4763e395', // Captain's final recording task
    status: 'complete',
    error: null,
    result:
      '**Success.** The one-sentence summary has been recorded to working memory under key `cascade_summary_91a93b04`. The cascade is complete. LOBSTER-VERIFIED',
  },
  worker: {
    task_id: WORKER_TASK_ID,
    status: 'complete',
    error: null,
    result: 'worker done',
  },
};

async function main() {
  console.log('[calibration] starting');
  console.log(`[calibration] captain workspace: ${CASCADE_CAPTAIN_WS}`);
  console.log(`[calibration] worker  workspace: ${CASCADE_WORKER_WS}`);

  const input = {
    workspaces: {
      // Use the coordinator's layer as "captain" for the purposes of the
      // calibration because the captain-forward-delegate task lives in
      // cascade-captain/tasks/d2880303-... — we pick it via primaryTaskIds.
      captain: CASCADE_CAPTAIN_WS,
      worker: CASCADE_WORKER_WS,
    },
    primaryTaskIds: {
      captain: CAPTAIN_FORWARD_TASK_ID,
      worker: WORKER_TASK_ID,
    },
    clusterState: {
      parentKey: '03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0',
      agents: {
        captain: { name: 'captain' },
        worker: { name: 'worker' },
      },
    },
    proofVerdict: syntheticProofVerdict,
    runNonce: RUN_NONCE,
    expectedBudgetSats: 2_500_000, // headroom over the cascade's ~2.04M baseline
    stderrLogs: {
      captain: path.join(CASCADE_CAPTAIN_WS, 'server-stderr.log'),
      worker: path.join(CASCADE_WORKER_WS, 'server-stderr.log'),
    },
    taskRecords: syntheticTaskRecords,
    // The cascade test does not use overlay_lookup, and captain does not
    // need captainIdentityKey pinned (reverse_path_existed accepts any
    // reverse call when the key is omitted).
    skipInvariants: ['overlay_was_used'],
    // Commission thresholds: cascade fires ~4 sent + ~4 received across
    // both captain and worker stderr. Use 1/1 to be a lower bound.
    expectedCommissions: { sent: 1, received: 1 },
    // Worker required tools — cascade uses web_fetch only (no execute_bash).
    requireWorkerTools: ['web_fetch'],
  };

  const verdict = await inspectRun(input);

  console.log('[calibration] inspector verdict:');
  console.log(JSON.stringify(verdict, null, 2));

  // Assertions
  let failures = 0;

  for (const check of verdict.layer1.checks) {
    const name = check.name;
    if (check.status !== 'PASS') {
      console.error(`[calibration] LAYER 1 FAIL: ${name} — ${check.reasoning}`);
      failures++;
    } else {
      console.log(`[calibration] L1 PASS: ${name}`);
    }
  }

  for (const inv of verdict.layer2.invariants) {
    const name = inv.name;
    if (inv.status === 'SKIP') {
      console.log(`[calibration] L2 SKIP: ${name}`);
      continue;
    }
    if (inv.status !== 'PASS') {
      console.error(`[calibration] LAYER 2 FAIL: ${name} — ${inv.reasoning}`);
      failures++;
    } else {
      console.log(`[calibration] L2 PASS: ${name}`);
    }
  }

  assert.strictEqual(verdict.layer3.status, 'DEFERRED', 'Layer 3 must be DEFERRED in first pass');

  if (failures > 0) {
    console.error(`[calibration] FAILED: ${failures} predicate(s) did not pass on known-green data`);
    process.exit(1);
  }

  assert.strictEqual(
    verdict.verdict,
    'PASS',
    `overall verdict should be PASS on known-green cascade (got ${verdict.verdict})`,
  );

  console.log('[calibration] ALL Layer 1 checks + Layer 2 invariants passed on known-green cascade');
  console.log(
    `[calibration] Summary: verdict=${verdict.verdict} layer1=${verdict.layer1.status} layer2=${verdict.layer2.status} satsSpent=${verdict.totals.satsSpent}`,
  );
  process.exit(0);
}

main().catch((e) => {
  console.error('[calibration] FAILED with error:', e);
  process.exit(1);
});
