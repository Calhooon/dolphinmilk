# Multi-Worm Test Infra Interface Contracts

> Single source of truth for the shapes every module in `tests/multi-worm/lib/` produces and consumes. Subagents building cluster.js, proof_verify.js, and inspector.js MUST conform to these interfaces so the integration step in test_poc_23.js is a no-op. Living spec — update this file in lockstep with the code. Created 2026-04-13 for DolphinMilkShake #23 quality gate. See `~/bsv/dolphinmilkshake/QUALITY-GATE-PLAN.md` for the strategic context.

## Conventions

- All modules are CommonJS (`module.exports = { ... }`) to match the existing multi-worm harness style. No ESM.
- All async functions return Promises. No callbacks.
- Timestamps in emitted JSON are ISO-8601 strings (`new Date().toISOString()`). Internal Date math uses `Date.now()` / milliseconds.
- Error handling: throw `Error` subclasses with descriptive messages. No silent failures. Don't swallow exceptions into "return null" unless a specific interface says so.
- Logging: use `console.log` for progress, `console.error` for failures. No external logging deps. Prefix each module's logs with `[cluster]`, `[proof]`, `[inspector]` for readability.
- BRC-31 auth: use the existing `lib/auth.js` helpers (`authGet`, `authPost`, `clearAuthCache`). Never re-implement auth.
- File paths in emitted JSON are absolute.
- Identity keys are 66-char hex strings (compressed secp256k1 pubkey).
- Txids are 64-char hex strings.
- Monetary values are integer sats (never floats, never USD).

---

## `lib/cluster.js`

### Exports

```js
module.exports = {
  startCluster,
  stopCluster,
};
```

### `startCluster(config: ClusterConfig): Promise<ClusterHandle>`

Brings up a multi-agent cluster. Each step is a hard gate — throw with a clear message on failure.

#### ClusterConfig

```ts
{
  parentWalletPort: number;          // default 3321 (MetaNet Client)
  binary: string;                    // absolute path to dolphin-milk binary
  overlay: {
    url: string;                     // e.g., 'https://rust-overlay.dev-a3e.workers.dev'
    verifyRegistration: boolean;     // if true, step 6 is enforced; if false, skipped
    registrationTimeoutMs: number;   // default 30000
  };
  outputDir: string;                 // directory where cluster-state.json will be written
  agents: Array<{
    name: string;                    // unique within the cluster, e.g., 'captain'
    port: number;                    // dolphin-milk HTTP port
    walletPort: number;              // bsv-wallet HTTP port
    model: string;                   // e.g., 'claude-haiku-4-5'
    workspace: string;               // absolute path to worm workspace dir
    capabilities: string[];          // role capabilities to declare via cert
    env?: Record<string, string>;    // optional extra env vars
  }>;
  healthTimeoutMs?: number;          // default 90000 (matches cascade test)
}
```

#### Startup sequence

Each step is a hard gate — fail fast with a clear error if any gate fails.

1. **Binary check**: `fs.existsSync(config.binary)` — throw with "Run `cargo build --release` first" if missing.
2. **Parent wallet probe**: `authGet` `http://localhost:${parentWalletPort}/agent` via BRC-31 (or fall back to direct wallet `getPublicKey` if the parent is a bare wallet). Resolve the parent identity key. Throw if unreachable.
3. **Per-agent health + spawn**: for each agent in parallel:
   - `httpGet http://localhost:${agent.port}/health`. If reachable with `{status:'ok', wallet_connected:true}` AND the returned identity matches the agent's expected wallet → reuse, mark `spawnedByUs=false`.
   - Otherwise: `spawn(binary, ['serve', '--port', String(port), '--workspace', workspace], { env: {...}, stdio })` with the env block documented below. Poll `/health` until healthy or `healthTimeoutMs` elapses. Mark `spawnedByUs=true`.
4. **Identity verification**: for each agent, `authGet /agent` and independently query the agent's wallet's `getPublicKey`. Assert they match. Throws on mismatch — catches "spawned with wrong wallet" silently.
5. **Cert audit + per-role issuance**: `authGet /certificates` on each agent. If a cert from the parent exists AND its `capabilities` array matches the declared set (exact superset — declared capabilities must all be present), reuse it. Otherwise issue a new cert via the endpoint Agent A investigation documents. Re-verify after issuance.
6. **Overlay registration verification**: for each agent's declared capability, poll `GET ${overlay.url}/lookup?service=ls_agent&findByCapability=${cap}` (exact endpoint shape per overlay docs) until the agent's identity key appears in the result set, with exponential backoff up to `registrationTimeoutMs`. Throw if registration never appears. If `overlay.verifyRegistration === false`, skip this step.
7. **State file emit**: write `${outputDir}/cluster-state.json` per the shape below. Ensure `outputDir` exists first.
8. **Return ClusterHandle**.

