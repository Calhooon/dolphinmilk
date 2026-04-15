//! Tests for x402 response cache -- LRU+TTL, tool-call exclusion,
//! deterministic keys, stats tracking.

use serde_json::json;
use std::time::Duration;

use dolphin_milk::think::ThinkResult;
use dolphin_milk::x402::cache::ResponseCache;

// ---------------------------------------------------------------------------
// Helper: create a minimal ThinkResult
// ---------------------------------------------------------------------------

fn make_result(text: &str, sats: u64) -> ThinkResult {
    ThinkResult {
        text: text.to_string(),
        model: "gpt-5-mini".to_string(),
        sats_paid: sats,
        sats_effective: sats,
        sats_refunded: 0,
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        finish_reason: "stop".to_string(),
        duration_ms: 500,
        tool_calls: vec![],
        payment_txid: Some("abc123".to_string()),
        refund_internalized: None,
        was_text_extracted: false,
    }
}

fn make_tool_call_result(text: &str, sats: u64) -> ThinkResult {
    ThinkResult {
        text: text.to_string(),
        model: "gpt-5-mini".to_string(),
        sats_paid: sats,
        sats_effective: sats,
        sats_refunded: 0,
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        finish_reason: "tool_calls".to_string(),
        duration_ms: 500,
        tool_calls: vec![json!({
            "id": "call_123",
            "type": "function",
            "function": {
                "name": "execute_bash",
                "arguments": "{\"command\": \"ls\"}"
            }
        })],
        payment_txid: Some("abc123".to_string()),
        refund_internalized: None,
        was_text_extracted: false,
    }
}

/// Predicate: only cache responses without tool_calls.
fn should_cache(result: &ThinkResult) -> bool {
    result.tool_calls.is_empty()
}

// ---------------------------------------------------------------------------
// 16. test_cache_miss_returns_none
// ---------------------------------------------------------------------------
#[test]
fn test_cache_miss_returns_none() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    assert!(cache.get("nonexistent_key").is_none());
}

// ---------------------------------------------------------------------------
// 17. test_cache_hit_returns_stored
// ---------------------------------------------------------------------------
#[test]
fn test_cache_hit_returns_stored() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    let result = make_result("hello world", 1000);
    let key = "test_key".to_string();

    cache.put(key.clone(), result.clone(), 1000);
    let cached = cache.get(&key);

    assert!(cached.is_some());
    let cached = cached.unwrap();
    assert_eq!(cached.text, "hello world");
    assert_eq!(cached.sats_paid, 1000);
}

// ---------------------------------------------------------------------------
// 18. test_cache_key_deterministic
// ---------------------------------------------------------------------------
#[test]
fn test_cache_key_deterministic() {
    let msgs = vec![json!({"role": "user", "content": "hello"})];
    let k1 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.7), 4096);
    let k2 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.7), 4096);
    assert_eq!(k1, k2);
    assert_eq!(k1.len(), 64); // SHA-256 hex
}

// ---------------------------------------------------------------------------
// 19. test_cache_key_differs_by_model
// ---------------------------------------------------------------------------
#[test]
fn test_cache_key_differs_by_model() {
    let msgs = vec![json!({"role": "user", "content": "hello"})];
    let k1 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.7), 4096);
    let k2 = ResponseCache::<ThinkResult>::cache_key("claude-sonnet-4-6", &msgs, Some(0.7), 4096);
    assert_ne!(k1, k2);
}

// ---------------------------------------------------------------------------
// 20. test_cache_key_differs_by_messages
// ---------------------------------------------------------------------------
#[test]
fn test_cache_key_differs_by_messages() {
    let msgs1 = vec![json!({"role": "user", "content": "hello"})];
    let msgs2 = vec![json!({"role": "user", "content": "goodbye"})];
    let k1 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs1, Some(0.7), 4096);
    let k2 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs2, Some(0.7), 4096);
    assert_ne!(k1, k2);
}

// ---------------------------------------------------------------------------
// 21. test_cache_key_differs_by_temperature
// ---------------------------------------------------------------------------
#[test]
fn test_cache_key_differs_by_temperature() {
    let msgs = vec![json!({"role": "user", "content": "hello"})];
    let k1 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.7), 4096);
    let k2 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.9), 4096);
    let k3 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, None, 4096);
    assert_ne!(k1, k2);
    assert_ne!(k1, k3);
    assert_ne!(k2, k3);
}

