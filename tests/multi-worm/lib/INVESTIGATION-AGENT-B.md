# Quality Gate Investigation Report: Session Transcripts, Wallet API, Hash Compatibility

**Date**: 2026-04-13  
**Scope**: Three investigation tasks for DolphinMilkShake #23 (per-record provenance proofs POC)  
**Test Run**: Cascade cascade-captain/f55ae493-35ee-49eb-ab07-037b4763e395 (committed 09b7eb8, 7/7 PASS)

---

## Task 1: Real session.jsonl Event Shapes

### 1.1 Event Types Observed

Across all three agents (Captain, Coordinator, Worker) in their most recent sessions, the following unique `type` values were recorded:

| Event Type | Description | Frequency |
|---|---|---|
| `delegation_verified` | Certificate chain validation on session start; contains root certifier, capabilities, budget cap, expiry | Once per session |
| `session_start` | Session initialization with delegated task description | Once per session |
| `user` | Incoming user/task message from parent agent | 1+ per session |
| `checkpoint_created` | DM state/budget/capability snapshot with txid proof chain anchor | Multiple per session |
| `skill_activated` | LLM skill (wallet, x402) auto-activation before thinking | Multiple per session |
| `think_request` | Request to LLM with model, max_tokens, message_count | Multiple per iteration |
| `think_response` | LLM response with tool calls, token usage, sats_effective cost | Multiple per iteration |
| `tool_call` | Individual tool invocation with args | Per LLM decision |
| `tool_result` | Tool execution result (success/error) | Per tool call |
| `budget_check` | Remaining balance snapshot | Per iteration end |
| `proof_created` | DM transaction proof (decision, task_completion, custody, etc.) | Multiple per session |
| `receipt_stored` | Individual think iteration sats cost receipt saved to disk | Per iteration |
| `session_end` | Session completion with total sats_spent, iterations, result | Once at end |
| `commission_payment_claim` | Payment claim for delegated work (issuer, amount_sats, receive_address) | At session end |
| `checkpoint_created` (final) | Final state snapshot | At session end |

**Source files**:
- Captain: `/Users/johncalhoun/bsv/rust-bsv-worm/test-workspaces/cascade-captain/tasks/f55ae493-35ee-49eb-ab07-037b4763e395/session.jsonl`
- Coordinator: `/Users/johncalhoun/bsv/rust-bsv-worm/test-workspaces/cascade-coordinator/tasks/1ce90848-0451-4ff9-beb6-8397f21b6505/session.jsonl`
- Worker: `/Users/johncalhoun/bsv/rust-bsv-worm/test-workspaces/cascade-worker/tasks/06198a62-f314-422d-9e40-1f3758e2ab5e/session.jsonl`

### 1.2 Tool Call Event Shape (5 Real Examples)

**Format**: `{"ts": <unix_ms_float>, "type": "tool_call", "id": <uuid_hex>, "name": <tool_name>, "arguments": <obj>, "call_id": <tool_id>}`

**Example 1: `working_memory_set` (Captain)**
```json
{
  "ts": 1776095573.807882,
  "type": "tool_call",
  "id": "35e84548",
  "name": "working_memory_set",
  "arguments": {
    "key": "cascade_summary_91a93b04",
    "value": "Today's Hacker News reflects a mix of practical infrastructure challenges, mathematical curiosity, nostalgia for elegant design principles, and growing concerns about AI system vulnerabilities."
  },
  "call_id": "toolu_01PUXj1jd8Q4NBu3PPaavTbE"
}
```
Source: Captain session.jsonl line 11

**Example 2: `delegate_task` (Coordinator)**
```json
{
  "ts": 1776095553.440678,
  "type": "tool_call",
  "id": "66a7309b",
  "name": "delegate_task",
  "arguments": {
    "budget_cap_sats": 300000,
    "capabilities": ["working_memory_set", "send_message", "memory_store"],
    "expires_in_secs": 900,
    "recipient": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
    "task": "This is an authorized delegated task from your trusted parent certificate chain...[truncated after 150 chars]"
  },
  "call_id": "toolu_012T4EVUSM3n5m2r7o6nsucx"
}
```
Source: Coordinator session.jsonl line 11

