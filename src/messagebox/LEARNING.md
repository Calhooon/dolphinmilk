# Learning — messagebox

## Lessons Learned
- **[2026-02-24]** The `unwrap_message_body()` function exists because the MessageBox server double-wraps bodies as `{"message": <original>}` and sometimes JSON-encodes that as a string. Both cases must be handled: parse string first, then strip the `"message"` wrapper. If your typed message struct has a top-level `"message"` key, it will be silently stripped — all typed structs use `"type"` at top level to avoid this.
- **[2026-02-24]** `acknowledge_message` is a destructive delete, not a "mark as read". There is no way to re-read an acknowledged message. Always process the message body *before* acknowledging.
- **[2026-02-24]** `send_message` hard-errors if the delivery quote requires payment (BRC-29 body-transport payment not yet implemented). Cross-wallet messaging only works today because the default fee is 0. This will break silently the moment any recipient calls `set_permission` with a nonzero fee.
- **[2026-02-24]** The quote endpoint returns fee data in two different shapes — nested under a `"quote"` key, or as top-level `deliveryFee`/`recipientFee` fields. `client.rs:66-81` handles both, but if you mock it in tests, pick one format and be consistent.

## Debugging Insights
- **[2026-02-24]** A `recipient_fee` of exactly `-1` means the sender is blocked (not an error). `DeliveryQuote::is_blocked()` checks this. If sends fail with a confusing "payment required" error, check whether the recipient set permissions — a `-1` fee won't trigger `requires_payment()` but will trigger `is_blocked()` separately.
- **[2026-02-24]** All MessageBox requests go through `AuthriteClient`. If you get auth errors, the problem is in `src/auth/`, not here. The Authrite session must be initialized (initial request handshake) before any MessageBox call will succeed.
- **[2026-02-24]** `poll_inboxes()` swallows per-box errors with `tracing::warn` and continues to the next box. If messages seem to be missing, check logs — one box may be failing silently while others succeed.

## Pattern Notes
- **[2026-02-24]** Error responses from the server use either `"description"` or `"message"` for the error text (inconsistent server behavior). Every error-handling block in `client.rs` does `.get("description").or(.get("message"))` — follow this pattern for any new endpoints.
- **[2026-02-24]** The four inbox boxes (`task_inbox`, `results_inbox`, `status_inbox`, `worm_coordination`) have a defined priority order in `INBOX_BOXES`. `poll_inboxes()` iterates in this order. If you add a new box, insert it at the right priority position in `types.rs:26-31`.
- **[2026-02-24]** `send_message` generates a UUID per message and injects it as `sentMessageId` into the response. This is the only way the caller can correlate a sent message with later acknowledgements — the server doesn't echo it back in a predictable field.
