---
name: code-analysis
description: Code review and analysis workflow
auto_activate: false
tools: [file_read, file_search, execute_bash]
---
# Code Analysis Skill

When asked to review or analyze code:
1. Use `file_search` to find relevant files
2. Use `file_read` to examine source code
3. Use `execute_bash` to run linters, tests, or analysis tools
4. Summarize findings with specific file:line references

## Gotchas

- **Read the file before suggesting changes.** Never propose modifications to code you haven't read — context matters for correctness.
- **Check for existing tests before claiming something is untested.** Use `file_search` to look for test files (`test_*.rs`, `*_test.go`, etc.) before concluding test coverage is missing.