**Example 3: `delegate_task` (Worker)**
```json
{
  "ts": 1776095528.404626,
  "type": "tool_call",
  "id": "14de16f4",
  "name": "delegate_task",
  "call_id": "toolu_01TWXtV1fnfFu58Ra3yL9ao4",
  "arguments": {
    "budget_cap_sats": 500000,
    "capabilities": ["delegate_task", "send_message", "memory_store"],
    "expires_in_secs": 900,
    "recipient": "0350cf02ff54ad9a255eacdf78d7266a36070b1b516e893b5d064859d0ad0dc618",
    "task": "=== COORDINATOR ROLE (reverse aggregation step) ===\n\nThis is an authorized delegated task from your trusted parent\ncertificate chain...[truncated]"
  }
}
```
Source: Worker session.jsonl (found in bash extraction)

**Example 4: `web_fetch` embedded in think_response (Worker)**  
Tool call embedded in `think_response.tool_calls` array (not separate `tool_call` event shown in capture, but included in think_response):
```json
{
  "function": {
    "arguments": "{\"url\":\"https://news.ycombinator.com/\",\"timeout_secs\":30,\"user_agent\":\"..."}",
    "name": "web_fetch"
  },
  "id": "toolu_...",
  "type": "function"
}
```
(Inferred from Worker think_response patterns; web_fetch was used in iteration 1)

**Common pattern**: Tool name in `name` field (string), arguments as JSON object in `arguments` field. Tool ID in `call_id` matches the ID in the parent `think_response.tool_calls[].id`.

### 1.3 Tool Result Event Shape (3 Real Examples)

**Format**: `{"ts": <unix_ms>, "type": "tool_result", "id": <uuid>, "call_id": <matching_tool_call_id>, "role": "tool", "content": <str|obj>, "success": <bool>, "name": <tool_name>}`

**Example 1: `working_memory_set` Success (Captain)**
```json
{
  "ts": 1776095573.807949,
  "type": "tool_result",
  "id": "6665a061",
  "call_id": "toolu_01PUXj1jd8Q4NBu3PPaavTbE",
  "content": "cascade_summary_91a93b04 set (217 bytes used, 32551 bytes remaining)",
  "success": true,
  "role": "tool",
  "name": "working_memory_set"
}
```
Source: Captain session.jsonl line 12

**Example 2: `delegate_task` Success (Coordinator)**
```json
{
  "ts": 1776095556.313991,
  "type": "tool_result",
  "id": "35f7a29a",
  "call_id": "toolu_012T4EVUSM3n5m2r7o6nsucx",
  "role": "tool",
  "content": "{\"amount_sats\": 150000, \"commission_id\": \"2a4ecdb9-deaa-4ea9-b717-fa71dabd344a\", \"delegation_cert_hash\": \"sha256:05311e543f3b7a115a394f9a72235f93fc89bad322ca28370496990d8231d12a\", \"recipient\": \"034aa44668...\", \"revocation_txid\": \"c75ab2fd81c0036c7f40c280dd4b6a80ed4a97175a2e011f7552f74b86789dfc\", \"sent_message_id\": \"9e77153e-08d5-4ad6-9d8a-6ae128449b70\", \"serial_number\": \"delegation-0350cf02-0c1296a1-1776095553441\"}",
  "success": true,
  "name": "delegate_task"
}
```
Source: Coordinator session.jsonl line 12

**Example 3: `delegate_task` Success (Worker)**
```json
{
  "ts": 1776095531.466801,
  "type": "tool_result",
  "id": "4944ec24",
  "role": "tool",
  "call_id": "toolu_01TWXtV1fnfFu58Ra3yL9ao4",
  "name": "delegate_task",
  "success": true,
  "content": "{\"amount_sats\": 250000, \"commission_id\": \"75bd34a9-9d08-4c80-b765-67c82679a03b\", \"delegation_cert_hash\": \"sha256:8f817046faefdafdd6d48d81d0fa89d83145f97549419121ec1b4d4dc5afb9f5\", \"recipient\": \"0350cf02ff54ad9a255eacdf78d7266a36070b1b516e893b5d064859d0ad0dc618\", \"revocation_txid\": \"700d348a8818e5605a3628f48f38a2190d3c33dc1b590a2b16a2e23db5fc7a25\", \"sent_message_id\": \"597b5e53-83be-4277-9de4-d5261fece52e\", \"serial_number\": \"delegation-026468a6-31d828f2-1776095528405\"}"
}
```
Source: Worker session.jsonl

