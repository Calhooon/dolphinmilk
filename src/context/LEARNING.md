# Learning — Context

## Lessons Learned
- **[2026-02-24]** The `build_messages()` truncation logic has a subtle ordering: it keeps the last 4 messages unconditionally, then fills from oldest forward with whatever fits. This means mid-conversation messages are the first to be dropped — not the oldest. If you're debugging missing context, check whether important messages landed in the "middle zone" that gets cut first.
- **[2026-02-24]** `estimate_tokens()` returns `max(1, len/4)` — the `.max(1)` prevents zero-token messages from slipping through budget checks uncounted. An empty string still costs 1 token in the budget. The 4-token per-message overhead in `estimate_message_tokens()` is separate from this.
- **[2026-02-24]** `PromptContext` is assembled by the runner, not by this module. If the prompt is wrong (e.g. missing tools, stale balance), the bug is in how runner populates `PromptContext`, not in `prompt.rs`.
- **[2026-02-24]** The messaging nudge (`inbox_count > 0`) was the key fix for cross-wallet conversation. Without it, the LLM would "reply" with plain text that went nowhere. The nudge plus the IMPORTANT block in `section_messaging` work together — removing either breaks agent-to-agent chat.

## Debugging Insights
- **[2026-02-24]** If the LLM seems to "forget" earlier context, check `build_messages()` output length vs `max_tokens`. The truncation marker `[N earlier messages truncated]` is injected as a system message — grep logs for "truncated" to confirm.
- **[2026-02-24]** Offloaded files go to `offload_dir` with sequential names like `label_0001.txt`. The counter is per-`ContextManager` instance, so restarting the process resets it. Old offloaded files from previous runs are not cleaned up automatically.
- **[2026-02-24]** When `truncate_messages()` truncates content, it uses `content[..max_chars]` which can split mid-UTF8 if the content contains multi-byte characters. Not yet a problem in practice since tool results are mostly ASCII, but worth knowing.

## Pattern Notes
- **[2026-02-24]** The "sandbox-first framing" from `section_identity` is deliberate — telling the LLM it "has a computer and a wallet" produces measurably better tool-use than chatbot framing. Don't weaken this language.
- **[2026-02-24]** Empty sections are filtered out via `.filter(|s| !s.is_empty())` in `build_system_prompt()`. This means `section_tools` and `section_memory` return `String::new()` when they have nothing — they don't return a header with no content. New sections should follow this pattern.
- **[2026-02-24]** `section_wallet` hardcodes `localhost:3322`. If wallet port becomes configurable, this prompt text needs updating too — it's easy to change the wallet config and forget the prompt still says 3322.
- **[2026-02-24]** The `low_power` flag only adds a warning string to the prompt. It doesn't change model selection or enforce anything. The LLM may ignore it. Actual enforcement happens in `budget.rs`.
- **[2026-02-24]** Available files are capped at 20 in the prompt (`take(20)`). If the agent needs a file not listed, it must use search tools to find it — the prompt explicitly says "search/read as needed".
