---
id: mem-005
category: knowledge
tags:
  - memory
  - search
  - tantivy
  - persistence
created: "2026-03-03T08:00:00Z"
source: agent-session-50
---

The memory system uses markdown files with YAML frontmatter stored in category subdirectories (knowledge/, sessions/, execution/). Search is powered by tantivy BM25 full-text indexing.

Categories:
- knowledge: Persistent facts, patterns, insights discovered by the agent
- session: Session summaries for continuity across restarts
- execution: Tool output caches and action logs

The memory index is rebuilt from files at startup. Memory search (memory_search tool) is the primary way the agent recalls past information. The agent should proactively store important findings using memory_store.

Dedup: Post-processing checks for duplicate content before indexing. Quality scoring ensures low-quality entries are flagged.
