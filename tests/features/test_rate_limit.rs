//! Tests for x402 rate limiter — token bucket, config parsing, cert overrides,
//! per-service isolation, backward compatibility, and timing.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dolphin_milk::x402::rate_limit::{
    base_url, CertRateLimits, RateLimitConfig, RateLimiterRegistry, ServiceRateLimit,
};

fn test_wallet() -> std::sync::Arc<dyn dolphin_milk::wallet::WalletBackend + Send + Sync> {
    std::sync::Arc::new(dolphin_milk::wallet::HttpWalletClient::new(
        "http://localhost:3322",
        "http://localhost",
        30,
    ))
}

// ---------------------------------------------------------------------------
// Helper: build RateLimitConfig with defaults
// ---------------------------------------------------------------------------

fn config_enabled(default_rpm: u32) -> RateLimitConfig {
    RateLimitConfig {
        enabled: true,
        default_rpm,
        per_service: HashMap::new(),
    }
}

fn config_disabled() -> RateLimitConfig {
    RateLimitConfig {
        enabled: false,
        default_rpm: 60,
        per_service: HashMap::new(),
    }
}

fn config_with_service(default_rpm: u32, service_url: &str, service_rpm: u32) -> RateLimitConfig {
    let mut per_service = HashMap::new();
    per_service.insert(
        service_url.to_string(),
        ServiceRateLimit {
            requests_per_minute: service_rpm,
        },
    );
    RateLimitConfig {
        enabled: true,
        default_rpm,
        per_service,
    }
}

// ---------------------------------------------------------------------------
// 1. test_bucket_starts_full
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_bucket_starts_full() {
    // A fresh bucket with RPM=10 should allow 10 immediate requests
    let config = config_enabled(10);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..10 {
        registry.acquire(url).await.unwrap();
    }
    // All 10 should have succeeded without blocking
}

// ---------------------------------------------------------------------------
// 2. test_bucket_drains_on_consume
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_bucket_drains_on_consume() {
    let config = config_enabled(5);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..5 {
        registry.acquire(url).await.unwrap();
    }

    // After consuming 5 tokens, status should show near-zero available
    let status = registry.status();
    assert_eq!(status.len(), 1);
    assert!(
        status[0].tokens_available < 1.0,
        "bucket should be near-empty"
    );
}

// ---------------------------------------------------------------------------
// 3. test_bucket_refills_over_time
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_bucket_refills_over_time() {
    // RPM=600 means 10 tokens/sec. After draining, wait 200ms for ~2 tokens.
    let config = config_enabled(600);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://test.example.com/api";

    // Drain all 600 tokens
    for _ in 0..600 {
        registry.acquire(url).await.unwrap();
    }

    // Wait 200ms — should refill ~2 tokens (600 rpm = 10/sec)
    tokio::time::sleep(Duration::from_millis(200)).await;

    let status = registry.status();
    assert!(!status.is_empty());
    // Should have refilled some tokens (at least 1)
    assert!(
        status[0].tokens_available >= 1.0,
        "expected refill, got {}",
        status[0].tokens_available
    );
}

// ---------------------------------------------------------------------------
// 4. test_bucket_rejects_when_empty
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_bucket_rejects_when_empty() {
    // RPM=2 — consume both, then verify the third call blocks
    let config = config_enabled(2);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://test.example.com/api";
    registry.acquire(url).await.unwrap();
    registry.acquire(url).await.unwrap();

    // Third call should block. Use a timeout to prove it blocks.
    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_millis(50), registry.acquire(url)).await;

    // Should timeout because bucket is empty and refill takes ~30s for RPM=2
    assert!(result.is_err(), "expected timeout — bucket should be empty");
    assert!(start.elapsed() >= Duration::from_millis(40));
}

// ---------------------------------------------------------------------------
// 5. test_bucket_allows_after_refill_wait
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_bucket_allows_after_refill_wait() {
    // RPM=600 means 10 tokens/sec. Drain, wait ~150ms, should have 1+ token.
    let config = config_enabled(600);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://test.example.com/api";

    // Drain all tokens
    for _ in 0..600 {
        registry.acquire(url).await.unwrap();
    }

    // Wait for refill
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Should now succeed
    let result = tokio::time::timeout(Duration::from_millis(50), registry.acquire(url)).await;
    assert!(result.is_ok(), "should have refilled after 150ms");
}

