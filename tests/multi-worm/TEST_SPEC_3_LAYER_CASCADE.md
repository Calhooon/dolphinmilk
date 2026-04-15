# Test Spec: 3-Layer Agent Cascade + NanoStore Data Sharing

Two E2E tests that validate the DolphinSense 25-agent architecture. Run after delegation Phase 3 and working memory are merged.

## Test 1: 3-Layer Cascade (Captain → Coordinator → Worker → back)

### What we're testing

A task flows through 3 layers of the agent pyramid via delegation + MessageBox and results return to Captain. This is the first test of the COORDINATOR role — an agent that receives a delegation, breaks it into sub-tasks, dispatches to workers, aggregates results, and returns.

The worm's existing `test_captain_coral_delegation.js` is 2-layer (Captain → Worker). This test adds the middle layer.

### Setup

3 agents, each with own wallet (daemon mode, pre-split UTXOs):

| Agent | Role | Port | Wallet | Model | Capabilities |
|-------|------|------|--------|-------|-------------|
| Captain | Orchestrator | 8081 | 3322 | claude-haiku-4-5 | orchestration |
| Coordinator | Dispatch + aggregate | 8082 | 3323 | claude-haiku-4-5 | coordination, scraping_dispatch |
| Worker | Scraper | 8083 | 3324 | gpt-5-mini | scraping, web_fetch, reddit |

All 3 share the same parent (MetaNet Client on 3321). All register on the overlay.

### Wallet provisioning notes (worker `:3324`)

The worker wallet was provisioned 2026-04-13. State on disk and runtime:

| Field | Value |
|-------|-------|
| Wallet binary | `/Users/johncalhoun/bsv/bsv-wallet-cli/target/release/bsv-wallet` |
| DB path | `~/bsv/wallets/worker-3324.db` |
| Root key env | `~/bsv/wallets/worker-3324.env` (gitignored, single `ROOT_KEY=...` line) |
| Daemon command | `export $(cat ~/bsv/wallets/worker-3324.env) && bsv-wallet daemon --db ~/bsv/wallets/worker-3324.db --port 3324` |
| Identity key | `026468a60d00ec36bc1dafafbd8992d12f40fb6a4740f278deb5ee94d346bd9722` |
| BRC-29 funding addr | `1GDzcxf7McziLpu4sFZGEvKJo6bXKrVty5` (hash160 `a6fefa1fb342d6e32faba5db94567a2b75913059`) |
| Final balance | 2,099,802 sats (1.05M initial fund + 1.05M recovery sweep − fees) |
| UTXO count | 42 (1 × 525,191 reserve + 20 × 52,490 + 20 × 26,235 + 1 × 111 dust) |
| Funding txids | `2a80edbd25fe59370ca3c30ea79030862f0445fb6a708fb764bcdf0816cd7ff2` (first fund, BRC-29 addr); `00dce09e68ddd55cb2e7aaa5dcee7bd5e09e366006b4bfd99c6e2318bfd234d1` (first split); `799491e77c0794f6f41d4ae6420c376612b97d214885eda9a4284b749deca861` (recovery sweep — see below); `f3acf07f8d6809a2ce193fdd55396b95b36662e60b66c0ac58a4119a89477478` (second split) |
| Funded from | `:3322` via `createAction` → BEEF-from-WoC → AtomicBEEF wrap → `bsv-wallet fund` |

**Critical funding gotcha:** the `bsv-wallet send`/`fund` flow hardcodes BRC-29 derivation constants (`SfKxPIJNgdI=` / `NaGLC6fMH50=`). Sending must be to the **BRC-29-derived funding address** from `bsv-wallet address`, NOT the identity-key address from `bsv-wallet identity` — they are different addresses for the same wallet. The first attempted fund tx (`2d6ad71e9ca73c964bec6bf7c2560c9358a4dcd682fcd8adf858cc3e92470210`, 1.05M sats) sent to the identity-key address and was rejected by `bsv-wallet fund` with "Derivation mismatch".

