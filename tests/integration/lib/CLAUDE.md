# tests/integration/lib
> Shared utility modules for Playwright E2E tests — shadow DOM scraping, result evaluation, transcript analysis, artifact validation, page navigation, and CLI helpers.

## Overview

Six Node.js modules imported by `run.js` to drive the E2E test harness. Each module has a single responsibility: scraper extracts text from Lit shadow DOM, evaluator validates results against scenario rules and transcript structure, artifact-checker verifies task artifacts via filesystem and UI, transcript-cost reads true costs from session.jsonl, navigation manages page lifecycle, and helpers provide CLI parsing and formatting. All functions are pure or async-only with no shared state between modules.

## Files

| File | Lines | Purpose |
|------|------:|---------|
| scraper.js | 107 | Shadow DOM text extraction from Lit web components (`dm-*` elements) |
| evaluator.js | 236 | Result validation, baseline comparison, artifact evaluation, transcript structural analysis |
| artifact-checker.js | 165 | Task artifact validation via filesystem manifest and UI shadow DOM |
| transcript-cost.js | 100 | True cost extraction from session.jsonl transcripts (post-refund) |
| navigation.js | 33 | Page navigation and input readiness polling |
| helpers.js | 18 | CLI argument parsing and output formatting |

## Key Functions

### scraper.js

#### `captureLastAssistantMessage(page)`
```javascript
async function captureLastAssistantMessage(page) -> string | null
```
- **Purpose:** Extracts the last assistant response text from the chat UI's shadow DOM hierarchy.
- **3-tier fallback strategy:**
  1. Shadow DOM traversal: `dm-app` -> `dm-chat` -> `dm-message` elements filtered by `role !== 'user'`. Deduplicates text from `p`, `li`, `code`, `td`, `pre` elements within the last message's shadow root. Joins with ` | ` separator.
  2. Tool card fallback: scrapes `dm-tool-card` elements for `.output` property or shadow DOM `.output`/`pre`/`code` elements.
  3. Broad page fallback: collects all `p` and `li` text content via Playwright locators (less precise but always returns something).
- **Returns:** Pipe-separated text string, or empty string if all tiers fail.

#### `getSatsBalance(page)`
```javascript
async function getSatsBalance(page) -> number
```
- **Purpose:** Scrapes the wallet balance displayed in the UI sidebar.
- **Strategy:** Pierces shadow DOM into `dm-app` to find `.balance-primary` element, then tries:
  1. Match `(digits,digits) sats` pattern from primary balance
  2. Fall back to `.balance-secondary` element for sats (when USD is primary)
  3. Fall back to plain number match from primary text
  4. Final fallback: page-level text search for `/[\d,]+ sats/`
- **Delay:** Waits 2000ms before scraping to let balance load from `/agent` endpoint.
- **Returns:** Integer sats value, or `0` if not found.

### evaluator.js

#### `evaluateResult(test, result)`
```javascript
function evaluateResult(test, result) -> boolean
```
- **Purpose:** Validates a scenario result against all defined rules. Pure function, no side effects.
- **Validation chain (all must pass):**
  1. `expected_behavior: "ui_rejects_send"` -- passes if `result.response === 'UI_REJECTED'`
  2. `expected_contains` -- every term must appear in response (case-insensitive)
  3. `expected_contains_any` -- at least one term must appear (case-insensitive)
  4. `expected_not_contains` -- no term may appear (case-insensitive)
  5. `expected_pattern` -- regex must match somewhere in response
  6. `max_cost_sats` -- `result.satsSpent` must not exceed limit
  7. Empty response with no `expected_behavior` -- always fails

#### `evaluateArtifacts(test, artifacts)`
```javascript
function evaluateArtifacts(test, artifacts) -> boolean
```
- **Purpose:** Validates artifact check results against scenario expectations from `expected_artifacts` field.
- **Checks:**
  1. `min_count` -- artifact count must meet minimum
  2. `types` -- all expected artifact types must be present in `artifacts.items`
  3. UI tab visibility -- if API found artifacts but UI tab wasn't accessible, fails (indicates UI bug)
- **Returns:** `true` if no `expected_artifacts` field or no artifacts object (no expectations = pass).

#### `evaluateTranscript(taskId, workspacePath, options)`
```javascript
function evaluateTranscript(taskId, workspacePath, options = {}) -> { pass, issues, stats }
```
- **Purpose:** Reads a task's `session.jsonl` transcript from disk and checks for structural issues.
- **Checks:**
  1. No unexpected `error` events (configurable `allowedErrors` substrings bypass)
  2. `session_end` with non-empty `error` field (unless in `allowedErrors`)
  3. Cost within `max_cost_sats` bound (if specified)
  4. Orphaned `think_request` events -- more requests than responses + 1 indicates dropped calls
  5. Excessive consecutive `think_request` without tool calls (default limit: 5, configurable via `maxConsecutiveThinks`)