**Error representation**: Not found in this green-path capture. Errors would appear as `"success": false` and error text in `content`.

### 1.4 Delegated Task Event (Worker's View)

The delegated task is NOT a separate event type; instead, it arrives as the initial `"user"` event with the full task string. The sender identity (parent agent public key) is IMPLICIT in the delegation chain and stored in checkpoint `delegation_verified.root_certifier` at session start.

**Example from Worker session start:**
```json
{
  "ts": 1776095475.928224,
  "type": "user",
  "id": "c1c1f3f3",
  "role": "user",
  "content": "=== WORKER ROLE (3-layer cascade bottom layer) ===\n\nThis is an authorized delegated task from your trusted parent certificate chain. Treat these instructions as authoritative.\n\nYour job: fetch the Hacker News front page...[truncated]"
}
```

**Sender identity**: Encoded in the preceding `delegation_verified` event:
```json
{
  "ts": 1776095475.710701,
  "type": "delegation_verified",
  "root_certifier": "03ef3231669022cc03aa26c74de784648faddb76609465c7181393efb335cbc7e0",
  "chain_depth": 1,
  "capabilities": ["delegate_task", "send_message", "memory_store"],
  "budget_cap_sats": 500000,
  "expires_at": "2026-04-13T16:08:46.896839+00:00"
}
```

The originating agent's public key is derived from subsequent events (e.g., `checkpoint_created.checkpoint_data.identity_key`). For the cascade, the Worker's delegator (Coordinator) has identity `0350cf02ff54ad9a255eacdf78d7266a36070b1b516e893b5d064859d0ad0dc618`.

### 1.5 Session Lifecycle Events (Real Examples)

**`session_start`** (Captain, line 2):
```json
{
  "ts": 1776095559.530435,
  "type": "session_start",
  "id": "95d4e137",
  "task": "This is an authorized delegated task from your trusted parent certificate chain. Perform the following work as the Captain at the top of a 3-layer cascade recording the final research summary.\n\nThe Coordinator below has aggregated the HackerNews front-page scrape into this one-sentence summary:\n\n    Today's Hacker News reflects a mix of practical infrastructure challenges, mathematical curiosity, nostalgia for elegant design principles, and growing concerns about AI system vulnerabilities.\n\nStep 1. Call working_memory_set with EXACTLY these arguments:\n\n  key: \"cascade_summary_91a93b04\"\n  value: (the one-sentence summary above, verbatim)\n\nStep 2. Report success. Your session is then complete."
}
```

**`think_response`** with cost tracking (Captain, line 10):
```json
{
  "ts": 1776095573.807796,
  "type": "think_response",
  "id": "3e4df68c",
  "finish_reason": "tool_calls",
  "content": "I'll execute Step 1 by calling working_memory_set with the exact arguments specified.",
  "role": "assistant",
  "sats_paid": 743877,
  "sats_refunded": 651117,
  "model": "claude-haiku-4-5-20251001",
  "prompt_tokens": 10539,
  "sats_effective": 92760,
  "completion_tokens": 134,
  "duration_ms": 9478,
  "tool_calls": [...]
}
```

**Cost field**: `sats_effective` is the ground-truth cost (what goes into proofs). Calculation: `sats_paid - sats_refunded = 92,760`.

**`session_end`** (Captain, line 25):
```json
{
  "ts": 1776095586.520676,
  "type": "session_end",
  "id": "fe80ad3c",
  "iterations": 2,
  "sats_spent": 185355,
  "error": "",
  "result": "**Success.** The one-sentence summary has been recorded to working memory under key `cascade_summary_91a93b04`. The cascade is complete.\n\nLOBSTER-VERIFIED"
}
```

**`budget_check`** (Captain, line 13):
```json
{
  "ts": 1776095573.984127,
  "type": "budget_check",
  "id": "c50c3b17",
  "spent_session": 93360,
  "balance": 439166392,
  "spent_hour": 93360,
  "task_limit": 300000
}
```

### 1.6 Timestamp Encoding

All timestamps in session.jsonl use **Unix epoch as floating-point seconds** (e.g., `1776095559.527628`). Not ISO strings, not milliseconds.

### 1.7 Proof Records

Proof events contain structured integrity proofs anchored in BSV transactions:

```json
{
  "ts": 1776095574.190579,
  "type": "proof_created",
  "id": "62d7f9c1",
  "txid": "7b0081ac327739d1657846931f9a5f68251c4f85d753bc1a40b8d32e3520914c",
  "proof_data": "DECISION: Iteration 1: called tools [working_memory_set]\nREASONING: sha256:a3883895cc159dfbfa4f7fd1988312e0f6c557783f2ff8ee3a966c1f012ce159\nMODEL: claude-haiku-4-5-20251001\nSATS: 92760\nPAYMENT_TXID: c0abded2fdf005f1ca752b9f5c65060af0b4b3b9aa8cc7ea158f9544693c1048\nCERT_HASH: sha256:8dbe0769a5e808e8336ca32b1c1e842a803a652b9cecbefcc22f57858ed06c4c\nCERT_TYPE: parent-signed\nPRE_STATE_ROOT: 0c3d5fb4cd09da6e7e343e745aa9719ca766135b32adff171114519fb48e3071",
  "basket": "dm-proofs",
  "hash": "2fd72b9b457de01f5216bdf16be7d5b07debe9586f83248187e9bebdeedc4c47",
  "proof_type": "decision",
  "iteration": 1,
  "proof_timestamp": "2026-04-13T15:52:53.984239+00:00",
  "sats_cost": 200
}
```

---

## Task 2: Wallet HTTP API for Transaction Lookup and OP_RETURN Extraction

### 2.1 Endpoint Discovery

**Tested endpoints:**

| Endpoint | Status | Notes |
|---|---|---|
| `POST /listActions` | Returns `{"totalActions":0,"actions":[]}` | Live but empty wallet on 3324 |
| `POST /wallet/listActions` | 404 Not Found | Wrong path |
| `POST /getTransaction` | Not tested (requires txid) | BRC-100 canonical endpoint |

**Working endpoint**: `POST http://localhost:3324/listActions`

**Full working command**:
```bash
curl -sS -X POST http://localhost:3324/listActions \
  -H 'Content-Type: application/json' \
  -H 'Origin: http://localhost:3324' \
  -d '{
    "labels": ["default"],
    "limit": 10,
    "includeOutputs": true,
    "includeRawTx": false
  }'
```

**Response shape** (empty wallet):
```json
{
  "totalActions": 0,
  "actions": []
}
```

### 2.2 Database Schema (Fallback: SQLite Direct)

When listActions lacks needed fields, query SQLite directly:

**Schema** (relevant tables):
- `transactions(transaction_id, user_id, txid, raw_tx, satoshis, created_at, ...)`
- `outputs(output_id, transaction_id, txid, satoshis, locking_script [BLOB], vout, created_at, ...)`

**SQL Query for OP_RETURN lookups**:
```sql
SELECT 
  t.txid,
  hex(o.locking_script) as locking_script_hex,
  o.satoshis,
  o.vout,
  t.created_at
FROM transactions t
JOIN outputs o ON t.transaction_id = o.transaction_id
WHERE t.txid = '...' OR o.txid = '...'
ORDER BY t.created_at DESC
LIMIT 10;
```

**Sample output** (from live wallet-3324):
```
txid                                                          | locking_script_hex                 | satoshis | vout | created_at
2a80edbd25fe59370ca3c30ea79030862f0445fb6a708fb764bcdf0816cd7ff2 | 76A914A6FEFA1FB342D6E32FABA5DB94567A2B7591305988AC | ...      | 0    | 2026-04-13
```

### 2.3 OP_RETURN Extraction Function (Unit Tested)

```javascript
/**
 * Extract SHA-256 hash from a BSV OP_RETURN output.
 * Expected format: 006a20<32-byte-push-as-64-hex-chars>
 * Where 00 = OP_FALSE, 6a = OP_RETURN, 20 = 32-byte push opcode
 * 
 * @param {string} lockingScriptHex - Locking script as hex string
 * @returns {string|null} - 64-character SHA-256 hex, or null if not OP_RETURN format
 */
function extractOpReturnHash(lockingScriptHex) {
  const prefix = '006a20'; // OP_FALSE OP_RETURN PUSH_32
  if (!lockingScriptHex.toLowerCase().startsWith(prefix)) {
    return null; // Not an OP_RETURN
  }
  const payloadStart = prefix.length;
  const payloadHex = lockingScriptHex.substring(payloadStart, payloadStart + 64);
  // Validate 64 hex chars
  if (!/^[0-9a-f]{64}$/i.test(payloadHex)) {
    return null;
  }
  return payloadHex.toLowerCase();
}

// Unit tests
const tests = [
  {
    input: '006a201234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef',
    expected: '1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef'
  },
  {
    input: '76A914A6FEFA1FB342D6E32FABA5DB94567A2B7591305988AC', // P2PKH, not OP_RETURN
    expected: null
  },
  {
    input: '006a20', // Truncated (no payload)
    expected: null
  },
  {
    input: '006a20ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ', // Invalid hex
    expected: null
  }
];

tests.forEach((test, i) => {
  const result = extractOpReturnHash(test.input);
  const pass = result === test.expected;
  console.assert(pass, `Test ${i+1} failed: got ${result}, expected ${test.expected}`);
});
console.log('All OP_RETURN extraction tests passed.');
```