#### Env block for spawned agents

Mirrors the cascade test's existing env exactly, plus agent-specific overrides. Specifically:

```js
{
  ...process.env,
  DOLPHIN_MILK_WALLET_URL: `http://localhost:${agent.walletPort}`,
  DOLPHIN_MILK_HEARTBEAT_ENABLED: 'true',
  DOLPHIN_MILK_HEARTBEAT_POLL_SECS: '15',
  DOLPHIN_MILK_OVERLAY_ENABLED: 'true',
  DOLPHIN_MILK_AGENT_NAME: agent.name,
  DOLPHIN_MILK_LOG_LEVEL: 'INFO',
  DOLPHIN_MILK_PARENT_KEY: parentKey,                            // resolved in step 2
  DOLPHIN_MILK_PARENT_WALLET_URL: `http://localhost:${parentWalletPort}`,
  DOLPHIN_MILK_TRUST_CERTIFIERS: parentKey,                      // trust our parent
  DOLPHIN_MILK_LLM_MODEL: agent.model,
  ...agent.env,                                                  // user overrides last
}
```

Agent A investigation may add env vars related to capability declaration if the worm supports that path. Add any new env vars here without changing the contract.

#### ClusterHandle

```ts
{
  agents: Map<string, AgentHandle>;
  stateFile: string;                   // absolute path to cluster-state.json
  parentKey: string;                   // parent identity key
  overlayUrl: string;
  createdAt: string;                   // ISO timestamp when startCluster completed
  stop(options?: { onlySpawned?: boolean }): Promise<void>; // shortcut to stopCluster
}
```

#### AgentHandle

```ts
{
  name: string;
  port: number;
  walletPort: number;
  identityKey: string;                 // 66-char hex, verified against wallet's getPublicKey
  workspace: string;                   // absolute path
  certHash: string;                    // hex hash of the issued BRC-52 cert
  certCapabilities: string[];          // actual capabilities on the issued cert
  overlayRegistered: boolean;          // true if step 6 passed or skipped
  overlayRegisteredAt: string | null;  // ISO timestamp if registered
  spawnedByUs: boolean;                // true if startCluster spawned it, false if reused
  proc: ChildProcess | null;           // null if reused, else the spawned process
  stdoutLogPath: string | null;        // absolute path to server-stdout.log
  stderrLogPath: string | null;        // absolute path to server-stderr.log
}
```

### `stopCluster(handle: ClusterHandle, options?: { onlySpawned?: boolean }): Promise<void>`

- Default `onlySpawned: true` — only stops agents with `spawnedByUs === true`.
- `onlySpawned: false` — force-stops every agent in the handle regardless.
- Graceful SIGTERM with 5s timeout, then SIGKILL. Same pattern as `stopAgent` in the cascade test.
- Safe to call multiple times (idempotent). Agents already stopped become no-ops.

### `cluster-state.json` shape

```json
{
  "createdAt": "2026-04-13T23:50:00.000Z",
  "parentKey": "03ef3231669022cc03aa...",
  "overlayUrl": "https://rust-overlay.dev-a3e.workers.dev",
  "agents": {
    "captain": {
      "name": "captain",
      "port": 8081,
      "walletPort": 3322,
      "identityKey": "034aa44668fbc73ca5d4...",
      "workspace": "/Users/johncalhoun/bsv/rust-bsv-worm/test-workspaces/cascade-captain",
      "certHash": "abcdef...",
      "certCapabilities": ["orchestration", "intelligence", "reporting"],
      "overlayRegistered": true,
      "overlayRegisteredAt": "2026-04-13T23:49:45.123Z",
      "spawnedByUs": true,
      "stdoutLogPath": "/Users/johncalhoun/bsv/rust-bsv-worm/test-workspaces/cascade-captain/server-stdout.log",
      "stderrLogPath": "/Users/johncalhoun/bsv/rust-bsv-worm/test-workspaces/cascade-captain/server-stderr.log"
    },
    "worker": { "... same shape ..." }
  }
}
```

---

## `lib/proof_verify.js`

### Exports

```js
module.exports = {
  verifyProofBatch,
  computeExpectedHash,   // exposed so test files can use the exact same fn
};
```

### `verifyProofBatch(input: ProofVerifyInput): Promise<ProofVerdict>`

Bijective verification. Independently computes expected hashes, extracts actual on-chain hashes, asserts 1:1 mapping.

#### ProofVerifyInput

```ts
{
  mode: 'snapshot' | 'live';
  records: any[];                    // ground-truth record objects
  reportedTxids: string[];           // txids Worker claimed it created
  workerWalletUrl: string;           // e.g., 'http://localhost:3324'
  workerWalletDbPath?: string;       // fallback for tx data if HTTP lookup fails
  timeWindow: { startMs: number; endMs: number }; // Date.now() at test start/end
  hashFn?: (record: any) => string;  // override hashing; default is computeExpectedHash
  chainCheckTimeoutMs?: number;      // default 120000 (2 min for WoC indexing)
  skipOnChain?: boolean;             // default false; if true, skip step 6
}
```

#### `computeExpectedHash(record: any): string`

The default hashing function. Must produce byte-identical results to `proof_records.sh`'s `echo -n "$record" | shasum -a 256`. Agent B investigation will determine the exact byte layout (likely `jq -c` compact output → hex sha256). If the shell script's behavior and `crypto.createHash('sha256').update(JSON.stringify(record)).digest('hex')` diverge, this function must match the shell script's behavior, not JavaScript's default. Document the exact bytes going into the hash.

#### Verification sequence

1. **Input validation**: `records.length === reportedTxids.length` (else FAIL with detail). All txids are 64-char hex.
2. **Expected hash set**: `records.map(hashFn)` → `Set<hex>`.
3. **On-chain hash set**: for each `txid` in `reportedTxids`, look up the tx via `POST ${workerWalletUrl}/listActions` (or whichever endpoint Agent B documents). Parse outputs, find OP_RETURN (`satoshis === 0`, script prefix `006a20`), extract 64-char hex after `006a20`. Build `actual: Set<hex>` and `txidToHash: Map<txid, hex>`. If HTTP lookup fails, fall back to `workerWalletDbPath` sqlite reads. Throw if both fail.
4. **Bijection**: compute `orphans = actual - expected`, `misses = expected - actual`, `duplicates = actual hashes appearing > once across txidToHash`. All must be empty for PASS.
5. **Temporal check**: for each tx, verify its `createdAt` (from wallet response) is within `[timeWindow.startMs, timeWindow.endMs + 60000]`. Report violations.
6. **On-chain broadcast check** (if `!skipOnChain`): for each txid, poll the worker wallet's `proven_txs` or the overlay/WoC until the tx has an attached merkle proof OR `chainCheckTimeoutMs` elapses. Record pending vs verified counts. Not a hard failure — timeout produces `onChain.status === 'PENDING'`, not FAIL (WoC indexing lag is a real state, not a bug).

#### ProofVerdict

```ts
{
  verdict: 'PASS' | 'FAIL';
  mode: 'snapshot' | 'live';
  expectedCount: number;
  actualCount: number;
  verifiedCount: number;              // records with a 1:1 match
  bijection: {
    status: 'PASS' | 'FAIL';
    orphans: string[];                // actual hashes without matching expected
    misses: string[];                 // expected hashes without matching actual
    duplicates: string[];             // hashes appearing > once
  };
  onChain: {
    status: 'PASS' | 'PENDING' | 'FAIL' | 'SKIPPED';
    verifiedCount: number;            // txs with attached merkle proofs
    pendingCount: number;             // txs without merkle proofs (indexing lag)
    failed: Array<{ txid: string; reason: string }>;
  };
  temporal: {
    status: 'PASS' | 'FAIL';
    windowStart: string;              // ISO
    windowEnd: string;                // ISO
    outOfWindow: Array<{ txid: string; timestamp: string }>;
  };
  evidence: {
    txidHashPairs: Array<{
      txid: string;
      hash: string;
      recordId?: string;              // if the record has an identifiable id field
      matched: boolean;
    }>;
    hashFnDescription: string;        // e.g., "sha256 over compact JSON (jq -c)"
  };
  totals: {
    satsSpentOnProofs: number;        // sum of (miner fee) across all proof txs
    wallClockMs: number;              // time spent in verification
  };
}
```

**Verdict rule**: `verdict === 'PASS'` iff `bijection.status === 'PASS'` AND `temporal.status === 'PASS'` AND `onChain.status !== 'FAIL'`. `onChain.status === 'PENDING'` does NOT fail the verdict.

---

## `lib/inspector.js`

### Exports

```js
module.exports = {
  inspectRun,
  readSessionJsonl,     // helper: parse a session.jsonl into an event array
  findEvents,           // helper: filter events by type/tool/predicate
};
```

### `inspectRun(input: InspectInput): Promise<InspectionVerdict>`

Layered verdict. Layer 1 (hard outcome) and Layer 2 (structural invariants) decide pass/fail. Layer 3 is deferred in the first pass.

#### InspectInput

```ts
{
  workspaces: Record<string, string>;   // { captain: '/abs/path', worker: '/abs/path' }
  clusterState: any;                     // cluster-state.json parsed
  proofVerdict: ProofVerdict;            // from proof_verify.js
  runNonce: string;                      // the unique identifier for this run
  expectedBudgetSats: number;            // for ±30% budget check
  stderrLogs: Record<string, string>;   // { captain: '/abs/path/server-stderr.log', ... }
  taskRecords: Record<string, any>;     // { captain: Captain's final /task/{id} response, ... }
}
```

#### Layer 1 — Hard outcome checks (decide verdict)

Each check is a binary assertion. `layer1.status = PASS` iff all checks PASS.

- **`proof_bijection`**: `input.proofVerdict.verdict === 'PASS'`. Delegates to proof_verify.js — inspector does not re-verify, just reads the verdict.
- **`wallet_tx_delta`**: Worker's reported `txids.length` equals `input.proofVerdict.expectedCount`. No off-by-one. Evidence: the delta and the expected count.
- **`captain_task_complete`**: `input.taskRecords.captain.status === 'complete'` AND `captain.error === null` AND `String(captain.result).includes(runNonce)`. Proves Captain processed Worker's reverse-delegation reply (not hallucinated).
- **`commission_settled`**: parse each agent's `server-stderr.log`. Count occurrences of `commission_payment_sent` and `commission_payment_received` (or equivalent log strings — Agent B confirms the exact log shape). For a 2-hop test (Captain ↔ Worker direct), expect ≥2 sent + ≥2 received. For a 3-hop cascade, expect ≥4 + ≥4. Parameterized via `expectedCommissions` if the test needs a specific count.
- **`budget_respected`**: sum `sats_effective` from each agent's transcripts (via `readSessionJsonl` + extract from `think` events), assert within `expectedBudgetSats * 1.3` upper bound. Per-agent caps also checked against cert budget limits if present in the cert.

#### Layer 2 — Structural invariants (decide verdict)

Each is a tiny substring-based predicate over the full event stream. `layer2.status = PASS` iff all invariants PASS.

- **`overlay_was_used`**: Captain's transcript has ≥1 `tool_call` event with tool name `overlay_lookup` (exact name TBD via Agent B; may be a variant) AND the `args` blob contains the substring `scraping` (case-insensitive, applied to `JSON.stringify(args).toLowerCase()`).
- **`delegation_happened`**: Captain's transcript has ≥1 `tool_call` with tool name `delegate_task` AND Worker's transcript has ≥1 corresponding `task_received` event (or equivalent — Agent B confirms the shape). The captain event's timestamp must precede the worker event's timestamp plus 60s tolerance. Cross-agent linkage can use `message_hash` or `recipient_key` if present.
- **`worker_did_external_work`**: Worker's transcript has ≥1 `tool_call` with tool name `web_fetch` whose corresponding `tool_result` does not start with `error` (case-insensitive), AND ≥1 `tool_call` with tool name `execute_bash` whose result does not start with `error`.
- **`reverse_path_existed`**: Worker's transcript has ≥1 `tool_call` with tool name `delegate_task` (or `send_message`, whichever the test's prompt uses for the reverse hop) with recipient matching the captain's identity key.
- **`commission_messages_fired`**: at least the expected number of commission-related log entries across `stderrLogs`. Exact string patterns per Agent B's investigation.

Each invariant produces:

```ts
{
  name: string;
  status: 'PASS' | 'FAIL';
  evidence: any;                    // e.g., matching event or stderr excerpt
  reasoning: string;                // human-readable one-liner
}
```

If a predicate throws an exception (e.g., bad JSON parse on an event), mark the invariant `status: 'FAIL'` with `reasoning: "predicate threw: ${error.message}"` and push to `warnings[]`. Do NOT let predicate bugs silently pass.

#### Layer 3 — deferred (first pass returns `status: 'DEFERRED'`)

```ts
layer3: { status: 'DEFERRED'; reason: 'Layer 3 not implemented in first pass (Phase 2). Add via follow-up once Layers 1+2 are green.' }
```

#### Verdict rule

- `PASS` = `layer1.status === 'PASS'` AND `layer2.status === 'PASS'`
- `FAIL` = `layer1.status === 'FAIL'` OR `layer2.status === 'FAIL'`
- `WARN` is not produced in the first pass (requires Layer 3). Exit code: 0 for PASS, 2 for FAIL.

#### InspectionVerdict

```ts
{
  verdict: 'PASS' | 'FAIL';
  exitCode: 0 | 2;
  runNonce: string;
  layer1: {
    status: 'PASS' | 'FAIL';
    checks: Array<{
      name: string;
      status: 'PASS' | 'FAIL';
      evidence: any;
      reasoning: string;
    }>;
  };
  layer2: {
    status: 'PASS' | 'FAIL';
    invariants: Array<{
      name: string;
      status: 'PASS' | 'FAIL';
      evidence: any;
      reasoning: string;
    }>;
  };
  layer3: {
    status: 'DEFERRED';
    reason: string;
  };
  totals: {
    satsSpent: number;
    txCount: number;
    wallClockMs: number;
    perAgentIters: Record<string, number>;
  };
  warnings: string[];
  generatedAt: string;                // ISO
}
```

### Helper: `readSessionJsonl(path: string): Promise<Event[]>`

Parse a `session.jsonl` file. Each line is a JSON object. Skip blank lines. Parse errors push to a `parseErrors[]` array on the return value, but don't throw — return what we can. The events array preserves file order.

Return shape:
```ts
{
  events: Event[];                    // parsed events in file order
  parseErrors: Array<{ line: number; error: string }>;
  path: string;
}
```

An `Event` has an `event_type` string field and arbitrary other data (shape varies by type). Agent B documents the real shapes during investigation.

### Helper: `findEvents(events: Event[], predicate: (e: Event) => boolean): Event[]`

Simple filter. Returns all matching events in order. Trivial to implement but having it as a named helper keeps invariant predicates readable.

---

## Integration contract: `test_poc_23.js`

Top-level test. Thin wiring (~150 lines max). Imports all three modules plus existing `lib/auth.js`. Flow:

1. Parse CLI args (none required for default run).
2. Compute run nonce: `crypto.randomBytes(4).toString('hex')`.
3. `startCluster({ ... Captain + Worker ... })`.
4. Snapshot Reddit: fetch `https://www.reddit.com/r/technology/hot.json?limit=10`, write the JSON to `cluster.outputDir/reddit-snapshot.json`, keep the parsed records in memory for proof_verify.js.
5. Write `proof_records.sh` to `cluster.agents.get('worker').workspace/proof_records.sh` with the exact shell script from `test_proof_batch_cost.js`.
6. Submit the task to Captain via `authPost /task`. Task text instructs: `overlay_lookup('scraping')` → `delegate_task(worker, capabilities=[web_fetch, execute_bash, delegate_task])` → Worker fetches Reddit, runs `proof_records.sh`, reverse-delegates results to Captain with RUN_NONCE, Captain `working_memory_set`s the result.
7. Poll Captain's task until complete or timeout (10 min).
8. Wait ~3 min for commission settlement (2-hop, slightly shorter than cascade's 4 min).
9. Read Captain's and Worker's final task records via `authGet /task/{id}`.
10. Call `proof_verify.verifyProofBatch({ mode: 'snapshot', records, reportedTxids, ... })`.
11. Call `inspector.inspectRun({ ... })` with the proof verdict, task records, stderr paths.
12. Print the verdict.
13. `cluster.stop()` — only stops spawned processes.
14. `process.exit(inspectionVerdict.exitCode)`.

No other logic. If you find yourself adding non-trivial code to test_poc_23.js, it belongs in one of the three modules instead.

---

## Update log

- **2026-04-13 — initial version** — Created alongside QUALITY-GATE-PLAN.md approval. Approach C (investigation + parallel build + integration). Subagents A, B, C-early kicked off in parallel. Layer 3 explicitly deferred.
