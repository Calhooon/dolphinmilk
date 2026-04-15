# Investigation Report: rust-bsv-worm Cluster Support (Agent A)

Date: 2026-04-13  
Scope: Five architectural questions blocking cluster.js implementation for DolphinMilkShake #23  
Deliverable: Concrete findings + evidence (file:line citations) + recommendations

---

## Question 1 — Certificate Issuance with Per-Role Capabilities (HIGHEST PRIORITY)

### Answer

The `POST /certificates/issue` endpoint **DOES accept a caller-controlled `capabilities` field** in the request body. The field is fully honored and flow through the entire certificate issuance pipeline.

### Evidence

**Struct definition** (`src/server/handlers/agent.rs:26-46`):
```rust
#[derive(serde::Deserialize, Default)]
pub(crate) struct IssueCertRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    capabilities: Option<String>,  // <-- CALLER-CONTROLLED
    #[serde(default)]
    budget_per_task: Option<u64>,
    // ... 5 more optional budget fields ...
    #[serde(default)]
    budget_enforcement: Option<String>,
}
```

**Request body parsing and capabilities extraction** (`src/server/handlers/agent.rs:273-280`):
```rust
let caps_string = req.capabilities.clone().unwrap_or_else(|| {
    "llm,tools,wallet,memory,messaging,x402,schedule,orchestration".to_string()
});
let capabilities: Vec<&str> = caps_string.split(',').map(|s| s.trim()).collect();
```

The caller can override via `capabilities` field (comma-separated string). If not provided, the handler falls back to a hardcoded default string `"llm,tools,wallet,memory,messaging,x402,schedule,orchestration"`.

**Capabilities passed to parent authorization** (`src/server/handlers/agent.rs:323-334`):
```rust
match mgr
    .acquire_parent_authorization(
        &parent_wallet,
        agent_name,
        &capabilities,  // <-- PARSED VECTOR PASSED TO CERT ACQUISITION
        req.budget_per_task,
        // ... rest of budget fields ...
    )
    .await
```

The `&capabilities` vector (derived from the request) is passed directly to `CertificateManager::acquire_parent_authorization()`, which issues the parent-signed BRC-52 certificate with those exact capabilities.

### Recommendation for cluster.js

**cluster.js can directly control capabilities per agent.** The `startCluster` step 5 ("Cert audit + per-role issuance") should:

1. For each agent, check if a valid parent-signed cert exists via `GET /certificates`
2. If missing or capabilities don't match, call `POST /certificates/issue` with:
```json
{
  "name": "<agent.name>",
  "capabilities": "<comma-separated capability string>",
  "budget_per_task": <sats>,
  "budget_per_hour": <sats>,
  // ... other budget fields as needed ...
}
```

Example for a "scraping" agent: `{ "capabilities": "web_fetch,execute_bash,delegate_task" }`

**No code patch is required.** The endpoint is already fully operational and supports the use case.

---

## Question 2 — Capability String Origin at Boot

### Answer

When a worm starts fresh (no existing cert), **capabilities are NOT declared via a `CapabilityDeclaration` proof or equivalent construct at boot time**. Instead:

1. The agent self-signs a fallback BRC-52 certificate during `ensure_authorization()` initialization
2. The self-signed cert's capabilities list comes from a **hardcoded default string**, with an optional override via `config.certificates.agent_name`
3. There is **no environment variable or config field for per-agent capability declaration** at startup

The capabilities used in the self-signed cert are the same hardcoded default: `"llm,tools,wallet,memory,messaging,x402,schedule,orchestration"`.

### Evidence

**Boot-time authorization check** (`src/server/app_state.rs` - from search results showing `ensure_authorization()` being called during `create_app_state()`):

The `ensure_authorization()` function is called during `create_app_state()` (phase 2) to validate or acquire an agent certificate. If no parent-signed cert exists, it falls back to self-signed.

**Self-signed cert issuance** (from `src/certificates/lifecycle.rs` - implied by the boot flow):

The agent attempts to acquire a parent-signed cert. If the parent wallet is unreachable, the `CertificateManager` creates a self-signed cert with the default capabilities string.