// ---------------------------------------------------------------------------
// 22. test_ttl_expiry_evicts
// ---------------------------------------------------------------------------
#[test]
fn test_ttl_expiry_evicts() {
    // 0 = disabled
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(0, 100);
    let result = make_result("ephemeral", 500);
    let stored = cache.put("key".to_string(), result, 500);
    assert!(!stored); // zero TTL means put returns false

    // Now with very short TTL
    let cache2: ResponseCache<ThinkResult> = ResponseCache::new(1, 100); // 1 second
    let result2 = make_result("short-lived", 500);
    cache2.put("key2".to_string(), result2, 500);

    // Should be available immediately
    assert!(cache2.get("key2").is_some());

    // Wait for expiry
    std::thread::sleep(Duration::from_millis(1100));
    assert!(cache2.get("key2").is_none());
}

// ---------------------------------------------------------------------------
// 23. test_lru_eviction
// ---------------------------------------------------------------------------
#[test]
fn test_lru_eviction() {
    // Cache with max 2 entries
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 2);

    cache.put("a".to_string(), make_result("first", 100), 100);
    cache.put("b".to_string(), make_result("second", 200), 200);
    assert_eq!(cache.len(), 2);

    // Adding a third should evict the least recently used ("a")
    cache.put("c".to_string(), make_result("third", 300), 300);
    assert_eq!(cache.len(), 2);
    assert!(cache.get("a").is_none()); // evicted
    assert!(cache.get("b").is_some()); // still there
    assert!(cache.get("c").is_some()); // just added
}

// ---------------------------------------------------------------------------
// 24. test_tool_call_responses_never_cached
// ---------------------------------------------------------------------------
#[test]
fn test_tool_call_responses_never_cached() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    let result = make_tool_call_result("I'll run that command", 1500);

    let stored = cache.put_if("tool_key".to_string(), result, 1500, should_cache);
    assert!(!stored); // should be rejected
    assert!(cache.get("tool_key").is_none());
    assert!(cache.is_empty());
}

// ---------------------------------------------------------------------------
// 25. test_hit_counter_tracks_savings
// ---------------------------------------------------------------------------
#[test]
fn test_hit_counter_tracks_savings() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    let result = make_result("cached response", 1000);

    cache.put("key".to_string(), result, 1000);

    // First hit -- saves 1000 sats
    cache.get("key");
    let stats = cache.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.total_sats_saved, 1000);

    // Second hit -- saves another 1000
    cache.get("key");
    let stats = cache.stats();
    assert_eq!(stats.hits, 2);
    assert_eq!(stats.total_sats_saved, 2000);
}

// ---------------------------------------------------------------------------
// 26. test_cache_report
// ---------------------------------------------------------------------------
#[test]
fn test_cache_report() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);

    // Miss
    cache.get("missing");
    let stats = cache.stats();
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.hit_rate_pct(), 0.0);

    // Put and hit
    cache.put("key".to_string(), make_result("text", 500), 500);
    cache.get("key");
    let stats = cache.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert!((stats.hit_rate_pct() - 50.0).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// 27. test_config_ttl_and_max_entries
// ---------------------------------------------------------------------------
#[test]
fn test_config_ttl_and_max_entries() {
    use dolphin_milk::config::DmConfig;

    let config = DmConfig::default();
    assert_eq!(config.llm.cache_ttl_secs, 3600);
    assert_eq!(config.llm.cache_max_entries, 1000);

    // Create cache from config values
    let cache: ResponseCache<ThinkResult> =
        ResponseCache::new(config.llm.cache_ttl_secs, config.llm.cache_max_entries);
    assert_eq!(cache.ttl(), Duration::from_secs(3600));
}

// ---------------------------------------------------------------------------
// 28. test_zero_ttl_disables_cache
// ---------------------------------------------------------------------------
#[test]
fn test_zero_ttl_disables_cache() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(0, 100);
    let result = make_result("should not be cached", 1000);

    // put returns false when TTL is 0
    assert!(!cache.put("key".to_string(), result, 1000));

    // get always returns None when TTL is 0
    assert!(cache.get("key").is_none());
    assert!(cache.is_empty());
}