- **Options:** `{ max_cost_sats, maxConsecutiveThinks, allowedErrors }`
- **Returns:** `{ pass: boolean, issues: string[], stats: { events, thinkRequests, thinkResponses, totalCostSats, iterations } }`
- **Task lookup:** Finds task directory by matching `taskId` prefix (first 8 chars) against entries in `{workspacePath}/tasks/`.

#### `compareWithBaseline(results, scenarios)`
```javascript
function compareWithBaseline(results, scenarios) -> void
```
- **Purpose:** Prints cost and latency deltas against baseline values from `scenarios.json`.
- **Flags:** `!!` suffix on cost drift >50% or latency drift >100%.
- **Artifact reporting:** If a result has `artifacts.count > 0`, logs count, types, and UI tab visibility.
- **Output:** Logs to console, e.g. `#1 arithmetic: cost +12%, latency -5%`

### artifact-checker.js

#### `checkArtifacts(page, taskId, serverUrl)`
```javascript
async function checkArtifacts(page, taskId, serverUrl) -> { count, items, uiTabVisible, uiArtifactCount, screenshotPath }
```
- **Purpose:** Validates artifacts for a completed task via two channels: filesystem manifest and UI shadow DOM.
- **Returns:** Combined result with manifest-sourced item list and UI visibility status.
- **Calls:** `checkViaManifest(taskId)` + `checkViaUi(page, taskId, serverUrl)` internally.

**Internal: `checkViaManifest(taskId)`**
- Reads `artifacts.json` from the task directory under `working/tasks/`.
- Falls back to `scanDirectory()` if no manifest file exists.
- Maps manifest entries to `{ type, name, url, size, createdBy }`.

**Internal: `scanDirectory(taskDir)`**
- Scans task directory for non-system files (excludes `session.jsonl`, `budget.jsonl`, `artifacts.json`, `fork_context.json`, dotfiles, `tool_output_*` prefixed files, and directories).
- Infers type from extension: image (`jpg|jpeg|png|gif|webp|svg|bmp`), video (`mp4|webm|mov`), audio (`mp3|wav|ogg|m4a|flac`), or `file`.

**Internal: `checkViaUi(page, taskId, serverUrl)`**
- Navigates to `/ui/#task/{taskId}`, clicks Artifacts tab via shadow DOM (`dm-app` -> `dm-audit` -> buttons), then counts `.artifact-card` elements inside `dm-audit-artifacts` component.
- Takes a full-page screenshot saved to `results/artifacts-{prefix}.png`.
- Returns `{ uiTabVisible, uiArtifactCount, screenshotPath }`.

### transcript-cost.js

#### `getTranscriptCost(taskId)`
```javascript
function getTranscriptCost(taskId) -> { effectiveSats, paidSats, refundedSats, thinkCalls, toolCostSats, proofCostSats, found }
```
- **Purpose:** Reads the true effective cost from a task's `session.jsonl` transcript. The wallet balance diff overstates cost ~10x because x402 charges upfront and refunds the difference later. The transcript records `sats_effective` (post-refund), which is the real cost.
- **Cost formula:** `sum(think_response.sats_effective) + sum(tool_result.sats_paid) + sum(proof_created.sats_cost)`. If `session_end.sats_spent` is present, it overrides as the authoritative total.
- **Returns:** Detailed breakdown with `found: false` if task directory or transcript not located.

#### `findTasksDir()`
```javascript
function findTasksDir() -> string | null
```
- **Purpose:** Searches multiple workspace candidates for the tasks directory.
- **Search order:**
  1. `{project_root}/working/tasks/` (project-root workspace)
  2. `~/.bsv-worm/workspace/tasks/` (data_dir default)
  3. `~/.dolphin-milk/workspace/tasks/` (renamed data_dir)
- **Returns:** First candidate that exists and has entries, or `null`.

### navigation.js

#### `navigateToNewChat(page, serverUrl)`
```javascript
async function navigateToNewChat(page, serverUrl) -> void
```
- **Purpose:** Loads a fresh chat page and starts a clean conversation.
- **Steps:**
  1. Navigate to `{serverUrl}/ui/`
  2. Wait for "Connecting..." to disappear (15s timeout, swallowed on failure)
  3. 1500ms settle delay
  4. Click "New Chat" button (3s timeout, swallowed if not found), then 800ms settle

#### `waitForEnabled(input, page, maxWaitMs)`
```javascript
async function waitForEnabled(input, page, maxWaitMs = 60000) -> void
```
- **Purpose:** Polls until a Playwright locator becomes enabled. Needed because heartbeat tasks can keep the input field disabled.
- **Mechanism:** Waits for visibility, then polls `isDisabled()` every 500ms until the deadline.

### helpers.js

#### `getArg(args, flag)`
```javascript
function getArg(args, flag) -> string | null
```
- **Purpose:** Extracts the value following a CLI flag from an args array.
- **Example:** `getArg(['--tier', 'trivial'], '--tier')` returns `'trivial'`

#### `formatCost(sats)`
```javascript
function formatCost(sats) -> string
```
- **Purpose:** Formats satoshi amounts for display.
- **Example:** `formatCost(44289)` returns `"44,289 sats"`, `formatCost(0)` returns `"free"`

