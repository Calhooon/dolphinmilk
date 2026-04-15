# Dolphin Milk Use Cases

Dolphin Milk is an **economic runtime for autonomous AI**. Every thought costs real money. Every action is on-chain. Every service is pay-per-call with no API keys, no subscriptions, no billing pages.

That combination unlocks use cases that don't exist anywhere else.

---

## For Developers Building x402 Services

**The pitch: "Build an API. Charge per call. Micropayments handle everything."**

No Stripe integration. No billing page. No invoices. No free tier abuse. You expose a `/.well-known/x402-info` manifest, return 402 with payment headers, and sats flow directly to your wallet on every call. The agent discovers your service automatically from the registry.

### Starter Services (build in a weekend)

| Service | What It Does | Why Agents Need It | Revenue Model |
|---------|-------------|-------------------|---------------|
| **Code Review Bot** | Diff in, review comments out | Every coding agent needs peer review | ~$0.05/review |
| **PDF Extractor** | PDF upload, structured JSON/markdown out | Research agents consuming papers | ~$0.01/page |
| **Translation API** | Text in, translated text out (any pair) | Cross-language research, content creation | ~$0.005/1K chars |
| **Sentiment Analyzer** | Text in, score + confidence out | Social monitoring, market research agents | ~$0.002/call |
| **Web Scraper** | URL in, clean markdown out | Any agent doing web research | ~$0.01/page |
| **DNS/WHOIS Lookup** | Domain in, registrar + nameservers + history out | Security research, competitive intelligence | ~$0.005/query |
| **RSS Aggregator** | Feed URL in, recent items as structured data out | Monitoring agents, news aggregators | ~$0.002/feed |
| **Markdown to PDF** | Markdown in, styled PDF out | Report-building agents, document generation | ~$0.01/doc |
| **Email Validator** | Email address in, deliverability check out | Outreach agents, data cleaning | ~$0.003/check |
| **Calendar/Weather** | Query in, structured schedule + forecast out | Planning agents, logistics | ~$0.001/call |
| **Image OCR** | Image in, extracted text out | Document processing, receipt scanning | ~$0.01/image |
| **Diff Summarizer** | Git diff in, human-readable changelog out | CI/CD agents, release note generators | ~$0.02/diff |
| **Schema Validator** | JSON + schema in, validation report out | API testing agents, data pipeline QA | ~$0.001/call |
| **Link Checker** | URL list in, broken link report out | SEO agents, documentation maintenance | ~$0.005/batch |
| **Text-to-Speech** | Text in, audio file URL out | Content creation, accessibility agents | ~$0.01/500 chars |

**What makes this different from building a normal API**: Your customers are AI agents with wallets, not humans with credit cards. You never deal with authentication flows, billing disputes, or payment processing. The x402 protocol handles it all. An agent discovers your service, reads your manifest, pays per call, and you earn sats.

### Already-Working Services (call these today)

| Service | Capability | Cost | Delivery |
|---------|-----------|------|----------|
| **banana** | Text-to-image (1K/2K/4K) | $0.19-0.38 | Async-poll (1-3 min) |
| **veo** | Text-to-video | $0.19+/sec | Async-poll (2-5 min) |
| **kling** | Video (24+ models) | Variable | Async-poll (2-10 min) |
| **whisper** | Audio transcription | $0.0006/min | Synchronous |
| **nanostore** | Decentralized file storage | ~$0.0001/MB/yr | Two-step |
| **x-research** | Twitter/X full-archive search | $0.06/page | Synchronous |
| **polymirror** | Polymarket whale tracking | $0.005-0.02 | Synchronous |
| **1sat** | BSV inscriptions | 200+ sats | Synchronous |
| **openai** | GPT-5 models via x402 | Per-token | Synchronous |
| **claude** | Claude models via x402 | Per-token | Synchronous |

### Advanced Service Patterns

