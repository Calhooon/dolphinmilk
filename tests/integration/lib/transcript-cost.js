/**
 * Read the true cost of a task from its session.jsonl transcript.
 *
 * The wallet balance diff (sats_paid) includes the upfront x402 payment
 * before refunds are internalized. The transcript records sats_effective
 * (after refund) which is the real cost. This module reads that truth.
 *
 * Cost = sum(think_response.sats_effective) + sum(tool_result.sats_paid)
 *      + sum(proof_created.sats_cost) + sum(checkpoint_created from budget)
 */

const fs = require('fs');
const path = require('path');

// Search paths for the workspace tasks directory.
// The server may use data_dir (~/.bsv-worm/workspace) or project-root working/.
const WORKSPACE_CANDIDATES = [
  path.join(__dirname, '..', '..', '..', 'working', 'tasks'),       // project root: working/tasks/
  path.join(process.env.HOME || '', '.bsv-worm', 'workspace', 'tasks'), // data_dir default
  path.join(process.env.HOME || '', '.dolphin-milk', 'workspace', 'tasks'), // renamed data_dir
];

/**
 * Find the tasks directory that actually has task subdirectories.
 */
function findTasksDir() {
  for (const candidate of WORKSPACE_CANDIDATES) {
    try {
      if (fs.existsSync(candidate) && fs.readdirSync(candidate).length > 0) {
        return candidate;
      }
    } catch { /* skip */ }
  }
  return null;
}

/**
 * Read the true effective cost from a task's transcript.
 *
 * @param {string} taskId - Full task UUID
 * @returns {{ effectiveSats: number, paidSats: number, refundedSats: number, thinkCalls: number, toolCostSats: number, proofCostSats: number, found: boolean }}
 */
function getTranscriptCost(taskId) {
  const result = {
    effectiveSats: 0,    // Real cost after refunds (sats_effective)
    paidSats: 0,         // Gross payment (sats_paid to x402)
    refundedSats: 0,     // Total refunded
    thinkCalls: 0,       // Number of LLM calls
    toolCostSats: 0,     // Cost of paid tool calls (x402_call, etc.)
    proofCostSats: 0,    // Cost of on-chain proofs
    found: false,
  };

  const tasksDir = findTasksDir();
  if (!tasksDir) return result;

  // Find the task directory — match by full ID or prefix
  const prefix = taskId.substring(0, 8);
  const dirs = fs.readdirSync(tasksDir).filter(d => d === taskId || d.startsWith(prefix));
  if (dirs.length === 0) return result;

  const sessionPath = path.join(tasksDir, dirs[0], 'session.jsonl');
  if (!fs.existsSync(sessionPath)) return result;

  result.found = true;

  const lines = fs.readFileSync(sessionPath, 'utf8').trim().split('\n');
  for (const line of lines) {
    if (!line.trim()) continue;
    try {
      const event = JSON.parse(line);

      if (event.type === 'think_response') {
        result.thinkCalls++;
        result.effectiveSats += event.sats_effective || 0;
        result.paidSats += event.sats_paid || 0;
        result.refundedSats += event.sats_refunded || 0;
      }

      if (event.type === 'tool_result' && event.sats_paid) {
        result.toolCostSats += event.sats_paid;
        result.effectiveSats += event.sats_paid;
      }

      if (event.type === 'proof_created' && event.sats_cost) {
        result.proofCostSats += event.sats_cost;
        result.effectiveSats += event.sats_cost;
      }

      // session_end has the authoritative total — use it if available
      if (event.type === 'session_end' && event.sats_spent) {
        result.effectiveSats = event.sats_spent;
      }
    } catch { /* skip malformed lines */ }
  }

  return result;
}

module.exports = { getTranscriptCost, findTasksDir };
