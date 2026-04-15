# Contributing to Dolphin Milk

Thanks for your interest in contributing! Dolphin Milk is an autonomous AI agent that pays for its own LLM inference via BSV micropayments. We welcome contributions that improve the agent, its protocols, and its tooling.

## Contributor License Agreement

By submitting a pull request, you agree to the Contributor License Agreement (CLA). A CLA bot will be added in the future to automate this check.

## How to Contribute

1. Fork the repository
2. Create a feature branch (`git checkout -b my-feature`)
3. Make your changes
4. Add tests for new functionality
5. Ensure all tests pass: `cargo test` (~1800+ tests)
6. Ensure clippy is clean: `cargo clippy -- -D warnings`
7. Commit with a descriptive message
8. Push and open a pull request

## What We Accept

- Bug fixes
- BRC protocol improvements and new implementations
- Performance improvements
- Documentation fixes
- Test improvements and coverage
- Tool improvements (new tools, better error handling)
- Memory and search improvements
- UI fixes and polish

## What Belongs Elsewhere

These features belong in the hosted platform, not this repo:

- Multi-tenant features
- Billing and payment UI (Stripe, credit ledger)
- Admin dashboards
- User management and SSO
- Cloud deployment infrastructure

## Code Style

- Rust 2021 edition, 1.85+
- Use `thiserror` for error types (`WormError` enum in `error.rs`)
- One test file per module in `tests/` (e.g., `tests/test_wallet.rs`)
- Use `tempfile` for filesystem tests, `mockito` for HTTP mocking
- Valid secp256k1 public keys in tests (use the generator point G), never fake hex strings
- Tool pattern: `ToolDef { name, description, parameters, execute, category }` with stateless closures

## Development Setup

### Prerequisites

- Rust 1.85+ (2021 edition)
- [bsv-wallet-cli](https://github.com/nicholasgasior/bsv-wallet-cli) running on `localhost:3322`
- BSV wallet funded with satoshis for x402 payments
- BSV SDK: local path dependency at `../rust-sdk` with `features = ["transaction"]`

### Build and Test

```bash
cargo build                    # Build
cargo test                     # Run all tests
cargo clippy -- -D warnings    # Lint (must be warning-free)
cargo run -- status            # Check wallet connectivity
cargo run -- think "prompt"    # Single LLM call via x402
cargo run -- serve --port 8080 # Start HTTP daemon
```

## Architecture

See [CLAUDE.md](CLAUDE.md) for the full architecture overview, module descriptions, HTTP API table, tool categories, and design decisions.

## Questions?

Open an issue if you have questions about contributing or the codebase.
