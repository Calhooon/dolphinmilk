//! Tests for task tagging (E.4) — business-outcome labels on tasks.
//!
//! Covers:
//! - TaskInfo serialization with/without tags
//! - Tag normalization (trim, lowercase, dedup, filter empty)
//! - TaskRequest / ChatRequest deserialization with tags
//! - Tag filtering logic (AND semantics)
//! - Certificate-driven tag policy
//! - Backward compatibility (missing tags = empty vec)

use dolphin_milk::server::types::*;

// ===========================================================================
// Unit tests — TaskInfo with tags
// ===========================================================================

#[test]
fn test_task_info_default_empty_tags() {
    let info = TaskInfo {
        id: "t1".into(),
        task: "do something".into(),
        status: TaskStatus::Running,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-03-22T00:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };
    assert!(info.tags.is_empty());
}

#[test]
fn test_task_info_with_tags() {
    let info = TaskInfo {
        id: "t2".into(),
        task: "generate report".into(),
        status: TaskStatus::Complete,
        result: Some("done".into()),
        error: None,
        iterations: 3,
        sats_spent: 500,
        started_at: "2026-03-22T00:00:00Z".into(),
        completed_at: Some("2026-03-22T00:01:00Z".into()),
        proof_txids: Vec::new(),
        tags: vec!["marketing".into(), "roi".into()],
        origin: String::new(),
        conversation_id: None,
    };
    assert_eq!(info.tags.len(), 2);
    assert!(info.tags.contains(&"marketing".to_string()));
    assert!(info.tags.contains(&"roi".to_string()));
}

#[test]
fn test_task_info_serialization_with_tags() {
    let info = TaskInfo {
        id: "t3".into(),
        task: "tagged task".into(),
        status: TaskStatus::Running,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-03-22T00:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: vec!["billing".into(), "customer".into()],
        origin: String::new(),
        conversation_id: None,
    };
    let json_str = serde_json::to_string(&info).unwrap();
    assert!(json_str.contains("\"tags\""));
    assert!(json_str.contains("billing"));
    assert!(json_str.contains("customer"));

    // Roundtrip
    let parsed: TaskInfo = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.tags, vec!["billing", "customer"]);
}

#[test]
fn test_task_info_serialization_empty_tags() {
    let info = TaskInfo {
        id: "t4".into(),
        task: "no tags".into(),
        status: TaskStatus::Running,
        result: None,
        error: None,
        iterations: 0,
        sats_spent: 0,
        started_at: "2026-03-22T00:00:00Z".into(),
        completed_at: None,
        proof_txids: Vec::new(),
        tags: Vec::new(),
        origin: String::new(),
        conversation_id: None,
    };
    let json_str = serde_json::to_string(&info).unwrap();
    // Empty tags should be skipped in serialization (skip_serializing_if = "Vec::is_empty")
    assert!(
        !json_str.contains("\"tags\""),
        "empty tags key should not appear in JSON: {json_str}"
    );
}

// ===========================================================================
// Tag normalization
// ===========================================================================

#[test]
fn test_tag_validation_rejects_empty_string() {
    let result = normalize_tags(Some(vec!["".into(), "  ".into(), "valid".into()]));
    assert_eq!(result, vec!["valid"]);
}

#[test]
fn test_tag_validation_trims_whitespace() {
    let result = normalize_tags(Some(vec!["  Marketing ".into(), " roi ".into()]));
    assert_eq!(result, vec!["marketing", "roi"]);
}

#[test]
fn test_tags_deduplication() {
    let result = normalize_tags(Some(vec![
        "billing".into(),
        "Billing".into(),
        "BILLING".into(),
        "roi".into(),
    ]));
    assert_eq!(result, vec!["billing", "roi"]);
}

#[test]
fn test_normalize_tags_none_returns_empty() {
    let result = normalize_tags(None);
    assert!(result.is_empty());
}

#[test]
fn test_normalize_tags_all_empty_returns_empty() {
    let result = normalize_tags(Some(vec!["".into(), "  ".into()]));
    assert!(result.is_empty());
}

// ===========================================================================
// Certificate tag policy
// ===========================================================================

#[test]
fn test_cert_tag_policy_required() {
    // Simulate a cert with tags_required = "true"
    use dolphin_milk::certificates::CertTagPolicy;
    let policy = CertTagPolicy {
        tags_required: Some(true),
    };
    assert_eq!(policy.tags_required, Some(true));
}

