---
name: fleet
description: "Fleet management — spawn, monitor, and coordinate child agents via BRC-33 MessageBox and BRC-52 authorization."
auto_activate: false
tools: [fleet_status, send_message, check_inbox]
---

# Fleet Management

Coordinate multiple Dolphin Milk agents as a fleet. The parent agent spawns children, delegates tasks, monitors health, and manages budgets. Children are autonomous agents that report status via MessageBox.

## Architecture

```
Parent Agent (you)
  |
  +-- Child A (template: researcher)    status: Running
  +-- Child B (template: writer)        status: Idle
  +-- Child C (template: analyst)       status: Error
```

- **Parent** spawns and monitors children, assigns tasks, allocates budgets
- **Children** are independent Dolphin Milk instances with their own wallets
- **Communication** uses BRC-33 MessageBox (send_message / check_inbox)
- **Authorization** uses BRC-52 parent certificates (proves parent-child relationship)

## Commands

### fleet:spawn — Create a new child agent

Spawn a child agent from a template. The parent issues a BRC-52 certificate granting the child specific capabilities, and funds the child's wallet with a budget allocation.

**Inputs:**
- `name` (string, required): Human-readable name for this child agent
- `template` (string, required): Agent template — determines the child's skills and behavior (e.g. "researcher", "writer", "analyst", "monitor")
- `budget_sats` (integer, required): Satoshis to allocate to the child's wallet
- `capabilities` (string[], optional): BRC-52 capability list to grant. Default: ["tools", "llm", "memory"]

**Outputs:**
- `agent_id`: The child's identity key (66-char hex pubkey)
- `name`: The name assigned
- `template`: The template used
- `budget_sats`: Amount funded
- `endpoint`: Where the child is reachable

**Workflow:**
1. Start a new Dolphin Milk instance with the specified template
2. Issue a BRC-52 certificate to the child granting capabilities
3. Fund the child's wallet with the allocated budget
4. Register the child in fleet state
5. Return child details

### fleet:status — Get fleet status

Query the health and status of all fleet members. Uses `fleet_status` tool.

```json
fleet_status({})
```

Returns an aggregate view:
- Each member's status (Running, Idle, Error, Stopped)
- Current task assignment per member
- Budget remaining and spent per member
- Last heartbeat timestamp
- Fleet-wide totals (total budget, total spent, active tasks)

Use this before assigning new tasks to see who is available.

### fleet:assign — Assign a task to a child agent

Send a task to a specific child agent via MessageBox.

**Inputs:**
- `agent_id` (string, required): Child's identity key (66-char hex pubkey)
- `task` (string, required): Task description for the child
- `priority` (string, optional): "low", "normal", "high". Default: "normal"
- `budget_limit_sats` (integer, optional): Max sats the child may spend on this task

**Workflow:**
1. Check fleet_status to verify the child exists and is available
2. Send the task via send_message to the child's task_inbox:
```json
send_message({
  "recipient": "<agent_id>",
  "message_box": "task_inbox",
  "body": {
    "type": "task_assignment",
    "task": "<task description>",
    "priority": "<priority>",
    "budget_limit_sats": 50000
  }
})
```
3. Update fleet state with the new task assignment

### fleet:recall — Recall a child agent

Send a shutdown signal to a child agent and reclaim unspent budget.

**Inputs:**
- `agent_id` (string, required): Child's identity key (66-char hex pubkey)
- `reason` (string, optional): Why the child is being recalled

**Workflow:**
1. Send a recall message via MessageBox:
```json
send_message({
  "recipient": "<agent_id>",
  "message_box": "task_inbox",
  "body": {
    "type": "recall",
    "reason": "<reason>"
  }
})
```
2. Wait for the child to acknowledge and return unspent funds
3. Update fleet state to mark the child as Stopped

### fleet:list — List all fleet members

Quick summary of all fleet members without detailed health checks.

```json
fleet_status({})
```

Same as fleet:status but used when you just need names and IDs.

## Authorization Model

Fleet authorization uses BRC-52 parent certificates:

1. **Parent certificate**: The operator issues a cert to the parent agent with `capabilities: "all"` or `capabilities: "fleet,tools,llm,wallet,messaging"`
2. **Child certificate**: The parent issues a cert to each child with scoped capabilities (e.g. `capabilities: "tools,llm,memory"`)
3. **Verification**: Children can verify their parent's identity key matches the certifier on their certificate
4. **Revocation**: Parent can revoke a child's certificate to cut off access

Children CANNOT spawn their own children (no transitive delegation) unless explicitly granted the "fleet" capability.

## Budget Delegation

- Parent has a total fleet budget
- Each child receives an allocation at spawn time via a funding transaction
- Children track their own spending independently
- Parent monitors spending via fleet:status heartbeats
- If a child exhausts its budget, it enters Idle state and reports back
- Parent can top up a child's budget by sending additional funds

## Status Reporting

Children report status to the parent via MessageBox heartbeats sent to `status_inbox`:

```json
{
  "type": "heartbeat",
  "agent_id": "03abc...",
  "status": "Running",
  "current_task": "Researching BSV transaction fees",
  "budget_remaining_sats": 45000,
  "budget_spent_sats": 5000
}
```

The parent should check_inbox periodically to collect these heartbeats and update fleet state.

## Gotchas

- **Always check fleet:status before fleet:assign.** Don't assign tasks to Stopped or Error agents.
- **Budget is real money.** Every child spends real satoshis. Monitor spending closely.
- **MessageBox delivery is not instant.** Allow a few seconds for messages to propagate.
- **Children are autonomous.** Once assigned a task, the child runs its own agent loop. You cannot interrupt mid-task — only recall between tasks.
- **Heartbeat timeout.** If no heartbeat for 5 minutes, assume the child is unhealthy and consider recalling it.
