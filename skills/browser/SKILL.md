---
name: browser
description: "Navigate web pages, read content, and interact with elements via headless Chrome."
auto_activate: false
tools: [browser]
---

# Browser Automation

Use the `browser` tool for JavaScript-heavy sites, form filling, clicking buttons, and reading dynamic content. For simple HTTP fetches, prefer `web_fetch` (free, no Chrome needed).

## Core Pattern: Snapshot-Ref-Act

1. **Navigate** to a URL
2. **Snapshot** to get the accessibility tree with element refs (e1, e2, ...)
3. **Interact** using refs: click, type, select
4. **Snapshot again** after each interaction to see the updated state

```json
browser({"action": "navigate", "url": "https://example.com"})
browser({"action": "snapshot"})
browser({"action": "click", "ref": "e3"})
browser({"action": "snapshot"})
browser({"action": "type", "ref": "e4", "text": "hello"})
```

## Actions

| Action | Required params | Description |
|--------|----------------|-------------|
| `navigate` | `url` | Open a URL (http/https only). Invalidates refs. |
| `snapshot` | | Get accessibility tree with interactive element refs. |
| `click` | `ref` | Click an element by ref. Invalidates refs. |
| `type` | `ref`, `text` | Type text into a field by ref. Invalidates refs. |
| `select` | `ref`, `value` | Select dropdown option by text or value. Invalidates refs. |
| `evaluate` | `expression` | Run JavaScript and return the result. |
| `screenshot` | | Save a PNG screenshot to workspace. |
| `close` | | Close current page. Use `page: "all"` to shut down Chrome. |

## Tips

- **Use URLs exactly as given** — NEVER correct, fix typos, or modify URLs the user provides. If they say `teragun.com`, navigate to `teragun.com`, not `theragun.com`.
- **Always snapshot after interactions** — refs are invalidated by click/type/select/evaluate.
- **Refs like `[e1]`** only appear on interactive elements (buttons, links, inputs, etc.).
- **Stale ref error** means you need a fresh snapshot.
- **Chrome launches lazily** on first `navigate` (~2-3s cold start).
- **Chrome auto-closes at task end** — you don't need to call close manually, but you can to free resources mid-task.
- **Multiple pages**: Pass `"page": "mypage"` to navigate/snapshot/close named pages (default is "main", max 3).

## When NOT to Use

- Simple HTTP GET → use `web_fetch` (free, faster)
- API calls → use `x402_call` or `web_fetch`
- Static HTML → use `web_fetch`

## Gotchas

- **ALWAYS snapshot before clicking or typing.** You need ref IDs from the accessibility tree — without a fresh snapshot, you have no valid refs.
- **Use ref IDs from snapshot (e.g. `[e1]`, `[e2]`), NOT CSS selectors.** The browser tool only accepts refs like `"e3"`, not selectors like `".btn-submit"`.
- **The browser tool is discoverable via `search_tools`, not always-on.** It won't appear in your tool list until you search for it.
- **`file://` and `javascript:` URLs are blocked** for security. Only `http://` and `https://` are allowed.
- **Max 3 concurrent pages — close pages when done.** Use `browser({"action": "close"})` to free resources, or `"page": "all"` to shut down Chrome entirely.