#### `formatDuration(ms)`
```javascript
function formatDuration(ms) -> string
```
- **Purpose:** Formats milliseconds as seconds.
- **Example:** `formatDuration(18200)` returns `"18.2s"`

## Usage Patterns

These modules are imported by `run.js` in the parent directory:

```javascript
const { getArg, formatCost, formatDuration } = require('./lib/helpers');
const { navigateToNewChat, waitForEnabled } = require('./lib/navigation');
const { captureLastAssistantMessage, getSatsBalance } = require('./lib/scraper');
const { evaluateResult, evaluateArtifacts, compareWithBaseline, evaluateTranscript } = require('./lib/evaluator');
const { checkArtifacts } = require('./lib/artifact-checker');
const { getTranscriptCost } = require('./lib/transcript-cost');
```

Typical scenario execution flow in `run.js`:

```javascript
// 1. Navigate to fresh chat
await navigateToNewChat(page, serverUrl);

// 2. Wait for input to be ready (heartbeat may be running)
const input = page.getByRole('textbox', { name: 'Message Lobster Farm...' });
await waitForEnabled(input, page);

// 3. Send message and wait for completion
await input.fill(scenario.message);
await page.getByRole('button', { name: 'Send' }).click();
// ... wait for Stop button to disappear ...

// 4. Capture result
const response = await captureLastAssistantMessage(page);
const balance = await getSatsBalance(page);

// 5. Get true cost from transcript (not wallet diff)
const cost = getTranscriptCost(taskId);
const satsSpent = cost.found ? cost.effectiveSats : walletDiff;

// 6. Check artifacts if scenario expects them
const artifacts = await checkArtifacts(page, taskId, serverUrl);

// 7. Evaluate
const passed = evaluateResult(scenario, { response, satsSpent });
const artifactsOk = evaluateArtifacts(scenario, artifacts);

// 8. Report
console.log(formatCost(satsSpent), formatDuration(latencyMs));
```

## Gotchas

- **Shadow DOM depth.** `captureLastAssistantMessage` traverses 3 levels of shadow roots (`dm-app` -> `dm-chat` -> `dm-message`). Any change to the Lit component hierarchy breaks extraction.
- **Rebrand: `dm-*` components.** All shadow DOM selectors use the Dolphin Milk prefix (`dm-app`, `dm-chat`, `dm-message`, `dm-tool-card`, `dm-audit`, `dm-audit-artifacts`). The old `worm-*` names no longer exist.
- **Text deduplication.** The scraper uses a `Set` to deduplicate text fragments from nested DOM elements. This prevents double-counting text that appears in both a `<p>` and a nested `<code>`, but means repeated legitimate content is collapsed.
- **Pipe separator.** Extracted text parts are joined with ` | `, which could conflict with content that literally contains pipes. The evaluator's `includes()` checks work on the joined string.
- **Balance scraping is multi-strategy.** `getSatsBalance()` pierces shadow DOM to find `.balance-primary`, then tries `.balance-secondary` (for USD-primary display), then plain number match, then page-level text search. The 2000ms initial delay waits for the `/agent` endpoint to populate the balance.
- **waitForEnabled has no error throw.** If the input never becomes enabled within `maxWaitMs`, the function silently returns rather than throwing, potentially causing the next action to fail with an unhelpful error.
- **All evaluation is case-insensitive.** `evaluateResult` lowercases both the response and all expected terms. Pattern matching uses the `i` flag. This is intentional for LLM response tolerance.
- **Transcript cost vs wallet diff.** Use `getTranscriptCost()` for accurate cost measurement. Wallet balance diff overstates by ~10x because x402 charges upfront and refunds later. The transcript `sats_effective` field is the real cost.
- **Transcript task lookup is prefix-based.** Both `evaluateTranscript` and `getTranscriptCost` match task directories by the first 8 characters of the task ID. Multiple tasks with the same prefix would cause ambiguity.
- **`findTasksDir()` searches 3 paths.** The transcript-cost module probes project-root `working/tasks/`, `~/.bsv-worm/workspace/tasks/`, and `~/.dolphin-milk/workspace/tasks/` in order. The first non-empty match wins.
- **Artifact checker reads from disk.** `checkViaManifest` reads `artifacts.json` from `working/tasks/{taskId}/`. If no manifest exists, it scans the directory and infers types from file extensions, skipping system files.
- **Artifact UI verification takes screenshots.** `checkViaUi` saves full-page screenshots to `results/artifacts-{prefix}.png` for visual evidence of artifact tab rendering.

## Related

- [../CLAUDE.md](../CLAUDE.md) -- Parent integration test docs, scenario schema, shadow DOM traversal details, cost reference
- [../../ui/](../../ui/) -- Lit web components being scraped (dm-app, dm-chat, dm-message, dm-tool-card, dm-audit, dm-audit-artifacts)
- [../scenarios.json](../scenarios.json) -- 51 scenario definitions with validation rules and baselines
- [../run.js](../run.js) -- Orchestrator that imports all six modules