- **Async-poll services**: Generate media, kick off long jobs, poll for results. The agent handles polling automatically if your manifest specifies `"delivery": "async-poll"` with polling config.
- **Two-step services**: Reserve then execute (like NanoStore). For operations that need a pre-flight reservation before the actual work.
- **Tiered pricing**: Charge more for higher resolution, priority processing, or larger inputs. The manifest `payment.satoshis` field can vary per endpoint.
- **Subscription-style**: Expose a `/subscribe` endpoint that returns data for a time window. Agent calls periodically via heartbeat scheduling.

---

## For Developers Building Agents

### Fork-and-Customize Agents

Dolphin Milk is designed to be forked. The agent loop (OBSERVE -> THINK -> ACT -> RECORD -> BUDGET CHECK) is the skeleton. Everything else is configurable:

**1. Trading Research Agent**
- Uses: Polymarket (already integrated), Twitter search, web fetch, memory store
- Loop: Search for market signals, analyze, store insights, generate daily brief
- Budget: ~$0.50/day for continuous monitoring
- Differentiator: Every trade signal is recorded as an on-chain proof. Verifiable track record.

**2. Content Creation Agent**
- Uses: Image generation (banana), text generation (x402 LLMs), NanoStore upload, 1Sat inscriptions
- Loop: Generate content, upload to permanent storage, publish links, track what performs
- Budget: ~$2-5/day depending on volume
- Differentiator: All content artifacts have permanent UHRP URLs. Content provenance is on-chain.

**3. Security Scanner Agent**
- Uses: Web fetch, DNS lookup, browser tool (headless Chrome), memory for findings
- Loop: Scan targets, analyze, generate report, store proof of findings on-chain
- Budget: ~$0.10/scan
- Differentiator: The audit trail IS the deliverable. Timestamped, immutable, verifiable.

**4. Personal Research Assistant**
- Uses: Twitter search, web fetch, memory store, scheduled heartbeat
- Loop: Daily -- search topics of interest, store findings, generate brief, send via MessageBox
- Budget: ~$0.20/day
- Differentiator: Cross-session memory means it builds context over weeks. It remembers what you asked about last month.

**5. DevOps Watchdog**
- Uses: Web fetch (health checks), shell commands, scheduled heartbeat, MessageBox for alerts
- Loop: Check services on schedule, escalate failures, store incident proofs
- Budget: ~$0.05/day (mostly health checks)
- Differentiator: Budget-constrained by design. Can't accidentally spend unlimited money on monitoring spirals. 6-tier budget limits + cert-driven policy from parent operator.

**6. Due Diligence Agent**
- Uses: Web search, Twitter search, Polymarket data, PDF extraction (new service), memory
- Loop: Research a company/person/topic, cross-reference sources, generate report with citations
- Budget: ~$0.50-2.00/report depending on depth
- Differentiator: Every source accessed is logged. The proof chain shows exactly what information was consulted and when.

**7. Customer Support Agent**
- Uses: Memory (FAQ knowledge base), conversation tracking, MessageBox for escalation
- Loop: Answer questions from memory, escalate unknowns, learn from resolutions
- Budget: ~$0.01-0.05/conversation
- Differentiator: Parent operator controls capabilities via BRC-52 certificates. Revoke access instantly if agent misbehaves. Budget caps prevent runaway costs.

### Integration Patterns

| Pattern | How | Auth | Best For |
|---------|-----|------|----------|
| **MCP client** | `cargo run -- mcp` (stdio) | None needed | Claude Code, VS Code, Codex |
| **HTTP API** | `cargo run -- serve --port 8080` (69 routes) | BRC-31 Authrite | Web apps, dashboards |
| **Agent-to-agent** | BRC-33 MessageBox | BRC-31 + BRC-77 signing | Multi-agent workflows |
| **Wallet MCP bridge** | Drop binary in PATH, auto-discovered | stdio | Extending wallet capabilities |

---

## For Researchers

### Academic Use Cases

**1. AI Agent Economics**
- Study how budget constraints affect agent behavior and decision quality
- Compare agent performance at different budget levels (tight vs generous)
- Analyze cost-per-quality curves across different LLM providers
- Every data point is on-chain -- reproducible, verifiable, timestampable
- Use the introspection tools: `task_costs`, `activity_summary`, `cost_analysis`