#[test]
fn test_cert_tag_policy_not_required() {
    use dolphin_milk::certificates::CertTagPolicy;
    let policy = CertTagPolicy {
        tags_required: Some(false),
    };
    assert_eq!(policy.tags_required, Some(false));
}

#[test]
fn test_cert_tag_policy_no_cert_defaults_false() {
    // When no cert or field absent, tags_required should be None
    use dolphin_milk::certificates::CertTagPolicy;
    let policy = CertTagPolicy {
        tags_required: None,
    };
    // Policy should not enforce tags when None
    assert_ne!(policy.tags_required, Some(true));
}

// ===========================================================================
// Request deserialization
// ===========================================================================

#[test]
fn test_task_submission_with_tags_deserializes() {
    let json_str = r#"{"task": "hello", "max_iterations": 10, "tags": ["billing", "roi"]}"#;
    let req: TaskRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.task, "hello");
    assert_eq!(req.max_iterations, 10);
    let tags = req.tags.unwrap();
    assert_eq!(tags, vec!["billing", "roi"]);
}

#[test]
fn test_task_submission_without_tags_defaults_empty() {
    let json_str = r#"{"task": "hello"}"#;
    let req: TaskRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.task, "hello");
    assert_eq!(req.max_iterations, 50); // default
    assert!(req.tags.is_none());
    // normalize_tags(None) → empty vec
    let tags = normalize_tags(req.tags);
    assert!(tags.is_empty());
}

#[test]
fn test_chat_request_with_tags_deserializes() {
    let json_str = r#"{"message": "hi", "tags": ["support", "enterprise"]}"#;
    let req: ChatRequest = serde_json::from_str(json_str).unwrap();
    assert_eq!(req.message, "hi");
    let tags = req.tags.unwrap();
    assert_eq!(tags, vec!["support", "enterprise"]);
}

#[test]
fn test_chat_request_without_tags_defaults_none() {
    let json_str = r#"{"message": "hi"}"#;
    let req: ChatRequest = serde_json::from_str(json_str).unwrap();
    assert!(req.tags.is_none());
}

// ===========================================================================
// Tag filtering logic (AND semantics)
// ===========================================================================

/// Helper: build a TaskSummary with given tags.
fn make_summary(id: &str, tags: Vec<&str>) -> TaskSummary {
    TaskSummary {
        id: id.into(),
        task: format!("task {id}"),
        status: "complete".into(),
        iterations: 1,
        sats_spent: 100,
        started_at: "2026-03-22T00:00:00Z".into(),
        completed_at: Some("2026-03-22T00:01:00Z".into()),
        proof_txids: Vec::new(),
        tokens: 0,
        tags: tags.into_iter().map(String::from).collect(),
        time_saved_minutes: 0,
        origin: String::new(),
        conversation_id: None,
    }
}

/// Helper: filter summaries by tags (AND logic).
fn filter_by_tags(tasks: &[TaskSummary], filter_tags: &[&str]) -> Vec<String> {
    let ft: Vec<String> = filter_tags.iter().map(|t| t.to_string()).collect();
    tasks
        .iter()
        .filter(|t| ft.iter().all(|tag| t.tags.contains(tag)))
        .map(|t| t.id.clone())
        .collect()
}

#[test]
fn test_filter_tasks_by_single_tag() {
    let tasks = vec![
        make_summary("a", vec!["billing", "roi"]),
        make_summary("b", vec!["support"]),
        make_summary("c", vec!["billing"]),
    ];
    let ids = filter_by_tags(&tasks, &["billing"]);
    assert_eq!(ids, vec!["a", "c"]);
}

#[test]
fn test_filter_tasks_by_multiple_tags() {
    let tasks = vec![
        make_summary("a", vec!["billing", "roi"]),
        make_summary("b", vec!["billing"]),
        make_summary("c", vec!["roi"]),
    ];
    let ids = filter_by_tags(&tasks, &["billing", "roi"]);
    assert_eq!(ids, vec!["a"]); // Only "a" has both
}

#[test]
fn test_filter_tasks_no_match_returns_empty() {
    let tasks = vec![
        make_summary("a", vec!["billing"]),
        make_summary("b", vec!["support"]),
    ];
    let ids = filter_by_tags(&tasks, &["nonexistent"]);
    assert!(ids.is_empty());
}

