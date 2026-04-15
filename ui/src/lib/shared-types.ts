/**
 * Canonical interfaces shared across UI components.
 * Single source of truth — no duplicate definitions elsewhere.
 */

// --- Task ---

export interface TaskSummary {
  id: string;
  task: string;
  status: string;
  iterations: number;
  sats_spent: number;
  started_at: string;
  completed_at?: string;
  model?: string;
  proof_txids?: string[];
  session_id?: string;
  result?: string;
  error?: string;
  tokens?: number;
  origin?: string;
  /** Conversation this task belongs to (null for orphan tasks). */
  conversation_id?: string | null;
}

// --- Agent ---

export interface AgentInfo {
  identity_key: string;
  balance: number;
  version: string;
  uptime_secs: number;
  total_tasks: number;
  total_sats_spent: number;
  tools: string[];
  certificate_status?: string;
  certificate_revoked?: boolean;
}

// --- Budget ---

export interface BudgetReport {
  task_sats: number;
  total_operations: number;
  services: Record<string, { total_sats: number; count: number }>;
  hourly_sats: number;
  daily_sats: number;
  weekly_sats: number;
  monthly_sats: number;
  lifetime_sats: number;
  limits: {
    max_per_task: number;
    max_per_hour: number;
    max_per_day: number;
    max_per_week: number;
    max_per_month: number;
    max_lifetime: number;
    task_remaining: number;
    hourly_remaining: number;
    daily_remaining: number;
    weekly_remaining: number;
    monthly_remaining: number;
    lifetime_remaining: number;
    enforcement: string;
  };
  balance: number;
}

export interface OperationStats {
  count: number;
  total_sats: number;
  avg_sats: number;
}

export interface ServiceBreakdown {
  total_sats: number;
  operations: Record<string, OperationStats>;
}

export interface SpendingEntry {
  timestamp: string;
  service: string;
  sats: number;
  operation?: string;
}

export interface BudgetDetailResponse {
  task_id?: string;
  entries: SpendingEntry[];
  by_service: Record<string, ServiceBreakdown>;
  total_sats: number;
}

export interface BudgetGaugeData {
  label: string;
  spent: number;
  limit: number;
  remaining: number;
}

export interface SpendingExportRow {
  date: string;
  taskId: string;
  service: string;
  operation: string;
  sats: number;
  usd: string;
  model: string;
  tokens: number | null;
  proofTxid: string;
}

export interface GlobalAuditEvent {
  timestamp: number;
  event_type: string;
  id: string;
  data: Record<string, unknown>;
  task_id: string;
  task_text: string;
  task_status: string;
}

// --- Receipts ---

export interface ReceiptDetail {
  iteration: number;
  model: string;
  sats_paid: number;
  sats_effective: number;
  sats_refunded: number;
  tokens: number;
  timestamp: string;
}

// --- Proofs ---

export interface ProofsResponse {
  task_id: string;
  /** BRC-18 OP_RETURN proof chain — hash-linked, immutable, auditable. */
  proofs: ProofDetail[];
  /** BRC-48 state tokens — spendable UTXOs (TaskCommitment, BudgetAllocation, etc.). */
  checkpoints: ProofDetail[];
}

export interface ProofDetail {
  txid: string;
  proof_type: string;
  hash: string;
  timestamp: number;
  proof_data?: string;
  proof_timestamp?: string;
  checkpoint_data?: unknown;
  sats_cost?: number;
  basket?: string;
  iteration?: number;
  prev_hash?: string;
}

// --- Schedule ---

export interface Schedule {
  id: string;
  description: string;
  enabled: boolean;
  schedule_type?: string;       // "interval" | "cron" | "once"
  interval_secs: number;
  cron_expression?: string;
  next_run?: string;
  last_run?: string;
  run_count: number;
  one_shot?: boolean;
  conversation_id?: string;
  created_at?: string;
  created_by?: string;
  run_history?: ScheduleRun[];
}

export interface ScheduleRun {
  task_id: string;
  started_at: string;
  completed_at?: string;
  status: string;
  sats_spent: number;
  iterations: number;
}

// --- Memory ---

