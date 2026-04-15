# src/certificates/
> BRC-52 agent authorization certificates — identity, delegation, revocation, and policy enforcement.

## Overview

This module implements BRC-52 certificate management for agent identity and parent-child authorization. A parent operator can issue a signed certificate to an agent, embedding spending caps, moderation policy, rate limits, tool approval requirements, and tag enforcement. The agent reads these policy fields at task start and enforces them — cert fields can only tighten policy, never relax it. Revocation is modeled as UTXO spending: the parent revokes by requesting the agent spend its revocation UTXO.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 117 | Module declarations, re-exports all public types and functions, unit tests for `CertificateStatus` serde and `CERT_TYPE_AGENT_AUTH` constant. |
| `types.rs` | 135 | Type definitions, constants, and helpers. `CertBudgetLimits` (6-tier spending caps + enforcement mode), `CertModerationPolicy` (PII/profanity modes), `CertToolApproval` (approval-required tool list), `CertTagPolicy` (mandatory tagging), `CertificateStatus` (serde-enabled status enum), `CertCheckResult` (boot-time validation result). Revocation outpoint helpers: `is_null_revocation_outpoint()`, `parse_revocation_outpoint()`. |
| `lifecycle.rs` | 585 | `CertificateManager` struct wrapping `WalletClient`. Status queries: `has_authorization()`, `has_parent_authorization()`, `certificate_status()`. Certificate CRUD: `acquire_parent_authorization()`, `acquire_authorization()` (self-signed), `relinquish_self_signed()`, `prove_authorization()` (selective field reveal), `list_all()`, `relinquish()`. Revocation checking: `is_revoked()` (paginated UTXO scan), `find_revocation_outpoint()` (3-source fallback). Boot-time `check_authorization()` free function. |
| `policy.rs` | 195 | Five async policy readers that extract enforcement rules from the first `agent-authorization` certificate found in the wallet: `read_cert_budget_limits()`, `read_cert_rate_limits()`, `read_cert_moderation_policy()`, `read_cert_tool_approval()`, `read_cert_tag_policy()`. All return `::none()` on missing cert or absent fields. |

## Key Exports

### Constants

| Name | Value | Purpose |
|------|-------|---------|
| `CERT_TYPE_AGENT_AUTH` | `"agent-authorization"` | Well-known certificate type for agent identity |

### Types

| Type | Description |
|------|-------------|
| `CertificateManager` | Wraps `WalletClient` for certificate CRUD and revocation checks |
| `CertificateStatus` | Serde struct: `status` ("parent-signed" / "self-signed" / "none"), optional `certificate` JSON, optional `identity_key` |
| `CertCheckResult` | Enum: `Valid`, `Revoked`, `None` — boot-time authorization check result |
| `CertBudgetLimits` | 6-tier spending caps (`per_task`, `per_hour`, `per_day`, `per_week`, `per_month`, `lifetime`) + `enforcement` mode ("strict" / "advisory") |
| `CertModerationPolicy` | Parent-enforced moderation: `enabled`, `pii_mode`, `profanity_mode` (each "block" / "flag" / "off") |
| `CertToolApproval` | List of tool names requiring manual approval before execution |
| `CertTagPolicy` | Whether all tasks must include at least one tag |

### Functions

| Function | Module | Description |
|----------|--------|-------------|
| `check_authorization()` | lifecycle | Boot-time cert validation: checks for parent-signed cert, validates revocation status. Returns `CertCheckResult`. |
| `read_cert_budget_limits()` | policy | Reads 6 budget tiers + enforcement mode from cert fields |
| `read_cert_rate_limits()` | policy | Reads `rate_limit_rpm_default` and `rate_limit_enabled` from cert fields. Returns `CertRateLimits` (defined in `x402::rate_limit`). |
| `read_cert_moderation_policy()` | policy | Reads `moderation_enabled`, `moderation_pii`, `moderation_profanity` from cert fields |
| `read_cert_tool_approval()` | policy | Reads comma-separated `approval_tools` field from cert |
| `read_cert_tag_policy()` | policy | Reads `tags_required` boolean field from cert |
| `is_null_revocation_outpoint()` | types | Checks if outpoint is empty or the 72-char zero placeholder |
| `parse_revocation_outpoint()` | types | Splits 72-char hex outpoint into `(txid, vout)` |

