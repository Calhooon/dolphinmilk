# Security Policy

bsv-worm is an autonomous AI agent that holds BSV private keys and makes real payments. Security vulnerabilities in this project can result in direct financial loss. We take every report seriously.

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Email security reports to: **security@x402agency.com**

Include:
- Description of the vulnerability
- Steps to reproduce (minimal, if possible)
- Affected version or commit hash
- Impact assessment (what an attacker could achieve)

If you want to encrypt your report, use the PGP key published at `https://x402agency.com/.well-known/pgp-key.txt`.

## Response Timeline

| Stage | Timeframe |
|-------|-----------|
| Acknowledgment | Within 48 hours |
| Initial triage | Within 72 hours |
| Fix for critical issues | Within 7 days |
| Fix for non-critical issues | Within 30 days |
| Public disclosure | After fix is released, coordinated with reporter |

We will credit reporters in the release notes unless they prefer to remain anonymous.

## Scope

The following are considered security issues:

### Critical
- **Private key exposure** -- any path that leaks wallet private keys, BRC-42 derived keys, or share material
- **Unauthorized payments** -- bypassing budget limits, making payments without user consent, or manipulating transaction construction
- **Authentication bypass** -- circumventing BRC-31 Authrite mutual authentication on protected endpoints
- **Prompt injection leading to fund theft** -- external messages that cause the agent to transfer funds to an attacker
- **Memory encryption bypass** -- reading wallet-encrypted memory entries without proper key derivation

### High
- **Budget limit bypass** -- any mechanism to exceed configured per-task, per-hour, or per-day spending limits
- **Tool sandbox escape** -- executing arbitrary commands outside the allowed sandbox (e.g., bypassing `FORBIDDEN_PATTERNS` or protected path checks)
- **Cross-agent impersonation** -- forging BRC-77 message signatures or spoofing identity keys
- **Certificate forgery** -- creating or manipulating BRC-52 authorization certificates without proper signing authority
- **On-chain proof manipulation** -- tampering with BRC-18 proof hashes or BRC-48 state tokens to falsify the audit trail

### Medium
- **Information disclosure** -- log redaction bypass (leaking API keys, Bearer tokens, private keys through logs)
- **Injection attacks** -- bypassing the 4-layer sanitization in `sanitize.rs` to inject instructions via external messages
- **Session hijacking** -- stealing or replaying Authrite sessions to access protected HTTP API endpoints
- **BEEF/transaction malleation** -- manipulating transaction data between construction and broadcast

## Out of Scope

The following are **not** considered security vulnerabilities for this project:

- Denial of service against self-hosted instances (the agent runs locally; network-level DoS is an infrastructure concern)
- Social engineering attacks against operators
- Vulnerabilities in upstream dependencies that do not affect bsv-worm's usage of them (report these to the upstream project)
- Rate limiting on the HTTP API (the API is intended for local or trusted-network access)
- Theoretical attacks that require physical access to the host machine
- Issues in the web UI that do not affect the backend (XSS in the Lit frontend that cannot reach wallet endpoints)

## Security Design

For context on the security architecture:

- **Wallet isolation** -- the wallet runs as a separate process (`bsv-wallet-cli` on `localhost:3322`). bsv-worm never directly handles raw private keys.
- **BRC-31 mutual authentication** -- protected endpoints require Authrite handshake. Session state is server-side with 1-hour TTL.
- **4-layer injection defense** -- external messages pass through content sanitization, boundary wrapping, tool allowlisting, and parent bypass checks (`sanitize.rs`).
- **Budget enforcement** -- multi-tier spending limits (per-task, per-hour, per-day, per-week, per-month, lifetime) with strict and advisory modes. High-value transactions require manual approval via staging.
- **Log redaction** -- `RedactingWriter` in `logging.rs` scrubs 6 categories of sensitive data from all log output.
- **Protected paths** -- the agent cannot modify its own configuration files (`worm.toml`, `Cargo.toml`, `Cargo.lock`) or execute forbidden shell commands.
- **Memory encryption** -- wallet-native encryption with protocol `[2, "worm memory"]` and counterparty `"self"`.
- **Certificate-based capability restriction** -- BRC-52 certificates limit which tool categories an agent can use. Revocation is modeled as UTXO spending.

## Supported Versions

Security fixes are applied to the latest release on the `main` branch. We do not backport fixes to older versions.