export interface MemoryListItem {
  id: string;
  category: string;
  tags: string[];
  created: string;
  source: string;
  content_preview: string;
}

export interface MemoryListResponse {
  entries: MemoryListItem[];
  total: number;
  categories: Record<string, number>;
}

export interface MemoryDetail {
  id: string;
  category: string;
  content: string;
  tags: string[];
  created: string;
  source: string;
}

export interface MemorySearchResult {
  id: string;
  category: string;
  content: string;
  score: number;
  tags: string[];
  created: string;
}

// --- Certificates ---

export interface CertificateFields {
  name?: string;
  capabilities?: string;
  deployed_at?: string;
  version?: string;
  budget_per_task?: string;
  budget_per_hour?: string;
  budget_per_day?: string;
}

export interface CertificateStatus {
  status: 'parent-signed' | 'self-signed' | 'none';
  certificate?: Record<string, unknown>;
  identity_key?: string;
  is_revoked?: boolean | null;
}

// --- Conversations ---

/** Conversation summary from GET /conversations. */
export interface ConversationSummary {
  id: string;
  title: string;
  updated_at: string;
  message_count: number;
  total_sats: number;
  task_ids: string[];
}

/** Conversation detail from GET /conversations/{id}. */
export interface ConversationDetail {
  conversation: ConversationSummary;
  messages: ConversationMessage[];
  summary: ConversationSummaryStats;
}

/** Aggregated statistics for a conversation. */
export interface ConversationSummaryStats {
  total_iterations: number;
  total_proofs: number;
  total_artifacts: number;
  duration_secs: number;
  tasks: ConversationTaskDetail[];
  cost_by_service: Record<string, number>;
  models_used: string[];
}

/** Per-task detail within a conversation summary. */
export interface ConversationTaskDetail {
  id: string;
  status: string;
  task: string;
  iterations: number;
  sats_spent: number;
  proof_count: number;
  artifact_count: number;
  started_at?: string;
  completed_at?: string;
}

/** A single message in a conversation's hash chain. */
export interface ConversationMessage {
  seq: number;
  role: string;
  content: string;
  hash: string;
  ts: number;
  task_id?: string;
  tool_calls?: any[];
  tool_call_id?: string;
  name?: string;
}

/** Per-message hash chain verification result. */
export interface MessageVerification {
  seq: number;
  role: string;
  valid: boolean;
  hash: string;
  expected_hash: string;
  prev_hash_valid: boolean;
  timestamp: number;
}

/** Full chain verification result from GET /conversations/{id}/verify. */
export interface ChainVerification {
  valid: boolean;
  message_count: number;
  head_hash: string;
  genesis_hash: string;
  messages: MessageVerification[];
}

// --- Audit ---

export interface AuditSummary {
  total_events: number;
  iterations: number;
  sats_spent: number;
  proof_count: number;
  tool_calls: number;
  duration_secs: number;
}

/** Single audit event from GET /task/{id}/audit — matches server AuditEvent struct. */
export interface AuditEvent {
  timestamp: number;
  event_type: string;
  data: Record<string, unknown>;
}

export interface AuditConversationMessage {
  role: string;
  content: string;
  tool_calls?: Array<{
    id: string;
    function: { name: string; arguments: string };
  }>;
  tool_call_id?: string;
}

// --- Proofs (extended) ---

export interface ReceiptsResponse {
  task_id: string;
  receipts: ReceiptDetail[];
  total_sats_paid: number;
  total_sats_refunded: number;
}

export type VerifyStatus = 'pending' | 'match' | 'mismatch';

export interface OnChainVerification {
  txid: string;
  hash_recompute: string;
  on_chain_match: string;
  on_chain_hash?: string;
  recomputed_hash?: string;
}

// --- Status helpers ---

export type StatusClass = 'running' | 'complete' | 'error' | 'pending';

export function getStatusClass(status: string): StatusClass {
  const s = status.toLowerCase();
  if (s === 'running' || s === 'in_progress') return 'running';
  if (s === 'complete' || s === 'completed' || s === 'done') return 'complete';
  if (s === 'error' || s === 'failed') return 'error';
  return 'pending';
}