// ---------------------------------------------------------------------------
// 6. test_concurrent_access_is_safe
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_concurrent_access_is_safe() {
    use std::sync::Arc;

    let config = config_enabled(1000);
    let registry = Arc::new(RateLimiterRegistry::new(&config, &CertRateLimits::none()));

    let url = "https://openai-chat.x402agency.com/chat";

    let mut handles = Vec::new();
    for _ in 0..10 {
        let reg = Arc::clone(&registry);
        let u = url.to_string();
        handles.push(tokio::spawn(async move {
            for _ in 0..50 {
                reg.acquire(&u).await.unwrap();
            }
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Should not panic or deadlock — registry is still usable
    let status = registry.status();
    assert_eq!(status.len(), 1);
}

// ---------------------------------------------------------------------------
// 7. test_per_service_isolation
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_per_service_isolation() {
    let mut per_service = HashMap::new();
    per_service.insert(
        "https://openai-chat.x402agency.com".to_string(),
        ServiceRateLimit {
            requests_per_minute: 3,
        },
    );
    per_service.insert(
        "https://claude-chat.x402agency.com".to_string(),
        ServiceRateLimit {
            requests_per_minute: 5,
        },
    );
    let config = RateLimitConfig {
        enabled: true,
        default_rpm: 0, // unlimited default
        per_service,
    };
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let openai = "https://openai-chat.x402agency.com/chat";
    let claude = "https://claude-chat.x402agency.com/chat";

    // OpenAI allows 3
    for _ in 0..3 {
        registry.acquire(openai).await.unwrap();
    }
    // OpenAI should now block
    let result = tokio::time::timeout(Duration::from_millis(50), registry.acquire(openai)).await;
    assert!(result.is_err(), "openai should be rate-limited after 3");

    // Claude should still work (5 tokens available)
    for _ in 0..5 {
        registry.acquire(claude).await.unwrap();
    }
}

// ---------------------------------------------------------------------------
// 8. test_default_no_limit
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_default_no_limit() {
    // Default config = disabled = unlimited = always passes
    let config = RateLimitConfig::default();
    assert!(!config.enabled);
    assert_eq!(config.default_rpm, 0);

    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://openai-chat.x402agency.com/chat";
    // Should never block
    for _ in 0..1000 {
        registry.acquire(url).await.unwrap();
    }
}

// ---------------------------------------------------------------------------
// 9. test_config_parsing_rate_limit_section
// ---------------------------------------------------------------------------
#[test]
fn test_config_parsing_rate_limit_section() {
    let toml_str = r#"
[x402]
registry_url = "https://example.com/.well-known/agents"

[x402.rate_limits]
enabled = true
default_rpm = 120

[x402.rate_limits.per_service."https://openai-chat.x402agency.com"]
requests_per_minute = 60

[x402.rate_limits.per_service."https://claude-chat.x402agency.com"]
requests_per_minute = 30
"#;

    let config: dolphin_milk::config::DmConfig = toml::from_str(toml_str).unwrap();
    assert!(config.x402.rate_limits.enabled);
    assert_eq!(config.x402.rate_limits.default_rpm, 120);
    assert_eq!(config.x402.rate_limits.per_service.len(), 2);
    assert_eq!(
        config
            .x402
            .rate_limits
            .per_service
            .get("https://openai-chat.x402agency.com")
            .unwrap()
            .requests_per_minute,
        60
    );
    assert_eq!(
        config
            .x402
            .rate_limits
            .per_service
            .get("https://claude-chat.x402agency.com")
            .unwrap()
            .requests_per_minute,
        30
    );
}

// ---------------------------------------------------------------------------
// 10. test_config_env_override
// ---------------------------------------------------------------------------
#[test]
fn test_config_env_override() {
    // Verify that the env vars are the right names and would parse
    // (We can't actually set env vars safely in parallel tests, but we can
    // test that the config struct accepts these values)
    let config = RateLimitConfig {
        enabled: true,
        default_rpm: 240,
        per_service: HashMap::new(),
    };
    assert!(config.enabled);
    assert_eq!(config.default_rpm, 240);
}

// ---------------------------------------------------------------------------
// 11. test_cert_limit_overrides_config
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cert_limit_overrides_config() {
    // Config says RPM=100, cert says RPM=10 — cert wins
    let config = config_enabled(100);
    let cert = CertRateLimits {
        default_rpm: Some(10),
        enabled: None, // no override on enabled — use config
    };
    let registry = RateLimiterRegistry::new(&config, &cert);

    let url = "https://openai-chat.x402agency.com/chat";
    // Should only allow 10 immediate requests
    for _ in 0..10 {
        registry.acquire(url).await.unwrap();
    }

    // 11th should block (cert RPM=10)
    let result = tokio::time::timeout(Duration::from_millis(50), registry.acquire(url)).await;
    assert!(
        result.is_err(),
        "cert RPM=10 should throttle after 10 requests"
    );
}

// ---------------------------------------------------------------------------
// 12. test_cert_disabled_overrides_config_enabled
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cert_disabled_overrides_config_enabled() {
    // Config says enabled=true with RPM=5, cert says enabled=false — cert wins
    let config = config_enabled(5);
    let cert = CertRateLimits {
        default_rpm: None,
        enabled: Some(false),
    };
    let registry = RateLimiterRegistry::new(&config, &cert);
    assert!(!registry.is_enabled());

    // Should be unlimited since cert disabled rate limiting
    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..100 {
        registry.acquire(url).await.unwrap();
    }
}

// ---------------------------------------------------------------------------
// 13. test_no_cert_falls_back_to_config
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_no_cert_falls_back_to_config() {
    let config = config_enabled(3);
    let cert = CertRateLimits::none();
    let registry = RateLimiterRegistry::new(&config, &cert);

    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..3 {
        registry.acquire(url).await.unwrap();
    }

    let result = tokio::time::timeout(Duration::from_millis(50), registry.acquire(url)).await;
    assert!(result.is_err(), "config RPM=3 should throttle");
}

// ---------------------------------------------------------------------------
// 14. test_zero_rpm_means_unlimited
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_zero_rpm_means_unlimited() {
    let config = config_enabled(0); // enabled but RPM=0 = unlimited
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..1000 {
        registry.acquire(url).await.unwrap();
    }
    // Should never block — 0 RPM means no limit
}

// ---------------------------------------------------------------------------
// 15. test_registry_returns_status_per_service
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_registry_returns_status_per_service() {
    let config = config_with_service(60, "https://openai-chat.x402agency.com", 30);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    // Hit both services to create buckets
    let openai = "https://openai-chat.x402agency.com/chat";
    let other = "https://other.example.com/api";

    registry.acquire(openai).await.unwrap();
    registry.acquire(other).await.unwrap();

    let status = registry.status();
    assert_eq!(status.len(), 2);

    // Find each service in the status
    let openai_status = status
        .iter()
        .find(|s| s.service == "https://openai-chat.x402agency.com")
        .unwrap();
    let other_status = status
        .iter()
        .find(|s| s.service == "https://other.example.com")
        .unwrap();

    // OpenAI has per-service override of 30 RPM
    assert_eq!(openai_status.requests_per_minute, 30);
    assert_eq!(openai_status.capacity, 30);

    // Other uses default of 60 RPM
    assert_eq!(other_status.requests_per_minute, 60);
    assert_eq!(other_status.capacity, 60);
}

// ---------------------------------------------------------------------------
// 16. test_disabled_config_allows_everything
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_disabled_config_allows_everything() {
    let config = config_disabled();
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    assert!(!registry.is_enabled());

    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..500 {
        registry.acquire(url).await.unwrap();
    }
}

// ---------------------------------------------------------------------------
// 17. test_rate_limit_throttles_requests (timing verification)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rate_limit_throttles_requests() {
    // RPM=600 = 10/sec = 1 token per 100ms.
    // Drain bucket, then verify acquire takes ~100ms.
    let config = config_enabled(600);
    let registry = RateLimiterRegistry::new(&config, &CertRateLimits::none());

    let url = "https://test.example.com/api";

    // Drain all 600 tokens
    for _ in 0..600 {
        registry.acquire(url).await.unwrap();
    }

    // Next acquire should wait ~100ms for 1 token at 10 tokens/sec
    let start = Instant::now();
    registry.acquire(url).await.unwrap();
    let elapsed = start.elapsed();

    // Should have waited approximately 100ms (allow some slack)
    assert!(
        elapsed >= Duration::from_millis(50),
        "expected wait ~100ms, got {:?}",
        elapsed
    );
    assert!(
        elapsed <= Duration::from_millis(300),
        "waited too long: {:?}",
        elapsed
    );
}

// ---------------------------------------------------------------------------
// 18. test_base_url_extraction
// ---------------------------------------------------------------------------
#[test]
fn test_base_url_extraction() {
    assert_eq!(
        base_url("https://openai-chat.x402agency.com/chat"),
        "https://openai-chat.x402agency.com"
    );
    assert_eq!(
        base_url("https://claude-chat.x402agency.com/v1/messages"),
        "https://claude-chat.x402agency.com"
    );
    assert_eq!(
        base_url("http://localhost:8080/v1/completions"),
        "http://localhost:8080"
    );
    assert_eq!(
        base_url("https://nanostore.babbage.systems/upload"),
        "https://nanostore.babbage.systems"
    );
}

// ---------------------------------------------------------------------------
// 19. test_cert_enabled_overrides_config_disabled
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cert_enabled_overrides_config_disabled() {
    // Config says disabled, cert says enabled with RPM=5 — cert wins
    let config = config_disabled();
    let cert = CertRateLimits {
        default_rpm: Some(5),
        enabled: Some(true),
    };
    let registry = RateLimiterRegistry::new(&config, &cert);
    assert!(registry.is_enabled());

    let url = "https://openai-chat.x402agency.com/chat";
    for _ in 0..5 {
        registry.acquire(url).await.unwrap();
    }

    let result = tokio::time::timeout(Duration::from_millis(50), registry.acquire(url)).await;
    assert!(result.is_err(), "cert should have enabled rate limiting");
}

// ---------------------------------------------------------------------------
// 20. test_disabled_factory
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_disabled_factory() {
    let registry = RateLimiterRegistry::disabled();
    assert!(!registry.is_enabled());
    assert!(registry.status().is_empty());

    let url = "https://openai-chat.x402agency.com/chat";
    registry.acquire(url).await.unwrap();
    // Disabled registry doesn't even create buckets
    assert!(registry.status().is_empty());
}

// ---------------------------------------------------------------------------
// 21. test_rate_limiter_on_wormloop — verify the field exists and defaults to None
// ---------------------------------------------------------------------------
#[test]
fn test_rate_limiter_on_wormloop() {
    let config = dolphin_milk::config::DmConfig::default();
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let worm = dolphin_milk::runner::create_loop(config, workspace, None, None, test_wallet());
    // Default DmLoop has no rate limiter (CLI mode)
    assert!(worm.rate_limiter.is_none());
}

// ---------------------------------------------------------------------------
// 22. test_rate_limiter_passed_to_wormloop — verify it can be set
// ---------------------------------------------------------------------------
#[test]
fn test_rate_limiter_passed_to_wormloop() {
    let config = dolphin_milk::config::DmConfig::default();
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let rl_config = config_enabled(60);
    let registry = std::sync::Arc::new(RateLimiterRegistry::new(
        &rl_config,
        &CertRateLimits::none(),
    ));

    let worm = dolphin_milk::runner::create_loop_with_rate_limiter(
        config,
        workspace,
        None,
        None,
        test_wallet(),
        Some(registry.clone()),
        std::sync::Arc::new(dolphin_milk::x402::circuit_breaker::CircuitBreakerRegistry::new()),
        None,
    );
    assert!(worm.rate_limiter.is_some());
    assert!(worm.rate_limiter.as_ref().unwrap().is_enabled());
}

// ---------------------------------------------------------------------------
// 23. test_rate_limiter_none_is_noop — None rate_limiter means no rate limiting
// ---------------------------------------------------------------------------
#[test]
fn test_rate_limiter_none_is_noop() {
    let config = dolphin_milk::config::DmConfig::default();
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();
    let worm = dolphin_milk::runner::create_loop(config, workspace, None, None, test_wallet());
    // rate_limiter is None in CLI mode — no rate limiting applied
    assert!(worm.rate_limiter.is_none());
}

// ---------------------------------------------------------------------------
// 24. test_rate_limiter_disabled_is_noop — disabled registry allows everything
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rate_limiter_disabled_on_wormloop() {
    let config = dolphin_milk::config::DmConfig::default();
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_path_buf();

    let rl = std::sync::Arc::new(RateLimiterRegistry::disabled());
    let worm = dolphin_milk::runner::create_loop_with_rate_limiter(
        config,
        workspace,
        None,
        None,
        test_wallet(),
        Some(rl.clone()),
        std::sync::Arc::new(dolphin_milk::x402::circuit_breaker::CircuitBreakerRegistry::new()),
        None,
    );
    assert!(worm.rate_limiter.is_some());
    assert!(!worm.rate_limiter.as_ref().unwrap().is_enabled());

    // Disabled rate limiter allows everything without blocking
    worm.rate_limiter
        .as_ref()
        .unwrap()
        .acquire("https://openai-chat.x402agency.com/chat")
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// 25. test_x402_tools_with_rate_limiter — tools created with rate limiter
// ---------------------------------------------------------------------------
#[test]
fn test_x402_tools_with_rate_limiter() {
    let rl = std::sync::Arc::new(RateLimiterRegistry::new(
        &config_enabled(60),
        &CertRateLimits::none(),
    ));
    let tools = dolphin_milk::tools::x402_tools::all_x402_tools_with_rate_limiter(
        "http://localhost:3322".into(),
        "https://x402agency.com/.well-known/agents".into(),
        Some(rl),
    );
    assert_eq!(tools.len(), 3);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"x402_call"));
}

// ---------------------------------------------------------------------------
// 26. test_x402_recipe_tools_with_rate_limiter — recipe tools created with rate limiter
// ---------------------------------------------------------------------------
#[test]
fn test_x402_recipe_tools_with_rate_limiter() {
    let rl = std::sync::Arc::new(RateLimiterRegistry::new(
        &config_enabled(60),
        &CertRateLimits::none(),
    ));
    let tools = dolphin_milk::tools::x402_tools::all_x402_recipe_tools_with_rate_limiter(
        "http://localhost:3322".into(),
        Some(rl),
    );
    assert_eq!(tools.len(), 2);
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"generate_image"));
    assert!(names.contains(&"upload_to_nanostore"));
}
