//! Tests for model_input_limit() and model_output_limit() in think.rs

use dolphin_milk::think::{model_input_limit, model_output_limit};

// === Input limit tests ===

#[test]
fn test_input_limit_gpt5_mini() {
    assert_eq!(model_input_limit("gpt-5-mini"), 272_000);
}

#[test]
fn test_input_limit_gpt5_nano() {
    assert_eq!(model_input_limit("gpt-5-nano"), 272_000);
}

#[test]
fn test_input_limit_gpt5() {
    assert_eq!(model_input_limit("gpt-5"), 272_000);
}

#[test]
fn test_input_limit_gpt5_2() {
    assert_eq!(model_input_limit("gpt-5.2"), 272_000);
}

#[test]
fn test_input_limit_gpt5_2_pro() {
    assert_eq!(model_input_limit("gpt-5.2-pro"), 272_000);
}

#[test]
fn test_input_limit_o4_mini() {
    assert_eq!(model_input_limit("o4-mini"), 100_000);
}

#[test]
fn test_input_limit_o3() {
    assert_eq!(model_input_limit("o3"), 100_000);
}

#[test]
fn test_input_limit_claude_haiku() {
    assert_eq!(model_input_limit("claude-haiku-4-5"), 136_000);
    // Versioned variant uses the same prefix match
    assert_eq!(model_input_limit("claude-haiku-4-5-20251001"), 136_000);
}

#[test]
fn test_input_limit_claude_sonnet() {
    assert_eq!(model_input_limit("claude-sonnet-4-6"), 936_000);
}

#[test]
fn test_input_limit_claude_opus() {
    assert_eq!(model_input_limit("claude-opus-4-6"), 872_000);
}

#[test]
fn test_input_limit_unknown_model() {
    assert_eq!(model_input_limit("unknown-model-v99"), 128_000);
}

// === Output limit tests ===

#[test]
fn test_output_limit_gpt5_mini() {
    assert_eq!(model_output_limit("gpt-5-mini"), 128_000);
}

#[test]
fn test_output_limit_o4_mini() {
    assert_eq!(model_output_limit("o4-mini"), 100_000);
}

#[test]
fn test_output_limit_claude_haiku() {
    assert_eq!(model_output_limit("claude-haiku-4-5"), 64_000);
}

#[test]
fn test_output_limit_claude_sonnet() {
    assert_eq!(model_output_limit("claude-sonnet-4-6"), 64_000);
}

#[test]
fn test_output_limit_claude_opus() {
    assert_eq!(model_output_limit("claude-opus-4-6"), 128_000);
}

#[test]
fn test_output_limit_unknown() {
    assert_eq!(model_output_limit("unknown"), 16_384);
}

// === Config cap behavior tests ===

#[test]
fn test_config_caps_model_limit() {
    // Config context_window (128K) < model input limit (272K) -> config wins
    let config_window: usize = 128_000;
    let model_limit = model_input_limit("gpt-5-mini");
    let effective = config_window.min(model_limit);
    assert_eq!(effective, 128_000);
}

#[test]
fn test_model_limit_caps_large_config() {
    // Config context_window (999K) > model input limit (272K) -> model wins
    let config_window: usize = 999_000;
    let model_limit = model_input_limit("gpt-5-mini");
    let effective = config_window.min(model_limit);
    assert_eq!(effective, 272_000);
}

#[test]
fn test_output_config_caps_model() {
    // Config max_tokens (16384) < model output limit (128K) -> config wins
    let config_output: usize = 16_384;
    let model_limit = model_output_limit("gpt-5-mini");
    let effective = config_output.min(model_limit);
    assert_eq!(effective, 16_384);
}

#[test]
fn test_output_model_caps_large_config() {
    // Config max_tokens (200K) > model output limit (128K) -> model wins
    let config_output: usize = 200_000;
    let model_limit = model_output_limit("gpt-5-mini");
    let effective = config_output.min(model_limit);
    assert_eq!(effective, 128_000);
}

// === Invariant: input + output <= context_window ===

#[test]
fn test_gpt5_input_plus_output_eq_context() {
    // gpt-5: 400K context = 272K input + 128K output
    assert_eq!(
        model_input_limit("gpt-5") + model_output_limit("gpt-5"),
        400_000
    );
}

#[test]
fn test_o4_input_plus_output_eq_context() {
    // o4-mini: 200K context = 100K input + 100K output
    assert_eq!(
        model_input_limit("o4-mini") + model_output_limit("o4-mini"),
        200_000
    );
}

#[test]
fn test_claude_haiku_input_plus_output_eq_context() {
    // claude-haiku: 200K context = 136K input + 64K output
    assert_eq!(
        model_input_limit("claude-haiku-4-5") + model_output_limit("claude-haiku-4-5"),
        200_000
    );
}

#[test]
fn test_claude_sonnet_input_plus_output_eq_context() {
    // claude-sonnet: 1M context = 936K input + 64K output
    assert_eq!(
        model_input_limit("claude-sonnet-4-6") + model_output_limit("claude-sonnet-4-6"),
        1_000_000
    );
}

#[test]
fn test_claude_opus_input_plus_output_eq_context() {
    // claude-opus: 1M context = 872K input + 128K output
    assert_eq!(
        model_input_limit("claude-opus-4-6") + model_output_limit("claude-opus-4-6"),
        1_000_000
    );
}