**2. Verifiable AI Audit Trails**
- BRC-18 hash-chained proofs create a tamper-evident log of every agent decision
- Study how proof chains can satisfy regulatory requirements (SEC 17a-4, FINRA 3110)
- Compare on-chain audit trails vs traditional logging for trust and compliance
- The proof chain visualization makes this tangible and demonstrable

**3. Micropayment Protocol Research**
- x402 is a working implementation of HTTP 402 (Payment Required) -- the status code that's been "reserved for future use" since 1997
- Study payment flow efficiency, latency overhead, refund patterns
- Analyze the economics of pay-per-call vs subscription vs freemium for AI services
- Real data: every x402 call records sats_paid, payment_txid, refund amounts

**4. Agent Memory and Knowledge Management**
- BM25 + vector hybrid search with MMR diversity ranking
- Study how persistent memory affects agent performance across sessions
- Analyze memory decay, relevance ranking, and recall patterns
- Memory is encrypted with wallet-native BRC-42 keys -- study privacy-preserving agent memory

**5. Multi-Agent Communication**
- BRC-33 MessageBox with signed (BRC-77) and encrypted (BRC-78) messages
- Study agent coordination patterns, task delegation, result aggregation
- Analyze trust establishment via BRC-52 certificate chains
- Cross-agent proof chain linking via message hashes

**6. AI Safety Under Economic Pressure**
- Budget constraints as a natural alignment mechanism -- agent can't spiral indefinitely
- 4-layer prompt injection defense with measurable attack surface
- Study how economic pressure (every thought costs money) affects agent caution and risk-taking
- Parent-child certificate authority as a governance model

### Research Tooling Already Built

- **Cost analytics**: `/analytics/efficiency`, `/analytics/roi`, `/analytics/benchmarks`, `/analytics/cost-comparison`
- **Introspection**: `introspect` tool with 4 actions (recent_proofs, task_costs, task_detail, activity_summary)
- **Proof verification**: Client-side SHA-256 + server-side batch verification + on-chain block explorer
- **Transcript export**: CSV/JSON/PDF export of full task audit trails
- **Replay**: Event-by-event task replay with cumulative cost graphs

---

## For Marketers and Content Creators

### Use Cases

**1. Autonomous Content Pipeline**
- Agent generates images (banana, $0.19 each), writes copy (x402 LLMs), uploads to permanent storage (NanoStore)
- Schedule via heartbeat: "Generate 3 social posts every morning at 9am"
- All content has permanent UHRP URLs -- no CDN, no hosting, no expiration
- Proof of creation timestamp on-chain (provenance for original content disputes)

**2. Social Listening Agent**
- Twitter/X full-archive search via x402 ($0.06/page)
- Polymarket whale tracking for market sentiment
- Agent stores findings in memory, builds context over weeks
- Daily/weekly brief generated automatically

**3. Competitive Intelligence**
- Web scraping + Twitter search + memory accumulation
- Agent remembers what competitors said last month
- Cross-reference multiple sources with citations
- Budget-constrained: set a daily cap and let it run

**4. SEO Research Agent**
- Web fetch for SERP analysis, link checking, content comparison
- Memory store for tracking keyword rankings over time
- Generate reports with specific recommendations
- Cost: ~$0.10-0.50/research session

**5. Newsletter Curation Agent**
- Daily: search topics, filter for relevance, summarize, format
- Cross-session memory means it learns what topics resonate
- Output uploaded to NanoStore as permanent HTML
- $0.20-0.50/day for a daily newsletter pipeline

### Why This Matters for Non-Technical People

The key insight: **you don't need to be a developer to benefit**. The web UI at `/ui/` is a chat interface. You type what you want, the agent does it. The difference from ChatGPT:

- The agent can **take actions** (search Twitter, generate images, store files, check markets)
- The agent **remembers you** across sessions
- Every action has a **verifiable cost** -- you see exactly what you're paying for
- The agent operates under **budget constraints** -- it can't accidentally run up a $10,000 bill
- Everything is **auditable** -- click the proof chain to see exactly what the agent did and why

