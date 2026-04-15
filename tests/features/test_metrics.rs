//! Integration tests for the Prometheus metrics module.

use dolphin_milk::metrics::MetricsRegistry;

// ── MetricsRegistry creation ───────────────────────────────────────────

#[test]
fn test_registry_creation() {
    let registry = MetricsRegistry::new();
    // All metrics should be registered — gather returns non-empty output
    let output = registry.gather();
    // Even with zero values, Prometheus text format includes metadata
    assert!(!output.is_empty(), "gather should return non-empty text");
}

#[test]
fn test_default_trait() {
    let registry = MetricsRegistry::default();
    let output = registry.gather();
    assert!(!output.is_empty());
}

// ── Counter increment ──────────────────────────────────────────────────

#[test]
fn test_tokens_total_counter() {
    let registry = MetricsRegistry::new();
    registry
        .tokens_total
        .with_label_values(&["gpt-5-mini"])
        .inc_by(1234);
    registry
        .tokens_total
        .with_label_values(&["claude-haiku-4-5"])
        .inc_by(567);

    let output = registry.gather();
    assert!(
        output.contains("dm_tokens_total"),
        "should contain dm_tokens_total metric"
    );
    assert!(
        output.contains("model=\"gpt-5-mini\""),
        "should contain gpt-5-mini label"
    );
    assert!(
        output.contains("model=\"claude-haiku-4-5\""),
        "should contain claude label"
    );
    assert!(output.contains("1234"), "should contain the counter value");
}

#[test]
fn test_tasks_total_counter() {
    let registry = MetricsRegistry::new();
    registry.tasks_total.with_label_values(&["complete"]).inc();
    registry.tasks_total.with_label_values(&["complete"]).inc();
    registry.tasks_total.with_label_values(&["error"]).inc();

    let output = registry.gather();
    assert!(
        output.contains("dm_tasks_total"),
        "should contain dm_tasks_total"
    );
    assert!(
        output.contains("status=\"complete\""),
        "should contain complete label"
    );
    assert!(
        output.contains("status=\"error\""),
        "should contain error label"
    );
}

#[test]
fn test_tool_calls_total_counter() {
    let registry = MetricsRegistry::new();
    registry
        .tool_calls_total
        .with_label_values(&["memory_search"])
        .inc();
    registry
        .tool_calls_total
        .with_label_values(&["memory_search"])
        .inc();
    registry
        .tool_calls_total
        .with_label_values(&["file_read"])
        .inc();

    let output = registry.gather();
    assert!(output.contains("dm_tool_calls_total"));
    assert!(output.contains("tool_name=\"memory_search\""));
    assert!(output.contains("tool_name=\"file_read\""));
}

#[test]
fn test_errors_total_counter() {
    let registry = MetricsRegistry::new();
    registry
        .errors_total
        .with_label_values(&["tool_failure"])
        .inc();
    registry.errors_total.with_label_values(&["budget"]).inc();

    let output = registry.gather();
    assert!(output.contains("dm_errors_total"));
    assert!(output.contains("error_type=\"tool_failure\""));
    assert!(output.contains("error_type=\"budget\""));
}

#[test]
fn test_budget_spent_sats_counter() {
    let registry = MetricsRegistry::new();
    registry
        .budget_spent_sats
        .with_label_values(&["llm"])
        .inc_by(500.0);
    registry
        .budget_spent_sats
        .with_label_values(&["tool"])
        .inc_by(100.0);

    let output = registry.gather();
    assert!(output.contains("dm_budget_spent_sats"));
    assert!(output.contains("service=\"llm\""));
    assert!(output.contains("service=\"tool\""));
}

// ── Histogram ──────────────────────────────────────────────────────────

#[test]
fn test_request_latency_histogram() {
    let registry = MetricsRegistry::new();
    registry
        .request_latency_seconds
        .with_label_values(&["openai"])
        .observe(1.5);
    registry
        .request_latency_seconds
        .with_label_values(&["openai"])
        .observe(2.3);
    registry
        .request_latency_seconds
        .with_label_values(&["claude"])
        .observe(0.8);

    let output = registry.gather();
    assert!(
        output.contains("dm_request_latency_seconds"),
        "should contain histogram"
    );
    assert!(
        output.contains("provider=\"openai\""),
        "should contain openai label"
    );
    assert!(
        output.contains("provider=\"claude\""),
        "should contain claude label"
    );
    // Histogram produces _bucket, _sum, _count lines
    assert!(
        output.contains("dm_request_latency_seconds_count"),
        "should have count"
    );
    assert!(
        output.contains("dm_request_latency_seconds_sum"),
        "should have sum"
    );
    assert!(
        output.contains("dm_request_latency_seconds_bucket"),
        "should have buckets"
    );
}

// ── Gauge ──────────────────────────────────────────────────────────────