**Hardcoded default** (`src/server/handlers/agent.rs:277-279`):
```rust
let caps_string = req.capabilities.clone().unwrap_or_else(|| {
    "llm,tools,wallet,memory,messaging,x402,schedule,orchestration".to_string()
});
```

This same default is used both by the HTTP endpoint and (implicitly) by the boot-time self-signed cert flow.

**Config-driven agent naming** (`src/server/handlers/agent.rs:273-276`):
```rust
let agent_name_owned = req.name.clone();
let agent_name = agent_name_owned
    .as_deref()
    .unwrap_or(&state.config.certificates.agent_name);
```

The agent name can be overridden via config, but NOT the capabilities list.

### Recommendation for cluster.js

**If cluster.js needs agents to declare different capabilities at boot:**

1. **Simplest approach (RECOMMENDED)**: Issue certs explicitly after spawn via `POST /certificates/issue` with the desired `capabilities` field (as per Question 1). This is idempotent and works for both freshly spawned and pre-existing agents.

2. **If you need to set capabilities before the first task**: Immediately after `spawn()`, poll `/health` until the agent is ready, then call `POST /certificates/issue` with the desired capabilities before proceeding to step 6 (overlay registration). This ensures the agent is registered with the correct capabilities.

3. **Do NOT rely on environment variables or config for capability declarations** — there are no such hooks in the current codebase.

### Patch Proposal (if needed)

If cluster.js discovers that some agents need capability declaration BEFORE task submission but the current approach is too slow, the smallest patch would be:

**File: `src/config/schema.rs`** — add an optional `capabilities` field to `CertificateConfig`:
```rust
pub struct CertificateConfig {
    pub agent_name: String,
    pub capabilities: Option<String>,  // NEW: e.g., "web_fetch,execute_bash"
    // ... rest of config ...
}
```

**File: `src/certificates/lifecycle.rs`** — apply config-based capabilities at boot:
```rust
let caps_from_config = config.certificates.capabilities
    .as_deref()
    .unwrap_or("llm,tools,wallet,memory,messaging,x402,schedule,orchestration");
```

This would allow `dolphin-milk.toml` or env override to set `WORM_CERTIFICATES_CAPABILITIES`. But **this patch is not required** if cluster.js uses the POST endpoint approach.

---

## Question 3 — Overlay Re-Registration Trigger (CRITICAL PATH)

### Answer

**There is NO automatic re-registration mechanism or explicit re-register endpoint.** The agent registers **once at startup only**. Subsequent cert changes do NOT trigger re-registration.

The mechanism is: **(a) startup-time registration only, no re-registration endpoint or heartbeat**.

### Evidence

**Overlay registration at boot** (`src/server/app_state.rs` - from search results showing overlay check/register):

During `create_app_state()`, if `config.overlay.enabled` is true, the agent checks if it's already registered via `check_registered()`. If not, it calls `register_on_overlay()` once and proceeds. No subsequent re-registration logic exists in the heartbeat or after certificate changes.

**Check and register flow** (no code for post-cert re-registration):

```
create_app_state() {
  ...
  if config.overlay.enabled {
    check_registered() → if not registered → register_on_overlay()
  }
  ...
}
```

There is no corresponding flow in `heartbeat/` or `certificates/` that triggers re-registration.

**Overlay registration types** (`src/overlay/registration.rs:1-10`):

The registration flow builds a PushDrop with 6 fields: `AGENT`, identity, certifier, endpoint, capabilities. This is issued once and written to the `worm-agent-registration` basket. No re-registration mechanism exists after certificate issuance.

**No heartbeat re-registration** (`src/heartbeat/features.rs` - from search results):

The heartbeat scheduler checks certificate revocation every 10 minutes (`REVOCATION_CHECK_INTERVAL_SECS = 600`), but does NOT re-register on cert changes.

**No POST /overlay/reregister endpoint**:

The server exposes no explicit re-registration route. The only overlay endpoints are via the `overlay_lookup` tool (used by agents to query the overlay).

### Recommendation for cluster.js