**Recovery from wrong-address send:** the stuck UTXO was recoverable because the worker holds the identity private key. A manual sweep tx was built outside the wallet using `@bsv/sdk` (`tests/multi-worm/sweep-worker-stuck.js`): load `ROOT_KEY` from `~/bsv/wallets/worker-3324.env`, build a 1-input/1-output P2PKH→P2PKH tx with `P2PKH().unlock(privKey)`, pay output to the BRC-29 funding address, `tx.fee()` + `tx.sign()` + `tx.broadcast()`, then `bsv-wallet fund` the resulting AtomicBEEF. Sweep cost: 20 sats. Result: 1,049,980 sats internalized as proper BRC-29 spendable balance. The same script can be repurposed if a future fund tx misses the BRC-29 address.

**Critical BEEF gotcha:** WoC returns BEEF format, but `bsv-wallet fund` expects AtomicBEEF (`01010101` magic + reversed txid + BEEF). Wrap with `Beef.fromString(beefHex).toBinaryAtomic(txid)` from `@bsv/sdk` before passing to `fund`.

**MUST use `daemon` not `serve`.** The serve mode has no chain monitor — txs stay unproven, BEEF chains grow deep, wallet hits double-spend errors after ~30 txs. This burned 3 test wallets during POC testing.

### Scenario

1. Submit task to Captain: "Find a scraping coordinator via overlay. Ask them to get the top 5 posts from /r/technology. Report what you get back."
2. Captain discovers Coordinator via `overlay_lookup(findByCapability: "scraping_dispatch")`
3. Captain delegates to Coordinator via `delegate_task` or `send_message`
4. Coordinator discovers Worker via `overlay_lookup(findByCapability: "scraping")`
5. Coordinator dispatches sub-task to Worker
6. Worker calls `web_fetch("https://www.reddit.com/r/technology/hot.json?limit=5")`
7. Worker sends results back to Coordinator
8. Coordinator aggregates (at minimum: confirms records received, counts them)
9. Coordinator sends aggregated results back to Captain
10. Captain receives results, stores in `working_memory_set`

### Measurements (TRACK ALL OF THESE)

After the test completes, query each wallet DB:

```sql
-- Run against each agent's wallet
SELECT description, COUNT(*) as cnt FROM transactions 
WHERE description NOT LIKE '%Intern%' AND description NOT LIKE '%split%' 
GROUP BY description ORDER BY cnt DESC;
```

Record:
- **Total txs per wallet** (Captain, Coordinator, Worker)
- **Total txs across all 3 wallets combined**
- **Wall clock time** from Captain task submission to Captain receiving final result
- **Iterations per agent** (from task audit endpoint)
- **txs per minute** = total txs / wall clock minutes

This number tells us whether 25 agents can hit 1.5M txs/day. Our single-agent tests showed 5.6 tx/iter at 48s/iter = ~7 tx/min/agent. Multi-agent messaging should push this higher.

### Deep Transcript Inspection (CRITICAL)

After the test, read EVERY tool call and result in ALL 3 agents' transcripts. Not summaries — the actual events.

**Captain's transcript — verify:**
- Called `overlay_lookup` and found the Coordinator
- Called `delegate_task` or `send_message` to Coordinator with research task
- RECEIVED results back from Coordinator
- Called `working_memory_set` to extract findings (if working memory is merged)
- Commission payment flow triggered (if applicable)
- No refusal loops, no hallucinated tool calls, no stuck iterations

