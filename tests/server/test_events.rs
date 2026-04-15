//! Integration tests for the events module and server auth/chat endpoints.

use dolphin_milk::events::{format_sse_event, truncate_for_sse, StepEvent};

// -- StepEvent serialization --

#[test]
fn test_all_event_types_serialize() {
    let events: Vec<StepEvent> = vec![
        StepEvent::ThinkingStarted {
            iteration: 1,
            model: "gpt-5-nano".into(),
        },
        StepEvent::ThinkingComplete {
            iteration: 1,
            text: "Hello".into(),
            sats_paid: 100,
            tokens: 50,
            has_tool_calls: false,
        },
        StepEvent::ToolCallStarted {
            iteration: 1,
            call_id: "call-1".into(),
            name: "execute_bash".into(),
            arguments: r#"{"cmd":"ls"}"#.into(),
        },
        StepEvent::ToolCallComplete {
            iteration: 1,
            call_id: "call-1".into(),
            name: "execute_bash".into(),
            output: "file.txt".into(),
            success: true,
        },
        StepEvent::Response {
            iteration: 1,
            text: "Done".into(),
        },
        StepEvent::Done {
            iterations: 3,
            sats_spent: 1500,
            result: "Task complete".into(),
        },
        StepEvent::Error {
            iteration: 2,
            message: "Budget exceeded".into(),
        },
        StepEvent::BudgetUpdate {
            balance: 50000,
            spent: 1500,
            remaining: 18500,
        },
        StepEvent::SessionStarted {
            session_id: "conv-test".into(),
            task_id: "task-test".into(),
            resumed: false,
        },
    ];

    for event in &events {
        let json = serde_json::to_string(event).unwrap();
        assert!(json.contains("\"type\":"));
        // Every event should contain a "type" field
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("type").is_some());
    }
}

#[test]
fn test_event_type_names_are_snake_case() {
    let events = vec![
        (
            StepEvent::ThinkingStarted {
                iteration: 0,
                model: "m".into(),
            },
            "thinking_started",
        ),
        (
            StepEvent::ThinkingComplete {
                iteration: 0,
                text: "".into(),
                sats_paid: 0,
                tokens: 0,
                has_tool_calls: false,
            },
            "thinking_complete",
        ),
        (
            StepEvent::ToolCallStarted {
                iteration: 0,
                call_id: "".into(),
                name: "".into(),
                arguments: "".into(),
            },
            "tool_call_started",
        ),
        (
            StepEvent::ToolCallComplete {
                iteration: 0,
                call_id: "".into(),
                name: "".into(),
                output: "".into(),
                success: true,
            },
            "tool_call_complete",
        ),
        (
            StepEvent::Response {
                iteration: 0,
                text: "".into(),
            },
            "response",
        ),
        (
            StepEvent::Done {
                iterations: 0,
                sats_spent: 0,
                result: "".into(),
            },
            "done",
        ),
        (
            StepEvent::Error {
                iteration: 0,
                message: "".into(),
            },
            "error",
        ),
        (
            StepEvent::BudgetUpdate {
                balance: 0,
                spent: 0,
                remaining: 0,
            },
            "budget_update",
        ),
        (
            StepEvent::SessionStarted {
                session_id: "conv-1".into(),
                task_id: "t-1".into(),
                resumed: false,
            },
            "session_started",
        ),
    ];

    for (event, expected_type) in events {
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"].as_str().unwrap(), expected_type);
    }
}

#[test]
fn test_session_started_serialization() {
    let event = StepEvent::SessionStarted {
        session_id: "conv-abc123".into(),
        task_id: "task-xyz".into(),
        resumed: true,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "session_started");
    assert_eq!(parsed["session_id"], "conv-abc123");
    assert_eq!(parsed["task_id"], "task-xyz");
    assert_eq!(parsed["resumed"], true);
}

#[test]
fn test_session_started_new_conversation() {
    let event = StepEvent::SessionStarted {
        session_id: "conv-new".into(),
        task_id: "task-new".into(),
        resumed: false,
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["resumed"], false);
}

// -- Truncation --

#[test]
fn test_truncate_preserves_short_text() {
    let short = "Hello, world!";
    assert_eq!(truncate_for_sse(short), short);
}

#[test]
fn test_truncate_cuts_long_text() {
    let long = "x".repeat(5000);
    let result = truncate_for_sse(&long);
    assert!(result.len() < 5000);
    assert!(result.contains("...truncated"));
    assert!(result.contains("5000 chars"));
}

#[test]
fn test_truncate_boundary_exact_2000() {
    let exact = "a".repeat(2000);
    let result = truncate_for_sse(&exact);
    assert_eq!(result, exact); // Exactly at limit, no truncation
}

#[test]
fn test_truncate_boundary_2001() {
    let over = "a".repeat(2001);
    let result = truncate_for_sse(&over);
    assert!(result.contains("...truncated"));
}

// -- SSE formatting --

#[test]
fn test_format_sse_produces_valid_json() {
    let event = StepEvent::ThinkingStarted {
        iteration: 5,
        model: "gpt-5-mini".into(),
    };
    let json_str = format_sse_event(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "thinking_started");
    assert_eq!(parsed["iteration"], 5);
    assert_eq!(parsed["model"], "gpt-5-mini");
}

#[test]
fn test_format_sse_done_includes_result() {
    let event = StepEvent::Done {
        iterations: 10,
        sats_spent: 5000,
        result: "The answer is 42.".into(),
    };
    let json_str = format_sse_event(&event).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "done");
    assert_eq!(parsed["iterations"], 10);
    assert_eq!(parsed["sats_spent"], 5000);
    assert_eq!(parsed["result"], "The answer is 42.");
}