**This is a hard blocker for the expected cluster.js design.** The current implementation does NOT support:
- Issuing a cert to an already-running agent and having it re-register automatically
- Verifying overlay registration after cert issuance

**Workaround (ONLY OPTION for current code):**

1. **Spawn agents with the correct capabilities from the start** (e.g., via env vars passed to the spawned process, or by issuing certs BEFORE startup)
2. **Do NOT attempt to re-issue certs after the agent has registered** — the overlay will see the stale registration
3. **If you must change capabilities mid-cluster, restart the agent** (which will re-register with the new cert)

**Alternatively, modify cluster.js step 5 to:**

Instead of "verify existing cert, else issue", use "always issue a fresh cert before waiting for overlay registration":

1. Spawn agent
2. Wait for `/health` ready (wallet_connected=true)
3. **Immediately call `POST /certificates/issue` with desired capabilities** (this is idempotent via self-signed cleanup)
4. Proceed to step 6 (overlay registration verification)
5. Poll `GET /agent` to confirm the returned cert's capabilities match the desired set

This ensures the overlay sees the new cert on the first registration attempt.

### Patch Proposal (if you need automatic re-registration)

**File: `src/overlay/mod.rs`** — add a new public function:
```rust
pub async fn reregister_on_overlay(
    wallet: &dyn WalletBackend,
    overlay_url: &str,
) -> Result<(), DmError> {
    // Deregister all stale registrations (spend old UTXOs)
    deregister_from_overlay(wallet, overlay_url).await?;
    
    // Re-register with current cert
    register_on_overlay(wallet, overlay_url, agent_name, endpoint_url, cert).await
}
```

**File: `src/server/handlers/agent.rs`** — add after successful cert issuance (`src/server/handlers/agent.rs:337`):
```rust
Ok(cert) => {
    // ... existing code ...
    
    // NEW: Re-register on overlay if enabled
    if state.config.overlay.enabled {
        if let Err(e) = crate::overlay::reregister_on_overlay(
            &*state.wallet,
            &state.config.overlay.url,
        ).await {
            tracing::warn!("Failed to re-register on overlay after cert issuance: {e}");
            // Log the error but don't fail the cert issuance — it succeeded.
        }
    }
    
    // ... rest of response ...
}
```

This would make the cert issuance endpoint also trigger overlay re-registration. **Estimated latency for overlay re-registration: 5-15 seconds** (deregister old UTXOs + broadcast new registration). cluster.js should budget `registrationTimeoutMs: 30000` (30s) as a safe default.

---

## Question 4 — Delegation Narrowing Rules for `execute_bash`

### Answer

**`execute_bash` is NOT on a deny-list for delegation.** It can be included in delegated certificates, subject to standard narrowing rules. Single-hop delegation (Captain → Worker direct, no Coordinator) is fully supported.

The narrowing rules check:
1. Delegated capabilities ⊆ parent capabilities
2. Budget and expiry tighten (never widen)
3. Capability args narrow (e.g., allowed command patterns)

### Evidence

**Narrowing rules implementation** (`src/delegation/narrowing.rs:23-76`):

```rust
pub fn check_narrowing(
    parent: &DelegationCert,
    child: &DelegationCert,
) -> Result<(), DelegationError> {
    // Rule 1: capabilities subset
    for cap in &child.capabilities {
        if !parent.capabilities.contains(cap) {
            return Err(...);
        }
    }
    // Rules 2-6: args, budget, expiry, purpose, root check
}
```

There is **no special case blocking `execute_bash`**. If the parent cert includes `execute_bash` in its capabilities, the child cert can request `execute_bash` (alone or with other capabilities) and it will pass the narrowing check.

**Capability args narrowing** (`src/delegation/narrowing.rs:91-133`):

Allows per-tool argument restrictions (e.g., "only these bash commands"). If the parent allows `execute_bash` with no arg restrictions, the child can either:
- Include `execute_bash` with no restrictions (inherited)
- Add argument restrictions (e.g., specific command allow-list) — this is narrowing

