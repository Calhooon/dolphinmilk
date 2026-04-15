//! Prometheus metrics registry for the worm agent.
//!
//! Defines standard metrics (counters, histograms, gauges) that track
//! LLM token usage, request latency, budget spending, task lifecycle,
//! tool invocations, and errors. The `MetricsRegistry` is held on
//! `AppState` and queried by `GET /metrics` to produce Prometheus
//! text-format exposition output.

use prometheus::{
    CounterVec, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};

/// Holds all Prometheus metrics and the underlying registry.
pub struct MetricsRegistry {
    pub(crate) registry: Registry,

    /// Total tokens consumed, labeled by `model`.
    pub tokens_total: IntCounterVec,

    /// Request latency in seconds, labeled by `provider`.
    pub request_latency_seconds: HistogramVec,

    /// Total satoshis spent, labeled by `service`.
    pub budget_spent_sats: CounterVec,

    /// Total tasks created, labeled by `status` (running, complete, error, cancelled).
    pub tasks_total: IntCounterVec,

    /// Total tool calls executed, labeled by `tool_name`.
    pub tool_calls_total: IntCounterVec,

    /// Total errors encountered, labeled by `error_type`.
    pub errors_total: IntCounterVec,

    /// Number of currently active (running) tasks.
    pub active_tasks: IntGauge,
}

impl MetricsRegistry {
    /// Create a new MetricsRegistry with all metrics registered.
    pub fn new() -> Self {
        let registry = Registry::new();

        let tokens_total = IntCounterVec::new(
            Opts::new("dm_tokens_total", "Total tokens consumed by LLM inference"),
            &["model"],
        )
        .expect("metric can be created");
        registry
            .register(Box::new(tokens_total.clone()))
            .expect("metric can be registered");

        let request_latency_seconds = HistogramVec::new(
            HistogramOpts::new(
                "dm_request_latency_seconds",
                "LLM request latency in seconds",
            )
            .buckets(vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]),
            &["provider"],
        )
        .expect("metric can be created");
        registry
            .register(Box::new(request_latency_seconds.clone()))
            .expect("metric can be registered");

        let budget_spent_sats = CounterVec::new(
            Opts::new("dm_budget_spent_sats", "Total satoshis spent"),
            &["service"],
        )
        .expect("metric can be created");
        registry
            .register(Box::new(budget_spent_sats.clone()))
            .expect("metric can be registered");

        let tasks_total = IntCounterVec::new(
            Opts::new("dm_tasks_total", "Total tasks created"),
            &["status"],
        )
        .expect("metric can be created");
        registry
            .register(Box::new(tasks_total.clone()))
            .expect("metric can be registered");

        let tool_calls_total = IntCounterVec::new(
            Opts::new("dm_tool_calls_total", "Total tool calls executed"),
            &["tool_name"],
        )
        .expect("metric can be created");
        registry
            .register(Box::new(tool_calls_total.clone()))
            .expect("metric can be registered");

        let errors_total = IntCounterVec::new(
            Opts::new("dm_errors_total", "Total errors encountered"),
            &["error_type"],
        )
        .expect("metric can be created");
        registry
            .register(Box::new(errors_total.clone()))
            .expect("metric can be registered");

        let active_tasks = IntGauge::new("dm_active_tasks", "Number of currently active tasks")
            .expect("metric can be created");
        registry
            .register(Box::new(active_tasks.clone()))
            .expect("metric can be registered");

        Self {
            registry,
            tokens_total,
            request_latency_seconds,
            budget_spent_sats,
            tasks_total,
            tool_calls_total,
            errors_total,
            active_tasks,
        }
    }

    /// Gather all metrics and encode them as Prometheus text format.
    pub fn gather(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = String::new();
        encoder
            .encode_utf8(&metric_families, &mut buffer)
            .expect("encoding should succeed");
        buffer
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}
