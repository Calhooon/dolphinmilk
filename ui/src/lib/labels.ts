/**
 * Business-language translations for developer-facing terms.
 * Used across audit, dashboard, budget, and proof views.
 */

/** Human-readable labels for event types shown in audit views. */
export const EVENT_LABELS: Record<string, string> = {
  think_request: 'AI Request',
  think_response: 'AI Decision',
  tool_call: 'Action Started',
  tool_result: 'Action Result',
  proof_created: 'Record Created',
  checkpoint_created: 'Checkpoint Saved',
  receipt_stored: 'Payment Receipt',
  budget_check: 'Budget Check',
  error: 'Error',
  user: 'User Input',
  system: 'System',
  session_start: 'Session Started',
  session_end: 'Session Ended',
  continuation_save: 'Paused',
  continuation_resume: 'Resumed',
  loop_warning: 'Warning',
  memory_stored: 'Memory Saved',
};

/** Explanatory tooltips for event types. */
export const EVENT_DESCRIPTIONS: Record<string, string> = {
  think_request: 'A request was sent to the AI model for processing.',
  think_response: 'The AI processed your request and made a decision. Cost shown is the inference fee.',
  tool_call: 'The agent executed an action — like running a command, reading a file, or calling an external service.',
  tool_result: 'The result of an action the agent took.',
  proof_created: 'A cryptographic proof was written to the blockchain — an immutable, tamper-proof record.',
  budget_check: 'The agent checked its remaining budget before proceeding.',
  checkpoint_created: 'A state checkpoint was saved on-chain — enables crash recovery and session continuity.',
  receipt_stored: 'A payment receipt (BEEF format) was stored for this transaction.',
  memory_stored: 'The agent stored a memory for future reference.',
};

/** Human-readable proof type labels with descriptions. */
export const PROOF_LABELS: Record<string, { label: string; description: string }> = {
  Decision: { label: 'Decision Record', description: 'Cryptographic proof of what the agent decided and why. Includes model used, cost, and reasoning hash.' },
  TaskCompletion: { label: 'Task Complete', description: 'Proof that a task finished — records iterations, total cost, and final status.' },
  BudgetSnapshot: { label: 'Budget Snapshot', description: 'On-chain record of the budget state at task completion.' },
  MemoryCommitment: { label: 'Memory Record', description: 'Proof that the agent stored a memory. Content-addressed for integrity verification.' },
  CapabilityProof: { label: 'Capability Record', description: 'Proof that the agent used a specific tool or service. Records what was accessed and what it cost.' },
  Checkpoint: { label: 'State Checkpoint', description: 'On-chain state snapshot for session recovery. Includes task context and continuation data.' },
};

/** Budget gauge explanations. */
export const BUDGET_HELP: Record<string, string> = {
  'This Task': 'Maximum the agent can spend on a single task. When reached, the task pauses and asks for approval.',
  'Last Hour': 'Rolling 60-minute spending cap across all tasks. Resets continuously — not on the clock hour.',
  'Last 24 Hours': 'Rolling 24-hour spending cap. Resets continuously — not at midnight.',
  'Last 7 Days': 'Rolling 7-day spending cap. Enforced across all tasks.',
  'Last 30 Days': 'Rolling 30-day spending cap. Monthly budget guardrail.',
  'Lifetime': 'Total cumulative spending cap. Never resets.',
};

/** Section-level help text for dashboard and page areas. */
export const SECTION_HELP: Record<string, string> = {
  'Budget Limits': 'Three tiers of spending control. The agent self-regulates: it enters low-power mode near limits and pauses when exhausted.',
  'Spending': 'Breakdown of where money went. Each service (AI inference, image generation, storage) is tracked independently.',
  'Recent Activity': 'Live feed of the last 5 agent actions across all tasks. Click any entry for full details.',
  'Top Tasks by Cost': 'Most expensive tasks ranked by total spend. Click to see the full audit trail.',
  'Proof Chain': 'Visual representation of the cryptographic proof chain. Green links = verified integrity. Red = chain break detected.',
};