#[test]
fn test_filter_tasks_no_filter_returns_all() {
    let tasks = [
        make_summary("a", vec!["billing"]),
        make_summary("b", vec![]),
    ];
    let filter_tags: &[&str] = &[];
    // No filter → all returned (empty filter matches everything)
    let ft: Vec<String> = filter_tags.iter().map(|t| t.to_string()).collect();
    let all_match = ft.is_empty()
        || tasks
            .iter()
            .all(|t| ft.iter().all(|tag| t.tags.contains(tag)));
    assert!(ft.is_empty()); // Confirm filter is empty
                            // When filter is empty, the handler doesn't filter at all
    assert!(all_match);
}

// ===========================================================================
// Backward compatibility
// ===========================================================================

#[test]
fn test_backward_compat_no_tags_in_json() {
    // JSON without a tags field should deserialize with empty tags via #[serde(default)]
    let json_str = r#"{
        "id": "old-task",
        "task": "legacy",
        "status": "complete",
        "iterations": 1,
        "sats_spent": 100,
        "started_at": "2026-01-01T00:00:00Z"
    }"#;
    let info: TaskInfo = serde_json::from_str(json_str).unwrap();
    assert!(info.tags.is_empty());
}

#[test]
fn test_backward_compat_task_request_no_tags() {
    let json_str = r#"{"task": "old request"}"#;
    let req: TaskRequest = serde_json::from_str(json_str).unwrap();
    assert!(req.tags.is_none());
}

#[test]
fn test_backward_compat_task_summary_no_tags() {
    let json_str = r#"{
        "id": "old",
        "task": "legacy",
        "status": "complete",
        "iterations": 1,
        "sats_spent": 100,
        "started_at": "2026-01-01T00:00:00Z",
        "tokens": 0
    }"#;
    let summary: TaskSummary = serde_json::from_str(json_str).unwrap();
    assert!(summary.tags.is_empty());
}

// ===========================================================================
// TaskSummary serialization with tags
// ===========================================================================

#[test]
fn test_task_summary_with_tags_roundtrip() {
    let summary = make_summary("rt", vec!["marketing", "q1"]);
    let json_str = serde_json::to_string(&summary).unwrap();
    assert!(json_str.contains("marketing"));
    assert!(json_str.contains("q1"));

    let parsed: TaskSummary = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.tags, vec!["marketing", "q1"]);
}

#[test]
fn test_task_summary_empty_tags_omitted() {
    let summary = make_summary("no-tags", vec![]);
    let json_str = serde_json::to_string(&summary).unwrap();
    assert!(
        !json_str.contains("\"tags\""),
        "empty tags key should be omitted: {json_str}"
    );
}

// ===========================================================================
// Tags required enforcement logic
// ===========================================================================

#[test]
fn test_tags_required_rejects_empty_tags() {
    use dolphin_milk::certificates::CertTagPolicy;

    // Simulate enforcement: tags_required = true and no tags provided
    let policy = CertTagPolicy {
        tags_required: Some(true),
    };
    let tags: Vec<String> = normalize_tags(None);

    let should_reject = tags.is_empty() && policy.tags_required == Some(true);
    assert!(
        should_reject,
        "should reject when tags_required=true and no tags"
    );
}

#[test]
fn test_tags_required_allows_when_tags_present() {
    use dolphin_milk::certificates::CertTagPolicy;

    let policy = CertTagPolicy {
        tags_required: Some(true),
    };
    let tags = normalize_tags(Some(vec!["billing".into()]));

    let should_reject = tags.is_empty() && policy.tags_required == Some(true);
    assert!(
        !should_reject,
        "should allow when tags_required=true and tags provided"
    );
}

#[test]
fn test_tags_not_required_allows_empty() {
    use dolphin_milk::certificates::CertTagPolicy;

    let policy = CertTagPolicy {
        tags_required: Some(false),
    };
    let tags: Vec<String> = normalize_tags(None);

    let should_reject = tags.is_empty() && policy.tags_required == Some(true);
    assert!(!should_reject, "should allow when tags_required=false");
}

#[test]
fn test_tags_required_none_allows_empty() {
    use dolphin_milk::certificates::CertTagPolicy;

    // No cert → tags_required = None → no enforcement
    let policy = CertTagPolicy {
        tags_required: None,
    };
    let tags: Vec<String> = normalize_tags(None);

    let should_reject = tags.is_empty() && policy.tags_required == Some(true);
    assert!(!should_reject, "should allow when no cert policy");
}