// ---------------------------------------------------------------------------
// 29. test_cache_key_differs_by_max_tokens
// ---------------------------------------------------------------------------
#[test]
fn test_cache_key_differs_by_max_tokens() {
    let msgs = vec![json!({"role": "user", "content": "hello"})];
    let k1 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.7), 4096);
    let k2 = ResponseCache::<ThinkResult>::cache_key("gpt-5-mini", &msgs, Some(0.7), 8192);
    assert_ne!(k1, k2);
}

// ---------------------------------------------------------------------------
// 30. test_empty_cache_stats
// ---------------------------------------------------------------------------
#[test]
fn test_empty_cache_stats() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    let stats = cache.stats();
    assert_eq!(stats.hits, 0);
    assert_eq!(stats.misses, 0);
    assert_eq!(stats.total_sats_saved, 0);
    assert_eq!(stats.evictions, 0);
    assert_eq!(stats.hit_rate_pct(), 0.0);
}

// ---------------------------------------------------------------------------
// 31. test_lru_eviction_tracks_stats
// ---------------------------------------------------------------------------
#[test]
fn test_lru_eviction_tracks_stats() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 2);
    cache.put("a".to_string(), make_result("a", 100), 100);
    cache.put("b".to_string(), make_result("b", 200), 200);
    cache.put("c".to_string(), make_result("c", 300), 300); // evicts "a"

    let stats = cache.stats();
    assert_eq!(stats.evictions, 1);
}

// ---------------------------------------------------------------------------
// 32. test_overwrite_same_key
// ---------------------------------------------------------------------------
#[test]
fn test_overwrite_same_key() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    cache.put("key".to_string(), make_result("first", 100), 100);
    cache.put("key".to_string(), make_result("second", 200), 200);

    let cached = cache.get("key").unwrap();
    assert_eq!(cached.text, "second");
    assert_eq!(cached.sats_paid, 200);
}

// ---------------------------------------------------------------------------
// 33. test_concurrent_cache_access
// ---------------------------------------------------------------------------
#[test]
fn test_concurrent_cache_access() {
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(ResponseCache::<ThinkResult>::new(3600, 1000));

    let mut handles = Vec::new();
    for i in 0..10 {
        let c = Arc::clone(&cache);
        handles.push(thread::spawn(move || {
            for j in 0..50 {
                let key = format!("key_{i}_{j}");
                c.put(key.clone(), make_result(&format!("val_{i}_{j}"), 100), 100);
                c.get(&key);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Should not panic or deadlock
    let stats = cache.stats();
    assert!(stats.hits > 0);
}

// ---------------------------------------------------------------------------
// 34. test_cache_preserves_all_fields
// ---------------------------------------------------------------------------
#[test]
fn test_cache_preserves_all_fields() {
    let cache: ResponseCache<ThinkResult> = ResponseCache::new(3600, 100);
    let result = ThinkResult {
        text: "detailed response".to_string(),
        model: "gpt-5.2".to_string(),
        sats_paid: 5000,
        sats_effective: 4800,
        sats_refunded: 200,
        prompt_tokens: 500,
        completion_tokens: 250,
        total_tokens: 750,
        finish_reason: "stop".to_string(),
        duration_ms: 2345,
        tool_calls: vec![], // no tool calls
        payment_txid: Some("tx123".to_string()),
        refund_internalized: Some(true),
        was_text_extracted: false,
    };

    cache.put("full".to_string(), result, 5000);
    let cached = cache.get("full").unwrap();

    assert_eq!(cached.text, "detailed response");
    assert_eq!(cached.model, "gpt-5.2");
    assert_eq!(cached.sats_paid, 5000);
    assert_eq!(cached.sats_effective, 4800);
    assert_eq!(cached.sats_refunded, 200);
    assert_eq!(cached.prompt_tokens, 500);
    assert_eq!(cached.completion_tokens, 250);
    assert_eq!(cached.total_tokens, 750);
    assert_eq!(cached.finish_reason, "stop");
    assert_eq!(cached.duration_ms, 2345);
    assert!(cached.tool_calls.is_empty());
    assert_eq!(cached.payment_txid, Some("tx123".to_string()));
    assert_eq!(cached.refund_internalized, Some(true));
}