**Coordinator's transcript — verify:**
- Received the task from Captain
- Called `overlay_lookup` and found the Worker
- Dispatched sub-task to Worker (re-delegate or send_message)
- RECEIVED results back from Worker
- AGGREGATED results (didn't just pass-through unchanged)
- Sent aggregated results back to Captain (not to Worker, not to nobody)
- Delegation cert chain worked (Coordinator had tool access, wasn't stripped to allowlist)

**Worker's transcript — verify:**
- Received sub-task from Coordinator
- Called `web_fetch` successfully (got real Reddit data)
- Sent results back to Coordinator (NOT to Captain)
- Terminated cleanly (session_end, no error)
- Reasonable iteration count (≤5 iterations for a simple fetch task)

**Red flags:**
- "For security I cannot..." → delegation cert not working, tools stripped
- Agent sending results to wrong recipient
- Empty web_fetch results
- Coordinator answering the question itself instead of dispatching to Worker
- Infinite loops or repeated tool calls
- "Content cleared" → microcompact eating results

### How to get transcripts

```javascript
// Via auth library
const audit = await authGet(`http://localhost:${port}/task/${taskId}/audit`, 3321);
// audit.body.events contains all tool_call, tool_result, think_response events
```

```bash
# Or direct from workspace files
cat test-workspaces/{agent}/tasks/{taskId}/session.jsonl
```

### Pass criteria

- [ ] Task flows down all 3 layers and results flow back up
- [ ] Worker actually called web_fetch and got real data
- [ ] Coordinator aggregated (not just forwarded)
- [ ] Captain received final results
- [ ] All 3 agents terminated cleanly
- [ ] Transcript inspection: no red flags in any agent
- [ ] Total wall clock time recorded
- [ ] Total txs across all 3 wallets recorded

---

## Test 2: NanoStore Data Sharing Between Agents

### What we're testing

Agent A uploads data to NanoStore via x402, gets a permanent UHRP URL, sends the URL to Agent B via MessageBox, Agent B fetches the data from the URL. This is how large datasets flow between agents without stuffing everything into MessageBox bodies.

### Setup

2 agents (can reuse Captain + Worker from Test 1, or any 2 agents with x402 capability).

### Scenario

1. Submit task to Agent A: "Create a small JSON dataset with 5 test records. Upload it to NanoStore using upload_to_nanostore. Then send the UHRP URL to Agent B via send_message."
2. Agent A calls `upload_to_nanostore` with test data
3. Agent A sends the UHRP URL to Agent B via `send_message`
4. Agent B receives the message, extracts the URL
5. Agent B calls `web_fetch` on the UHRP URL
6. Agent B confirms data matches

### Measurements

- Did `upload_to_nanostore` succeed? Cost in sats?
- Round-trip time: upload → URL → message → fetch
- Data integrity: does fetched data match uploaded data?
- Txs generated per agent

### Deep Transcript Inspection

**Agent A:**
- `upload_to_nanostore` tool call: what params? what response? UHRP URL returned?
- `send_message` tool call: did it include the UHRP URL in the body?
- Cost of the upload (sats)

**Agent B:**
- Received message: does it contain the UHRP URL?
- `web_fetch` on the URL: did it return data? Is it the right data?
- Any auth errors fetching from NanoStore?

### Pass criteria

- [ ] Upload succeeds, returns UHRP URL
- [ ] URL is fetchable by a different agent
- [ ] Data matches
- [ ] Round-trip time and costs recorded

---

## Wallet Setup (CRITICAL — learned from POC testing today)

**MUST use `bsv-wallet daemon` not `serve`**. The serve mode has no monitor — txs stay unproven, BEEF chains grow deep, wallet hits double-spend errors after ~30 txs. We burned through 3 test wallets learning this.

**Pre-split UTXOs** after funding: `bsv-wallet split --count 20` creates parallel UTXOs so the wallet doesn't chain every tx off a single UTXO.

**Working memory branch** (`feat/working-memory`) needs to be merged before running. It removes `memory_search` from COMPACTABLE_TOOLS and adds the working_memory_set/clear/list tools. **Status:** merged to `main` 2026-04-13 as commit `f107c22` / merge `be29009`. Tool names confirmed in source (`src/tools/working_memory_tools.rs`): `working_memory_set`, `working_memory_clear`, `working_memory_list`, `working_memory_clear_all`.

**Delegation Phase 3** also needs to be merged. **Status:** merged to `main` 2026-04-13 across `48aa1c8` / `6a13d03` / `0778155` / `088247a`. Verified via `test_captain_coral_delegation.js` (9/9 PASS).

**Model id `claude-haiku-4-5`** confirmed valid in `src/config/schema.rs`.

**Capability strings (`scraping_dispatch`, `scraping`, `coordination`)** are free-form — no overlay-side enumeration. The cascade test must register Coordinator with `capabilities=[..., "scraping_dispatch"]` and Worker with `capabilities=[..., "scraping"]` at agent startup so `overlay_lookup(findByCapability: ...)` resolves them.

## Why these tests matter

Single-agent tests show 5.6 tx/iteration at ~48 seconds. Extrapolating to 25 agents: ~252K txs/day — 6x short of the 1.5M target. Multi-agent messaging adds txs we haven't measured yet (delegation certs, commission payments, message proofs on both sides). These tests give us the REAL multi-agent tx rate to plan with.
