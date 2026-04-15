# src/replay/
> Interactive replay and fork execution from JSONL transcripts.

## Overview

Provides two capabilities for working with completed task transcripts: (1) structured replay with enriched metadata (cost tracking, tool usage, iteration numbering, elapsed time), and (2) fork execution that reconstructs conversation state up to a given event index so a new agent run can branch from that point. Both components read from the append-only JSONL transcripts produced by `session/transcript.rs` and are consumed by the HTTP API via `server/handlers/replay.rs`.

## Files

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 14 | Module declarations, re-exports all public types from `viewer` and `fork` |
| `viewer.rs` | 344 | `ReplayViewer` — reads a transcript and builds a `ReplayTimeline` with per-event metadata (cumulative cost, iteration, elapsed time, tool name, model), cost timeline (spending events only), and tool usage entries (call→result pairing). Also supports single-event and range-based access. |
| `fork.rs` | 218 | `ForkExecutor` — reconstructs OpenAI-format conversation messages from a transcript up to a given event index. Produces `ForkResult` with `prior_messages`, original task description, iteration count, and cumulative cost at the fork point. |

## Key Exports

### From `viewer.rs`

| Type | Description |
|------|-------------|
| `ReplayViewer` | Main entry point. Wraps a `Transcript` and provides `build_timeline()`, `get_event(index)`, `get_events_range(from, to)`, and `event_count()`. |
| `ReplayEvent` | Single event with enriched metadata: `index`, `timestamp`, `event_type`, `id` (8-char UUID prefix), `iteration`, `event_sats`, `cumulative_sats`, `tool_name`, `model`, `elapsed_secs`, `data`. |
| `ReplayTimeline` | Complete structured timeline: `task_id`, `events`, `cost_timeline`, `tool_usage`, `total_sats`, `total_events`, `total_iterations`, `duration_secs`. |
| `CostTimelinePoint` | A point on the spending curve: `elapsed_secs`, `cumulative_sats`, `event_type`, `index`. Only created for events that spend sats. |
| `ToolUsageEntry` | Tool invocation summary: `name`, `iteration`, `call_index`, `result_index`, `success`, `sats_paid`. Call→result pairing matches by tool name (most recent unresolved call). |

### From `fork.rs`

| Type | Description |
|------|-------------|
| `ForkExecutor` | Stateless struct with a single method `prepare_fork(transcript_path, params)`. Validates the transcript exists and the event index is in bounds. |
| `ForkParams` | Fork request parameters: `event_index` (zero-based, exclusive upper bound), optional `message`, `model`, `max_iterations`. Deserializable from JSON. |
| `ForkResult` | Reconstructed state: `original_task` (from first `user` event), `prior_messages` (OpenAI-format `Vec<Value>`), `events_consumed`, `fork_iteration`, `original_sats_at_fork`. |

## Cost Tracking

Both `ReplayViewer` and `ForkExecutor` extract per-event costs from transcript data using the same logic:

- `think_response` events: cost = `data.sats_effective` (net of refunds)
- `tool_result` events: cost = `data.sats_paid` (x402 tool calls)
- All other event types: cost = 0

Iteration counting increments on each `think_request` event.

## Message Reconstruction (fork.rs)

`build_messages_up_to()` converts transcript events to OpenAI-format messages, handling 5 event types:

| Event Type | OpenAI Role | Notes |
|------------|-------------|-------|
| `system` | `system` | System prompt content |
| `user` | `user` | User/task messages |
| `think_response` | `assistant` | Includes `tool_calls` array if present; tracks pending call IDs |
| `tool_result` | `tool` | Only emitted if a matching `tool_call_id` exists in pending calls |
| `loop_warning` | `system` | Loop detector warnings injected as system messages |

Orphaned `tool_result` events (no matching pending call) are silently skipped, matching the behavior in `session/transcript.rs`.

## HTTP API Integration

Two routes in `server/handlers/replay.rs` consume this module:

| Method | Path | Handler | Description |
|--------|------|---------|-------------|
| GET | `/task/{id}/replay` | `get_replay` | Returns `ReplayResponse` with full timeline, cost curve, and tool usage |
| POST | `/task/{id}/fork` | `fork_task` | Accepts `ForkRequest`, prepares fork via `ForkExecutor`, spawns new task via `spawn_task()`, writes `fork_context.json` metadata |

The fork handler writes `tasks/{new_task_id}/fork_context.json` containing `source_task_id`, `event_index`, `fork_iteration`, `original_sats_at_fork`, `events_consumed`, and `prior_messages`. The forked task is tagged with `["fork", "fork-of:{source_id}"]`.

## Usage

### Replay a completed task

```rust
use bsv_worm::replay::ReplayViewer;

let viewer = ReplayViewer::new(Path::new("workspace/tasks/abc123/session.jsonl"));
let timeline = viewer.build_timeline();

println!("Total cost: {} sats over {} iterations in {:.1}s",
    timeline.total_sats, timeline.total_iterations, timeline.duration_secs);

for tool in &timeline.tool_usage {
    println!("  {} at iter {} — {} sats", tool.name, tool.iteration, tool.sats_paid);
}
```

### Fork from a specific point

```rust
use bsv_worm::replay::{ForkExecutor, ForkParams};

let params = ForkParams {
    event_index: 8,  // fork after event 7
    message: Some("Try a different approach".into()),
    model: None,
    max_iterations: Some(10),
};

let result = ForkExecutor::prepare_fork(
    Path::new("workspace/tasks/abc123/session.jsonl"),
    &params,
)?;

// result.prior_messages can be injected into a new WormLoop
println!("Forking at iteration {}, {} sats spent so far",
    result.fork_iteration, result.original_sats_at_fork);
```

## Internal Helpers

| Function | File | Description |
|----------|------|-------------|
| `extract_event_sats(event)` | viewer.rs | Returns sats for a single transcript event (think_response or tool_result) |
| `extract_tool_name(event)` | viewer.rs | Returns tool name from tool_call or tool_result events |
| `extract_model(event)` | viewer.rs | Returns model name from think_request or think_response events |
| `extract_task_id(path)` | viewer.rs | Derives task ID from transcript path (parent directory name) |
| `build_messages_up_to(events, index)` | fork.rs | Converts transcript events to OpenAI-format messages up to a given index |

## Related

- [`../session/CLAUDE.md`](../session/CLAUDE.md) — `Transcript` struct that this module reads from (`replay()`, `event_count()`)
- [`../server/handlers/CLAUDE.md`](../server/handlers/CLAUDE.md) — HTTP handlers that expose replay and fork endpoints
- [`../runner/CLAUDE.md`](../runner/CLAUDE.md) — `WormLoop` that accepts `prior_messages` for forked runs
- [`../../tests/CLAUDE.md`](../../tests/CLAUDE.md) — `test_replay.rs` covers ReplayViewer, ForkExecutor, and server routes
