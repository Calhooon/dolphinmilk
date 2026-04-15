# Fleet Tools
> Status aggregation and coordination types for multi-agent fleet management.

## Overview

One discoverable tool (`fleet_status`) that aggregates child agent statuses into a fleet-wide summary. The module also defines the core fleet types (`FleetMember`, `FleetStatus`, `TaskAssignment`, `AgentStatus`, `TaskPriority`) used by the fleet coordination system. Discoverable via `search_tools`, NOT always-on. Category: `fleet`.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 332 | `all_fleet_tools()` constructor, `fleet_status` tool definition, `parse_members_from_params()` JSON→struct parser, `fleet_status_impl()` async handler (includes inline tests) |
| `types.rs` | 202 | `AgentStatus` enum (5 variants), `FleetMember` struct (9 fields), `FleetStatus` struct (4 fields), `TaskAssignment` struct (4 fields), `TaskPriority` enum (3 variants) — all `Serialize`/`Deserialize` (includes inline tests) |
| `status.rs` | 351 | `aggregate_fleet_status()` — heartbeat timeout detection + budget totals + active task counting. `format_fleet_status()` — human-readable summary. `HEARTBEAT_TIMEOUT_SECS = 300` (includes inline tests) |

## Key Exports

### Tool

| Tool | Parameters | Description |
|------|-----------|-------------|
| `fleet_status` | `members` (array of member objects) | Aggregate fleet member status. Returns both structured JSON (`fleet_status` field) and human-readable text (`display` field). Members with stale heartbeats (>5 min) are marked Unknown. |

Constructor: `all_fleet_tools()` returns `Vec<ToolDef>` with 1 tool. No captured state — the tool is fully stateless. `ToolDef` fields: `cleanup: None`, `deferred: true`, `always_load: false`, `search_hint: "Multi-agent fleet status aggregation"`.

### Types

**`AgentStatus`** — tagged enum (`#[serde(tag = "state", content = "detail")]`):
- `Running` — actively executing a task
- `Idle` — alive but no current task
- `Error(String)` — error with detail message
- `Stopped` — recalled or shut down
- `Unknown` — no heartbeat within timeout window

**`FleetMember`** — a single agent in the fleet:
- `agent_id: String` — 66-char hex compressed pubkey
- `name: String` — human-readable name
- `template: String` — agent template (e.g. "researcher", "writer")
- `status: AgentStatus`
- `current_task: Option<String>`
- `budget_remaining_sats: u64`
- `budget_spent_sats: u64`
- `last_heartbeat: DateTime<Utc>`
- `endpoint: String` — MessageBox identity key or URL

**`FleetStatus`** — aggregate fleet summary:
- `members: Vec<FleetMember>` — all members (with status corrections applied)
- `total_budget_sats: u64` — sum of (remaining + spent) across all members
- `total_spent_sats: u64` — sum of spent across all members
- `active_tasks: usize` — count of Running members with a current_task

**`TaskAssignment`** — task dispatched to a child agent:
- `agent_id: String`, `task: String`, `priority: TaskPriority`, `budget_limit_sats: Option<u64>`

**`TaskPriority`** — `Low`, `Normal` (default), `High`. Serializes as lowercase (`#[serde(rename_all = "lowercase")]`).

### Functions

**`aggregate_fleet_status(members, now) -> FleetStatus`** (status.rs):
- Clones each member, applies heartbeat timeout check (marks non-Stopped members as Unknown if heartbeat > 300s old)
- Accumulates budget totals using `saturating_add` (overflow-safe)
- Counts active tasks (Running + has current_task)

**`format_fleet_status(status) -> String`** (status.rs):
- Header line: member count + active task count
- Budget line: allocated / spent / remaining
- Per-member lines: `name (agent_id[..8]) [template] - status: X, task: Y, budget: remaining/total sats`
- Empty fleet: "No fleet members."

**`parse_members_from_params(params) -> Vec<FleetMember>`** (mod.rs):
- Parses JSON array under `members` key
- `agent_id` is required — members without it are silently skipped
- All other fields have defaults: name="unnamed", template="default", status=Unknown, budgets=0, heartbeat=now, endpoint=""
- Error status parsed from `"Error: <detail>"` string prefix

## Usage

The `fleet_status` tool is invoked by the agent LLM to monitor child agents. The fleet skill (`skills/fleet/SKILL.md`) guides usage patterns:

```json
// Empty call — verify fleet system is operational
fleet_status({})

// With member data collected from child agents
fleet_status({
  "members": [
    {
      "agent_id": "034aa44668fbc73ca...",
      "name": "researcher-1",
      "template": "researcher",
      "status": "Running",
      "current_task": "Analyzing fees",
      "budget_remaining_sats": 45000,
      "budget_spent_sats": 5000,
      "last_heartbeat": "2026-03-23T12:00:00Z",
      "endpoint": "034aa44668fbc73ca..."
    }
  ]
})
```

Response format:
```json
{
  "fleet_status": {
    "members": [...],
    "total_budget_sats": 50000,
    "total_spent_sats": 5000,
    "active_tasks": 1
  },
  "display": "Fleet: 1 members, 1 active tasks\nBudget: 50000 sats allocated, 5000 sats spent (45000 sats remaining)\n\n  researcher-1 (034aa446) [researcher] - status: Running, task: Analyzing fees, budget: 45000/50000 sats"
}
```

## Related

- [../CLAUDE.md](../CLAUDE.md) — parent tools directory, registry, and tool conventions
- `skills/fleet/SKILL.md` — fleet coordination skill (references `fleet_status`, `send_message`, `check_inbox`)
- `src/tools/registry.rs` — `ToolDef` struct and `ToolRegistry` where fleet tools are registered
- `src/tools/messagebox_tools.rs` — `send_message` / `check_inbox` used alongside fleet_status for agent-to-agent coordination
- `src/discovery.rs` — BRC-56 peer discovery for finding fleet members
- `src/certificates.rs` — BRC-52 certificates for parent-child authorization