#[test]
fn test_active_tasks_gauge() {
    let registry = MetricsRegistry::new();
    registry.active_tasks.set(3);
    let output = registry.gather();
    assert!(output.contains("dm_active_tasks"));
    assert!(output.contains("3"), "should contain gauge value");

    registry.active_tasks.set(0);
    let output2 = registry.gather();
    assert!(
        output2.contains("dm_active_tasks 0"),
        "should show 0 when no active tasks"
    );
}

// ── Gather format ──────────────────────────────────────────────────────

#[test]
fn test_gather_prometheus_text_format() {
    let registry = MetricsRegistry::new();
    registry
        .tokens_total
        .with_label_values(&["test-model"])
        .inc_by(42);

    let output = registry.gather();
    // Prometheus text format has HELP and TYPE lines
    assert!(output.contains("# HELP dm_tokens_total"));
    assert!(output.contains("# TYPE dm_tokens_total counter"));
}

#[test]
fn test_gather_contains_all_metric_names() {
    let registry = MetricsRegistry::new();
    // Touch at least one label per metric so they appear in output
    registry.tokens_total.with_label_values(&["m"]).inc();
    registry
        .request_latency_seconds
        .with_label_values(&["p"])
        .observe(0.1);
    registry
        .budget_spent_sats
        .with_label_values(&["s"])
        .inc_by(1.0);
    registry.tasks_total.with_label_values(&["complete"]).inc();
    registry.tool_calls_total.with_label_values(&["t"]).inc();
    registry.errors_total.with_label_values(&["e"]).inc();
    registry.active_tasks.set(1);

    let output = registry.gather();
    let expected_names = [
        "dm_tokens_total",
        "dm_request_latency_seconds",
        "dm_budget_spent_sats",
        "dm_tasks_total",
        "dm_tool_calls_total",
        "dm_errors_total",
        "dm_active_tasks",
    ];
    for name in &expected_names {
        assert!(
            output.contains(name),
            "gather output should contain metric: {name}"
        );
    }
}

// ── Label correctness ──────────────────────────────────────────────────

#[test]
fn test_label_values_are_correct() {
    let registry = MetricsRegistry::new();

    // Tokens labeled by model
    registry
        .tokens_total
        .with_label_values(&["gpt-5.2"])
        .inc_by(100);
    let output = registry.gather();
    assert!(output.contains("model=\"gpt-5.2\""));

    // Budget labeled by service
    registry
        .budget_spent_sats
        .with_label_values(&["proofs"])
        .inc_by(200.0);
    let output = registry.gather();
    assert!(output.contains("service=\"proofs\""));

    // Tasks labeled by status
    registry.tasks_total.with_label_values(&["cancelled"]).inc();
    let output = registry.gather();
    assert!(output.contains("status=\"cancelled\""));
}

#[test]
fn test_multiple_label_values_same_metric() {
    let registry = MetricsRegistry::new();
    registry
        .tokens_total
        .with_label_values(&["gpt-5-mini"])
        .inc_by(100);
    registry
        .tokens_total
        .with_label_values(&["gpt-5-nano"])
        .inc_by(200);
    registry
        .tokens_total
        .with_label_values(&["claude-sonnet-4-6"])
        .inc_by(300);

    let output = registry.gather();
    assert!(output.contains("model=\"gpt-5-mini\""));
    assert!(output.contains("model=\"gpt-5-nano\""));
    assert!(output.contains("model=\"claude-sonnet-4-6\""));
}

// ── /metrics route (axum oneshot) ──────────────────────────────────────

#[tokio::test]
async fn test_metrics_endpoint_returns_prometheus_text() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dolphin_milk::config::DmConfig;
    use dolphin_milk::server::build_router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let app = build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        path,
    )
    .await;

    let req = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let content_type = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/plain"),
        "Content-Type should be text/plain, got: {content_type}"
    );
    assert!(
        content_type.contains("version=0.0.4"),
        "Content-Type should contain Prometheus version, got: {content_type}"
    );

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();

    // The IntGauge (no labels) always appears in output.
    // IntCounterVec and HistogramVec only appear after at least one label has been observed.
    assert!(
        text.contains("dm_active_tasks"),
        "should contain dm_active_tasks gauge"
    );

    // Verify it is valid Prometheus text format (contains TYPE declarations)
    assert!(
        text.contains("# TYPE dm_active_tasks gauge"),
        "should contain TYPE declaration for gauge"
    );
    assert!(
        text.contains("# HELP dm_active_tasks"),
        "should contain HELP declaration for gauge"
    );
}

#[tokio::test]
async fn test_metrics_endpoint_active_tasks_gauge() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use dolphin_milk::config::DmConfig;
    use dolphin_milk::server::build_router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let app = build_router(
        {
            let mut c = DmConfig::default();
            c.wallet.url = "http://127.0.0.1:19999".into();
            c
        },
        path,
    )
    .await;

    let req = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body.to_vec()).unwrap();

    // Active tasks should be 0 in a fresh server with no running tasks
    assert!(
        text.contains("dm_active_tasks 0"),
        "active tasks should be 0 in fresh server"
    );
}
