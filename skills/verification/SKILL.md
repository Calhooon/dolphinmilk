---
name: verification
description: "Double-check outputs by re-running queries and comparing results. Use when accuracy is critical or when you suspect an error in a previous tool result."
auto_activate: false
tools: [verify_output]
---

# Verification Skill

When accuracy is critical, use this skill to verify outputs by re-execution and comparison.

## When to Verify

- After getting unexpected or surprising results
- When the user explicitly asks for verification
- When working with numerical calculations or data lookups
- Before making irreversible decisions based on tool output

## Verification Process

1. **Record the original result**: Note the tool name, parameters, and output
2. **Re-execute**: Run the same query again with identical parameters using `verify_output`
3. **Compare**: The tool will automatically compare outputs and flag discrepancies
4. **Report**: If results differ, investigate why (timing, randomness, external state changes)

## Interpreting Results

- **Match**: Both executions returned the same result — high confidence
- **Mismatch**: Results differ — investigate the cause before proceeding
- **Acceptable variance**: Some tools have inherent variability (e.g., search rankings, timestamps). Minor differences in these are expected.

## Best Practices

- Verify critical tool outputs, not every single call
- For non-deterministic tools (search, LLM calls), focus on structural consistency rather than exact match
- If verification reveals an error in your previous reasoning, correct it explicitly
- Record verification results for audit trail via `memory_store`