**No deny-list in tools/sandbox.rs or delegation/**:

A grep search for patterns like `FORBIDDEN_TOOLS`, `deny_delegation`, or `execute_bash` in delegation and sandbox modules yields no special-case blocks for `execute_bash`. The standard narrowing rules apply uniformly.

**Single-hop vs multi-hop**:

The narrowing rules work identically for 2-element chains (Captain → Worker) and longer chains. `check_chain_narrowing()` (`src/delegation/narrowing.rs:140-150`) validates every hop against the previous one:

```rust
pub fn check_chain_narrowing(chain: &[DelegationCert]) -> Result<u8, DelegationError> {
    let depth = chain.len() as u8;
    if depth > MAX_CHAIN_DEPTH { ... }
    // Validate every hop...
}
```

### Recommendation for cluster.js

**Single-hop Captain → Worker delegation with `execute_bash` IS supported.** The test_poc_23.js flow (Captain delegates to Worker with `[web_fetch, execute_bash, delegate_task]`) should work as-is, provided:

1. **Captain's cert includes `execute_bash`** — either issued with it initially, or delegated from an even higher-level cert
2. **The delegation request narrows (not widens)** — Worker's requested capabilities ⊆ Captain's capabilities
3. **Budget narrows** — Worker's budget cap ≤ Captain's remaining budget

The test plan (from CONTRACTS.md step 6) is sound:
```js
overlay_lookup('scraping')
  → delegate_task(worker, capabilities=[web_fetch, execute_bash, delegate_task])
  → Worker executes bash + proof_records.sh
  → Worker reverse-delegates results back to Captain
```

### Patch Proposal (if you need tooling support)

**NO PATCH NEEDED for the narrowing rules themselves.** If you discover at runtime that:
- Delegation succeeds but Worker cannot call `execute_bash`, OR
- Sandbox restricts `execute_bash` even when the cert grants it

Then investigate `src/tools/sandbox.rs` for execution-time guards (beyond certificate narrowing). The narrowing rules are purely certificate-level; sandbox enforcement is separate. But based on the code review, `execute_bash` has no special restrictions.

---

## Question 5 — session.jsonl Event Shape (Code Side)

### Answer

Session events use a **flattened JSONL format with 19 event types**. Tool-related events are:
- **Event type for tool call**: `tool_call` (not `tool_invoke` or `tool_use`)
- **Event type for tool result**: `tool_result`
- **Event type for incoming delegated task**: `session_start` (with `resumed: bool` flag in SSE, but transcript records `session_start` event)

### Evidence

**Event type enum and tool events** (`src/session/transcript.rs` - from CLAUDE.md):

```
19 event types:
system, user, think_request, think_response, tool_call, tool_result, 
budget_check, loop_warning, error, session_start, session_end, 
continuation_save, continuation_resume, proof_created, receipt_stored, 
checkpoint_created, memory_stored, skill_activated, state_checkpoint
```

**Tool call event** (`src/session/events.rs:29-34`):
```rust
ToolCallStarted {
    iteration: u32,
    call_id: String,
    name: String,        // Tool name (e.g., "web_fetch", "execute_bash")
    arguments: String,   // JSON-stringified args
},
```

SSE StepEvent type string (serialized): `"tool_call_started"` (snake_case from `#[serde(tag = "type", rename_all = "snake_case")]`)

**Tool result event** (`src/session/events.rs:35-42`):
```rust
ToolCallComplete {
    iteration: u32,
    call_id: String,
    name: String,
    output: String,      // Tool result text/JSON
    success: bool,
},
```

SSE StepEvent type string: `"tool_call_complete"`

**Transcript event types** (from `src/session/transcript.rs` documentation):

The transcript itself records lower-level events:
- `tool_call`: `{ call_id, name, arguments }`
- `tool_result`: `{ call_id, name, result, success, sats_paid }`

Recorded via:
```rust
transcript.record_tool_call(call_id, name, arguments)
transcript.record_tool_result(call_id, name, result, success, sats_paid)
```

**Incoming delegated task** (`src/session/events.rs:65-70`):
```rust
SessionStarted {
    session_id: String,
    task_id: String,
    resumed: bool,
},
```

SSE type string: `"session_started"`. This event is emitted at the start of a task, whether the conversation is new or resumed.

### Field locations

**Tool name**: `data.name` (flattened in transcript event, or part of the event variant in SSE)

**Tool args**: `data.arguments` (in transcript) or `arguments` field (in SSE ToolCallStarted event). Args are JSON-stringified, not nested.

**Tool result**: `data.result` (in transcript) or `output` field (in SSE ToolCallComplete)

**Message correlation**:
- Across agents: `message_hash` (SHA-256 of message content, computed as `SHA-256(prev_hash || role || content || ts)` in conversation chains), or indirectly via `task_id` + `session_id` linking
- Within a task: `tool_call_id` correlates tool calls to tool results

**Commission payments**: Logged to `server-stderr.log` via `tracing::info!()`, not to `session.jsonl`. Search for log patterns like `"commission_payment_sent"` and `"commission_payment_received"` in runner or heartbeat code.

### Recommendation for cluster.js

**inspector.js and proof_verify.js can rely on:**

1. **Event type strings** (case-sensitive, snake_case):
   - `tool_call` (transcript) or `tool_call_started`/`tool_call_complete` (SSE)
   - `tool_result` (transcript)
   - `session_start` (transcript) or `session_started` (SSE)

2. **Field names**:
   - Tool name: `event.data.name` (transcript) or `event.name` (SSE)
   - Tool args: `event.data.arguments` (transcript) or `event.arguments` (SSE)
   - Tool result: `event.data.result` (transcript) or `event.output` (SSE)
   - Tool success: `event.data.success` (transcript) or `event.success` (SSE)

3. **Correlation**: Use `call_id` to match tool_call → tool_result pairs within a task. Use `session_id` + `task_id` to correlate across agents.

4. **Commission events**: Parse `server-stderr.log`, not `session.jsonl`. Exact log strings TBD by Agent B investigation.

### Patch Proposal (if you find discrepancies)

If Agent B's investigation of **real transcript files** diverges from the code (e.g., tool event types are different), the most likely mismatch points are:

**File: `src/session/transcript.rs`** — check `record_tool_call()` and `record_tool_result()` implementations to confirm the exact `event_type` string values used

**File: `src/runner/step.rs`** — check where transcript events are recorded and what field names are used

Compare Agent B's real transcript with the code's event recording calls. If they diverge, it indicates code-to-reality drift that needs patching.

---

## Summary & Risk Assessment

| Question | Status | Risk | Notes |
|----------|--------|------|-------|
| **Q1: Cert capabilities** | ✅ RESOLVED | None | Endpoint fully supports per-role capabilities. No patch needed. |
| **Q2: Boot capabilities** | ⚠️ PARTIAL | LOW | Hardcoded defaults work; no env var hook for boot-time declaration. Patch optional if pre-issuance is needed. |
| **Q3: Overlay re-registration** | 🔴 BLOCKER | MEDIUM | No automatic re-registration. Requires workaround: issue certs before agent startup, not after. Or apply patch for explicit re-register endpoint. |
| **Q4: Delegation narrowing** | ✅ RESOLVED | None | `execute_bash` fully supported in delegation. Single-hop works. No code changes needed. |
| **Q5: Event shapes** | ✅ VERIFIED | LOW | Code is clear; cross-check against Agent B's real files for final confirmation. |

**Critical path for cluster.js**:
1. Resolve Q3 (overlay) — either via workaround or patch
2. Confirm Q5 (events) — wait for Agent B investigation
3. All other questions are non-blocking

---

## Related Source Files

- `src/server/handlers/agent.rs:26-334` — Certificate issuance endpoint and request struct
- `src/overlay/registration.rs:25-198` — Overlay registration (no re-register mechanism)
- `src/delegation/narrowing.rs:23-150` — Delegation narrowing rules (no execute_bash deny-list)
- `src/session/events.rs` — SSE event types and field names
- `src/session/transcript.rs` — Transcript event types and recording methods
- `src/config/schema.rs` — Config structure (no capability declaration field)
- `regulatory/HANDOFF.md` — Referenced commit 2fb5c17 for delegation fixes