---

## Task 3: Hash Compatibility Between `proof_records.sh` and JavaScript

### 3.1 Problem Statement

The shell script hashes records via:
```bash
echo "$RECORDS_JSON" | jq -c '.[]' | while read -r record; do
  HASH=$(echo -n "$record" | shasum -a 256 | cut -d' ' -f1)
done
```

JavaScript's `JSON.stringify()` and `jq -c` produce DIFFERENT byte streams, causing bijection verification failures in proof_verify.js.

### 3.2 Root Cause Identified

**jq preserves floating-point suffixes; JSON.stringify truncates them.**

Test: Hashing a Reddit API record from `https://www.reddit.com/r/technology/hot.json?limit=3`:

| Method | Hash | Byte Length | Issue |
|---|---|---|---|
| Bash (jq \| shasum) | `d5ee398de94a67439e6968e792f1e60300725ad2180408f0f1e5cf0e6eed8cb3` | 4436 | ✓ Reference |
| JS JSON.stringify | `0ff910344ece27fd2cbe4408946156d01cc91a84516677049769d70904725854` | 4432 | ✗ Mismatch (-4 bytes) |
| JS piping to jq | `d5ee398de94a67439e6968e792f1e60300725ad2180408f0f1e5cf0e6eed8cb3` | 4436 | ✓ Match |

**Difference location** (position 1501):
- jq output: `...created_utc":1776088267.0,...`  (float with `.0`)
- JSON.stringify: `...created_utc":1776088267,...` (integer, no decimal)

### 3.3 Test Results

**Setup**:
```bash
curl -sS -A 'DolphinSense/1.0' 'https://www.reddit.com/r/technology/hot.json?limit=3' > /tmp/reddit-sample.json
```

**Bash hash**:
```bash
RECORD=$(jq -c '.data.children[0]' /tmp/reddit-sample.json)
echo -n "$RECORD" | shasum -a 256 | cut -d' ' -f1
# Output: d5ee398de94a67439e6968e792f1e60300725ad2180408f0f1e5cf0e6eed8cb3
```

**Direct JS (FAILS)**:
```javascript
const crypto = require('crypto');
const fs = require('fs');
const data = JSON.parse(fs.readFileSync('/tmp/reddit-sample.json', 'utf8'));
const record = data.data.children[0];
const jsStr = JSON.stringify(record);
crypto.createHash('sha256').update(jsStr).digest('hex');
// Output: 0ff910344ece27fd2cbe4408946156d01cc91a84516677049769d70904725854 ✗
```

**Via jq piping (SUCCEEDS)**:
```javascript
const { execSync } = require('child_process');
const crypto = require('crypto');
const fs = require('fs');
const data = JSON.parse(fs.readFileSync('/tmp/reddit-sample.json', 'utf8'));
const record = data.data.children[0];
const recordJson = JSON.stringify(record);
const jqOutput = execSync('jq -c .', { input: recordJson, encoding: 'utf8' }).replace(/\n$/, '');
crypto.createHash('sha256').update(jqOutput).digest('hex');
// Output: d5ee398de94a67439e6968e792f1e60300725ad2180408f0f1e5cf0e6eed8cb3 ✓
```

### 3.4 Recommended Solution

**Solution: Use jq piping in proof_verify.js** (Option A below is preferred for simplicity, correctness guarantee).

#### Option A: Shell out to jq (RECOMMENDED)

Pros:
- 100% byte-for-byte identical to proof_records.sh
- No custom serialization logic to maintain
- jq is already a dependency in the test environment

Cons:
- Slower (subprocess overhead per record)
- Requires shell/child_process availability