// -- StepEvent Clone + Debug --

#[test]
fn test_step_event_clone() {
    let event = StepEvent::ThinkingStarted {
        iteration: 1,
        model: "gpt-5-mini".into(),
    };
    let cloned = event.clone();
    let json1 = serde_json::to_string(&event).unwrap();
    let json2 = serde_json::to_string(&cloned).unwrap();
    assert_eq!(json1, json2);
}

#[test]
fn test_step_event_debug() {
    let event = StepEvent::Error {
        iteration: 3,
        message: "test error".into(),
    };
    let debug = format!("{event:?}");
    assert!(debug.contains("Error"));
    assert!(debug.contains("test error"));
}

// -- Server chat types --

use dolphin_milk::auth::{HandshakeRequest, HandshakeResponse};
use dolphin_milk::server::ChatMessage;
use dolphin_milk::server::ChatRequest;

#[test]
fn test_handshake_request_serde() {
    let json = r#"{"version":"0.1","messageType":"initialRequest","identityKey":"02abc","initialNonce":"dGVzdA=="}"#;
    let req: HandshakeRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.version, "0.1");
    assert_eq!(req.message_type, "initialRequest");
    assert_eq!(req.identity_key, "02abc");
}

#[test]
fn test_handshake_response_serde() {
    let resp = HandshakeResponse {
        version: "0.1".into(),
        message_type: "initialResponse".into(),
        identity_key: "02server".into(),
        initial_nonce: "c2VydmVy".into(),
        your_nonce: "Y2xpZW50".into(),
        signature: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("messageType"));
    assert!(json.contains("initialResponse"));
    assert!(json.contains("yourNonce"));
}

#[test]
fn test_chat_request_default_iterations() {
    let req: ChatRequest = serde_json::from_str(r#"{"message":"hello"}"#).unwrap();
    assert_eq!(req.message, "hello");
    assert_eq!(req.max_iterations, 50);
}

#[test]
fn test_chat_request_custom_iterations() {
    let req: ChatRequest =
        serde_json::from_str(r#"{"message":"hello","max_iterations":5}"#).unwrap();
    assert_eq!(req.max_iterations, 5);
}

#[test]
fn test_chat_request_with_session_id() {
    let req: ChatRequest =
        serde_json::from_str(r#"{"message":"hello","session_id":"conv-abc"}"#).unwrap();
    assert_eq!(req.session_id, Some("conv-abc".to_string()));
}

#[test]
fn test_chat_request_without_session_id() {
    let req: ChatRequest = serde_json::from_str(r#"{"message":"hello"}"#).unwrap();
    assert_eq!(req.session_id, None);
}

#[test]
fn test_chat_message_serde() {
    let msg = ChatMessage {
        role: "assistant".into(),
        content: "Hello!".into(),
        tool_name: None,
        call_id: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    // None fields should not appear
    assert!(!json.contains("tool_name"));
    assert!(!json.contains("call_id"));
}

#[test]
fn test_chat_message_with_tool() {
    let msg = ChatMessage {
        role: "tool".into(),
        content: "file.txt\nfile2.txt".into(),
        tool_name: Some("execute_bash".into()),
        call_id: Some("call-1".into()),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("execute_bash"));
    assert!(json.contains("call-1"));
}

// -- Server endpoint tests (via oneshot) --

use axum::body::Body;
use axum::http::{self, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use dolphin_milk::config::DmConfig;
use dolphin_milk::server;

#[path = "../common/mod.rs"]
mod common;

#[tokio::test]
async fn test_brc31_handshake_endpoint() {
    let app = common::test_router().await;
    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/.well-known/auth")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "version": "0.1",
                "messageType": "initialRequest",
                "identityKey": "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce",
                "initialNonce": "dGVzdG5vbmNl"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let handshake: HandshakeResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(handshake.version, "0.1");
    assert_eq!(handshake.message_type, "initialResponse");
    assert_eq!(handshake.your_nonce, "dGVzdG5vbmNl");
    assert!(!handshake.initial_nonce.is_empty());
}

#[tokio::test]
async fn test_brc31_handshake_rejects_wrong_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let mut config = DmConfig::default();
    config.parent.identity_key =
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".into();
    let state = server::create_app_state(config, path).await;
    let app = server::build_router_with_state(state);

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/.well-known/auth")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "version": "0.1",
                "messageType": "initialRequest",
                "identityKey": "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
                "initialNonce": "dGVzdA=="
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_chat_without_wallet_no_auth() {
    // When wallet is unreachable, auth is off (no identity key fetched)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let mut config = DmConfig::default();
    config.wallet.url = "http://127.0.0.1:19999".into(); // unreachable
    let app = server::build_router(config, path).await;

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/chat")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"hello","max_iterations":1}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Should return 200 (SSE stream starts), not 401
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_budget_without_wallet_no_auth() {
    let app = common::test_router().await;
    let req = Request::builder()
        .uri("/budget")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
#[ignore] // Requires real wallet for identity key fetch — tested in E2E
async fn test_budget_with_parent_key_requires_auth() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let mut config = DmConfig::default();
    config.parent.identity_key =
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".into();
    let app = server::build_router(config, path).await;

    let req = Request::builder()
        .uri("/budget")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore] // Requires real wallet for identity key fetch — tested in E2E
async fn test_chat_with_parent_key_requires_auth() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    std::mem::forget(dir);

    let mut config = DmConfig::default();
    config.parent.identity_key =
        "034aa44668fbc73ca5d490f0fa54b98b398b790856d8c55d540759ccefa5e6d0ce".into();
    let app = server::build_router(config, path).await;

    let req = Request::builder()
        .method(http::Method::POST)
        .uri("/chat")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"message":"hello"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
