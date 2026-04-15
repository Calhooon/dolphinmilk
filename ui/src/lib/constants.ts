/**
 * Shared constants used across multiple UI components.
 * Single source of truth — import from here, don't duplicate.
 */

/** Human-readable names for x402 service keys. */
export const SERVICE_NAMES: Record<string, string> = {
  'llm': 'LLM Inference',
  'proofs': 'On-Chain Proofs',
  'openai-chat': 'OpenAI Chat',
  'claude-chat': 'Claude Chat',
  'nano-banana-pro': 'Image Gen (Banana)',
  'banana': 'Image Generation',
  'veo-3-1-fast': 'Video Gen (Veo)',
  'veo': 'Video Generation',
  'whisper-large-v3-turbo': 'Transcription (Whisper)',
  'whisper': 'Transcription',
  'x-research': 'X/Twitter Search',
  '1sat': 'Inscriptions (1Sat)',
  'nanostore': 'NanoStore',
  'messagebox': 'MessageBox',
  'kling': 'Video Gen (Kling)',
  'polymirror': 'PolyMirror',
};

/** Colors for per-service bars (cycled by index). */
export const SERVICE_COLORS = [
  '#14A8C4', '#fcbe2d', '#00b69b', '#a78bfa',
  '#f472b6', '#fb923c', '#38bdf8', '#facc15',
];

/** Human-readable labels for proof types. */
export const PROOF_TYPE_LABELS: Record<string, string> = {
  decision: 'Agent Decision',
  task_completion: 'Task Complete',
  budget_snapshot: 'Budget Record',
  checkpoint: 'Session Checkpoint',
  capability_proof: 'Tool Usage',
  capability_declaration: 'Capabilities',
  memory_commitment: 'Memory Saved',
};

/** CSS color per proof type. */
export const TYPE_COLORS: Record<string, string> = {
  decision: 'var(--accent, #14A8C4)',
  task_completion: 'var(--success, #00b69b)',
  budget_snapshot: 'var(--warning, #fcbe2d)',
  capability_proof: 'var(--success, #00b69b)',
  memory_commitment: '#a78bfa',
  checkpoint: 'var(--text-bright, #fff)',
};

/** Unicode shape per proof type (used in chain visualization). */
export const TYPE_SHAPES: Record<string, string> = {
  decision: '\u25CF',           // filled circle
  task_completion: '\u25A0',    // filled square
  budget_snapshot: '\u25C6',    // filled diamond
  capability_proof: '#',
  memory_commitment: '@',
  checkpoint: '\u25B2',         // triangle
};

/** Convert a service key to a human-readable name. */
export function friendlyName(service: string): string {
  return SERVICE_NAMES[service] ?? service;
}