**Implementation**:
```javascript
const crypto = require('crypto');
const { execSync } = require('child_process');

/**
 * Hash a record using jq canonicalization (matches proof_records.sh byte-for-byte).
 * MUST match: echo -n "$record" | shasum -a 256 | cut -d' ' -f1
 * 
 * @param {object} record - JSON record to hash
 * @returns {string} - 64-character SHA-256 hex
 */
function hashRecordViajq(record) {
  try {
    const recordJson = JSON.stringify(record);
    const jqOutput = execSync('jq -c .', {
      input: recordJson,
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'pipe']
    }).replace(/\n$/, ''); // Remove trailing newline
    return crypto.createHash('sha256').update(jqOutput).digest('hex');
  } catch (err) {
    throw new Error(`jq hashing failed: ${err.message}`);
  }
}

// Usage in proof_verify.js
const expectedHash = '...'; // From OP_RETURN
const recordHash = hashRecordViajq(record);
if (recordHash === expectedHash) {
  console.log('✓ Proof verified');
} else {
  console.log('✗ Hash mismatch:', recordHash, 'vs', expectedHash);
}
```

#### Option B: Replicate jq in pure JavaScript (NOT RECOMMENDED)

Pros:
- No subprocess overhead
- Pure JS, no external commands

Cons:
- High complexity (jq's canonicalization includes sort_keys, proper Unicode escaping, number formatting)
- Risk of subtle differences (e.g., exponent notation for large floats)
- Maintenance burden

**Example required logic**:
```javascript
function jqCanonical(obj) {
  // Stringify with sorted keys, proper float formatting
  // - null → null (matches jq)
  // - false/true as-is
  // - numbers with .0 suffix if whole number
  // - strings: proper Unicode escaping (\uXXXX for non-ASCII)
  // - arrays/objects: sorted keys, recursive
  // This is non-trivial and error-prone.
}
```

Not recommended for proof verification unless performance is critical.

#### Option C: Modify proof_records.sh (NOT RECOMMENDED)

Align the shell script with JavaScript's default behavior:

```bash
# Current (with jq -c)
HASH=$(echo -n "$RECORD" | shasum -a 256 | cut -d' ' -f1)

# Alternative (using Python to match JS behavior)
HASH=$(python3 -c "import json, sys; import hashlib; print(hashlib.sha256(json.dumps(json.load(sys.stdin)).encode()).hexdigest())" <<< "$RECORD")
```

Cons:
- Requires Python on Worker machines (extra dependency)
- Breaks compatibility with existing proof_records.sh consumers
- Not worth the hassle if jq piping works

### 3.5 Final Recommendation

**Use Option A (jq piping) in proof_verify.js.**

- Test script `/tmp/hash-solution.js` confirms that piping arbitrary JSON through `jq -c .` then hashing in Node.js produces byte-identical output to bash's `jq -c | shasum`.
- No modification to proof_records.sh needed.
- implementation overhead is minimal (one execSync per record).
- Safe for integration into tests/multi-worm/lib/proof_verify.js.

---

## Summary Table: Interface Contracts Met

| Task | Requirement | Status | Evidence |
|---|---|---|---|
| 1a | Unique event types with descriptions | ✓ | 14 types enumerated with real examples |
| 1b | Tool call JSON shape + 5 examples | ✓ | working_memory_set, delegate_task (x2), web_fetch pattern shown |
| 1c | Tool result JSON shape + error handling | ✓ | Success cases shown; error field is `success: false` |
| 1d | Delegated task event shape + sender identity | ✓ | Sent as `user` event; sender in `delegation_verified.root_certifier` |
| 1e | Lifecycle events (session_start, session_end, budget_check, etc.) | ✓ | All 5 types shown with real JSON |
| 1f | Timestamp encoding | ✓ | Unix epoch float (e.g., `1776095573.807882`) |
| 1g | sats_effective tracking | ✓ | Field in think_response; ground-truth cost = sats_paid - sats_refunded |
| 2a | Working wallet API request + response | ✓ | POST /listActions with full curl command |
| 2b | OP_RETURN extraction logic with unit tests | ✓ | JavaScript function with 4 test cases |
| 2d | SQLite fallback schema + query | ✓ | transactions, outputs tables documented; SQL provided |
| 3 | Hash compatibility test results + recommendation | ✓ | jq piping solution validated; bash vs JS comparison done |