### CertificateManager Methods

| Method | Description |
|--------|-------------|
| `new(wallet)` | Construct from `WalletClient` |
| `has_authorization()` | Returns true if any `agent-authorization` cert exists |
| `has_parent_authorization()` | Returns true if any cert has `certifier != subject` |
| `certificate_status()` | Returns `CertificateStatus` with status string, cert JSON, identity key |
| `acquire_parent_authorization()` | Acquire parent-signed cert (creates revocation UTXO, parent signs) |
| `acquire_authorization()` | Acquire self-signed cert (null revocation outpoint) |
| `relinquish_self_signed()` | Remove self-signed certs (certifier == subject), returns count |
| `prove_authorization(fields)` | Selective field reveal via BRC-100 prove endpoint |
| `is_revoked(cert)` | Paginated UTXO scan to check revocation status |
| `find_revocation_outpoint(cert)` | 3-source fallback to locate revocation UTXO |
| `list_all()` | List all certificates (any type) from wallet |
| `relinquish(cert)` | Release a specific certificate from wallet |

## Certificate Fields

An `agent-authorization` certificate contains these fields (all stored as strings in the cert's `fields` map):

| Field | Example | Reader |
|-------|---------|--------|
| `name` | `"research-agent"` | `CertificateManager` |
| `capabilities` | `"llm,tools,browser"` | `CertificateManager` |
| `deployed_at` | RFC 3339 timestamp | `CertificateManager` |
| `version` | `"0.1.0"` (from `CARGO_PKG_VERSION`) | `CertificateManager` |
| `budget_per_task` | `"500000"` (sats) | `read_cert_budget_limits()` |
| `budget_per_hour` | `"2000000"` | `read_cert_budget_limits()` |
| `budget_per_day` | `"10000000"` | `read_cert_budget_limits()` |
| `budget_per_week` | `"50000000"` | `read_cert_budget_limits()` |
| `budget_per_month` | `"100000000"` | `read_cert_budget_limits()` |
| `budget_lifetime` | `"500000000"` | `read_cert_budget_limits()` |
| `budget_enforcement` | `"strict"` or `"advisory"` | `read_cert_budget_limits()` |
| `rate_limit_rpm_default` | `"60"` | `read_cert_rate_limits()` |
| `rate_limit_enabled` | `"true"` | `read_cert_rate_limits()` |
| `moderation_enabled` | `"true"` | `read_cert_moderation_policy()` |
| `moderation_pii` | `"block"` / `"flag"` / `"off"` | `read_cert_moderation_policy()` |
| `moderation_profanity` | `"block"` / `"flag"` / `"off"` | `read_cert_moderation_policy()` |
| `approval_tools` | `"execute_bash,file_write"` | `read_cert_tool_approval()` |
| `tags_required` | `"true"` | `read_cert_tag_policy()` |

## Certificate Lifecycle

### Acquisition

Two paths:

1. **Parent-signed** (`acquire_parent_authorization()`): Agent gets its identity key, parent wallet signs `"{cert_type} {agent_key} {serial}"` with protocol `[2, "certificate signing"]`, a revocation UTXO is created in the `worm-revocation` basket, and the agent acquires the cert with the parent as certifier. Serial format: `worm-{name}-{key_prefix}-{timestamp}`.

2. **Self-signed** (`acquire_authorization()`): Agent uses its own identity key as both subject and certifier. Revocation outpoint is the null placeholder (72 zero-hex chars). Used as a bootstrap placeholder until a parent issues a real cert.

### Revocation

Modeled as UTXO spending, checked via `is_revoked()`:

1. `find_revocation_outpoint()` locates the UTXO using 3-source fallback:
   - Fast path: cert's `revocationOutpoint` field (hex string → txid + vout)
   - Fallback 1: scan `worm-revocation` basket for matching serial number (PushDrop field parsing)
   - Fallback 2: scan `worm-state` basket (legacy certs stored there)
2. `is_revoked()` paginates through the revocation basket (1000 UTXOs/page) checking if the UTXO still exists. If found → not revoked. If exhausted all pages without finding → revoked.
3. **Fail-open**: On wallet errors, `is_revoked()` returns `Ok(false)` to avoid blocking the agent on transient connectivity issues.
4. Null outpoints (self-signed certs) always return `Ok(false)` — no revocation mechanism.

### Relinquishment

`relinquish_self_signed()` removes self-signed certs (where `certifier == subject`) after a parent-signed cert is acquired, maintaining the one-active-auth-cert rule. Parent-issued certs cannot be relinquished — they can only be revoked by the parent.

### Selective Proof

`prove_authorization()` uses the wallet's BRC-100 prove endpoint to reveal only requested certificate fields to a verifier without exposing the full certificate.

## Boot-Time Authorization Check

`check_authorization()` runs during server startup:

- Checks if a parent-signed cert exists (`has_parent_authorization()`)
- If found, validates revocation status via `is_revoked()`
- Returns `CertCheckResult::Valid` (present, not revoked), `Revoked` (present, revoked), or `None` (no cert)
- **Never blocks startup** — agent starts regardless of cert status
- Fail-open on wallet errors (returns `Valid` if status check fails)

## Policy Reader Pattern

All five `read_cert_*` functions in `policy.rs` follow the same pattern:

1. Create a `CertificateManager` from the wallet
2. List all certificates via `list_all()`
3. Find the first `agent-authorization` cert (checking both `certificateType` and `type` field names)
4. Extract relevant fields from the cert's `fields` map
5. Return `::none()` on any error or missing data

This pattern ensures graceful degradation — missing or malformed cert fields default to "no policy override" rather than failing.

## Integration Points

- **Runner** (`runner/lifecycle.rs`): Reads cert budget limits at task start via `read_cert_budget_limits()`, applies to `BudgetTracker` via `apply_cert_limits()`. Also reads tool approval policy.
- **Server** (`server/handlers/agent.rs`): Four certificate routes — `/certificates` (status), `/certificates/issue` (parent-signed acquisition), `/certificates/revoke` (spend revocation UTXO), `/certificates/relinquish` (remove self-signed only).
- **Task spawner** (`server/task_spawner.rs`): Checks `check_authorization()` result; gates task creation on revoked certs.
- **Heartbeat** (`heartbeat/features.rs`): `check_certificate_revocation()` runs every 10 minutes, logs error if parent cert is revoked.
- **Moderation** (`moderation.rs`): `ModerationEngine` reads `CertModerationPolicy` to apply parent-enforced content filtering.
- **Budget** (`onchain/budget.rs`): `apply_cert_limits()` overrides config-derived limits with cert-derived limits.
- **Rate limiter** (`x402/rate_limit.rs`): `CertRateLimits` from cert fields merged into rate limiter registry.
- **On-chain state** (`onchain/state.rs`): Revocation UTXOs stored in `BASKET_REVOCATION` (`"worm-revocation"`), parsed via `parse_push_drop_fields()`.

## Related

- [../CLAUDE.md](../CLAUDE.md) — Parent module overview with certificate management section
- [../onchain/CLAUDE.md](../onchain/CLAUDE.md) — BRC-48 state tokens and `BASKET_REVOCATION` basket
- [../runner/CLAUDE.md](../runner/CLAUDE.md) — Agent loop reads cert budget limits and tool approval at task start
- [../server/CLAUDE.md](../server/CLAUDE.md) — HTTP routes for certificate CRUD
- [../heartbeat/CLAUDE.md](../heartbeat/CLAUDE.md) — Periodic revocation check (every 10 minutes)
- [../../CLAUDE.md](../../CLAUDE.md) — Root project docs (BRC-52 in protocol standards table)
