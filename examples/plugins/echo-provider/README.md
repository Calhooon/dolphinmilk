# Echo Provider

Example bsv-worm provider plugin demonstrating the `Provider` trait.

## What it does

A mock LLM provider that echoes the last user message back as the response. Useful for testing the agent loop, pipelines, and integrations without incurring real LLM costs.

## Usage

```rust
use echo_provider::EchoProvider;
use bsv_worm_sdk::provider::{Provider, ThinkRequest};
use bsv_worm_sdk::types::Message;

let provider = EchoProvider::new();
let request = ThinkRequest::new(
    vec![Message::user("Hello!")],
    "echo-v1",
);
let result = provider.think(request).await.unwrap();
assert_eq!(result.text, "Echo: Hello!");
```

Custom prefix:
```rust
let provider = EchoProvider::with_prefix("Bot says: ");
```

## Testing

```bash
cargo test -p echo-provider
```