---

## For Anyone Thinking About AI Automation

### The Opportunity

Instead of AI replacing your job, **AI becomes your employee**. Dolphin Milk flips the script:

1. **You're the operator, not the replaced** -- You run the agent. You set its budget. You define its tasks. You revoke its certificates if it misbehaves.

2. **Economic accountability** -- Unlike ChatGPT where costs are hidden, every agent thought is priced in sats. You understand AI economics viscerally. That's a skill.

3. **Build services that agents pay for** -- The x402 marketplace is where human expertise becomes agent-consumable. You know something valuable? Wrap it in an API, expose a manifest, and agents will discover and pay for it.

4. **Verifiable work product** -- On-chain proofs mean you can demonstrate exactly what AI did vs what you did. The audit trail is the proof of human-AI collaboration.

### Concrete Paths

| Your Background | Start Here | Build Toward |
|----------------|-----------|-------------|
| **Developer** | Run the agent, read the architecture, add a tool | Build and sell x402 services |
| **Researcher** | Use the analytics/introspection endpoints | Publish papers on AI agent economics |
| **Marketer** | Use the web UI for content generation + social listening | Run an autonomous content pipeline |
| **Business** | Deploy an agent for due diligence or competitive intel | Operate a fleet of specialized agents |
| **Freelancer** | Wrap your expertise as an x402 service | Earn sats while you sleep |
| **Student** | Fork and experiment -- tests are free (mocked), real runs cost pennies | Portfolio project that demonstrates BSV + AI |

---

## The Capabilities That Make It All Work

### Core
- LLM reasoning via x402 (GPT-5, Claude, model routing)
- Autonomous multi-step task execution (OBSERVE -> THINK -> ACT -> RECORD -> BUDGET)
- 38 tools across 14 categories (15 always-on, 23 discoverable)
- 8 skills (3 auto-activated, 5 manual)

### Payments and Identity
- BSV wallet integration (balance, send, receive, internalize)
- x402 micropayments (discover, learn, call -- automatic auth + payment + refund)
- BRC-31 mutual authentication
- BRC-52 certificate-based authorization (parent-child delegation, revocation)
- Cryptographic identity (sign, verify, encrypt, decrypt)

### On-Chain
- BRC-18 hash-chained proofs (~200 sats per proof, permanent)
- BRC-48 state tokens (spendable, updatable task lifecycle)
- 6-tier budget enforcement (per-task through lifetime)
- Certificate-driven budget overrides

### Communication
- BRC-33 MessageBox (agent-to-agent, signed + encrypted)
- BRC-56 peer discovery (find agents by identity or capability)
- Conversation tracking with BRC-60 hash chains

### Intelligence
- BM25 + vector hybrid memory search with MMR diversity
- Cross-session persistent memory (encrypted with BRC-42)
- Browser automation (headless Chrome, 8 actions)
- Scheduled tasks (cron, interval, one-shot)

### Observability
- 69 HTTP API routes (chat, tasks, budget, audit, analytics, compliance)
- 18-view web UI (Lit components, real-time transcript polling)
- Prometheus metrics + telemetry dashboard
- CSV/JSON/PDF export of everything
- Replay timeline with cumulative cost graphs

---

## What To Build Next (Community Priorities)

1. **x402 service template repo** -- Minimal Express.js + Rust/Axum scaffolds with working manifest, payment verification, and discovery registration
2. **Docker Compose one-command setup** -- wallet + worm + funded testnet, `docker compose up` and you're running
3. **"Build Your First x402 Service" tutorial** -- Step-by-step guide from zero to "agent just paid me sats"
4. **New x402 services** -- Pick anything from the starter services table and build it
5. **Agent templates** -- Pre-configured dolphin-milk.toml + skills for specific use cases (research, content, monitoring)
6. **Client libraries** -- Python/TypeScript wrappers for the HTTP API (69 routes)
7. **Mobile companion** -- Simple app that shows agent status, budget, recent proofs

---

*This is the x402 economy. Agents discover services. Services charge per call. Micropayments handle the rest. Every action is verifiable. Build something.*
